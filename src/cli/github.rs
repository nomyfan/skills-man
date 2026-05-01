use crate::{
    errors::{SkillsError, SkillsResult},
    models::{GitHubUrl, GitHubUrlSpec},
};
use flate2::read::GzDecoder;
use serde::Deserialize;
use std::{env, fs, path::PathBuf};
use tar::Archive;
use ureq::typestate::WithoutBody;
use ureq::{RequestBuilder, config::Config};

const GITHUB_API_VERSION: &str = "2026-03-10";

fn proxy_from_env() -> Option<String> {
    for key in [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ] {
        if let Ok(value) = env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn github_token_from_env() -> Option<String> {
    for key in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if let Ok(value) = env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn config_github_request(request: RequestBuilder<WithoutBody>) -> RequestBuilder<WithoutBody> {
    let mut request = request
        .header("User-Agent", "skills-man")
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION);
    if let Some(token) = github_token_from_env() {
        request = request.header("Authorization", &format!("Bearer {token}"));
    }
    request
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedSkill {
    pub name: String,
    pub source_url: String,
    pub slug: String,
    pub sha: String,
    pub path: String,
}

#[derive(Debug)]
pub(super) struct InstallPlan {
    pub tarball_url: String,
    pub is_batch: bool,
    pub skills: Vec<ResolvedSkill>,
}

pub(super) struct ExtractTarget {
    pub path: String,
    pub dest_dir: PathBuf,
}

pub(super) fn create_agent() -> SkillsResult<ureq::Agent> {
    if let Some(proxy_url) = proxy_from_env() {
        let proxy =
            ureq::Proxy::new(&proxy_url).map_err(|e| SkillsError::NetworkError(e.to_string()))?;
        let config = Config::builder().proxy(Some(proxy)).build();
        Ok(ureq::Agent::new_with_config(config))
    } else {
        Ok(ureq::Agent::new_with_defaults())
    }
}

pub(super) fn download_and_extract(
    agent: &ureq::Agent,
    url: &str,
    targets: &[ExtractTarget],
) -> SkillsResult<()> {
    let response = match config_github_request(agent.get(url))
        .header("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(status)) => {
            return Err(match status {
                404 => SkillsError::NotFound {
                    url: url.to_string(),
                },
                403 => SkillsError::Forbidden {
                    url: url.to_string(),
                },
                429 => SkillsError::RateLimited,
                _ => SkillsError::HttpError {
                    status,
                    message: url.to_string(),
                },
            });
        }
        Err(e) => return SkillsError::NetworkError(e.to_string()).into(),
    };

    let decoder = GzDecoder::new(response.into_body().into_reader());
    let mut archive = Archive::new(decoder);

    let mut found = vec![false; targets.len()];
    let mut top_level_dir: Option<String> = None;

    for entry in archive
        .entries()
        .map_err(|e| SkillsError::InvalidArchive(e.to_string()))?
    {
        let mut entry = entry.map_err(|e| SkillsError::InvalidArchive(e.to_string()))?;
        let entry_path = entry
            .path()
            .map_err(|e| SkillsError::InvalidArchive(e.to_string()))?;
        let entry_str = entry_path.to_string_lossy();

        if top_level_dir.is_none()
            && let Some(slash_pos) = entry_str.find('/')
        {
            top_level_dir = Some(entry_str[..slash_pos].to_string());
        }

        if let Some(ref top_dir) = top_level_dir {
            for (idx, target) in targets.iter().enumerate() {
                let expected_prefix = format!("{}/{}/", top_dir, target.path);
                if entry_str.starts_with(&expected_prefix) {
                    let relative = &entry_str[expected_prefix.len()..];
                    if relative.is_empty() {
                        break;
                    }
                    found[idx] = true;
                    let dest_path = target.dest_dir.join(relative);
                    if let Some(parent) = dest_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    entry.unpack(&dest_path)?;
                    break;
                }
            }
        }
    }

    let missing_paths: Vec<_> = targets
        .iter()
        .zip(found)
        .filter_map(|(target, found)| (!found).then_some(target.path.clone()))
        .collect();

    if !missing_paths.is_empty() {
        return SkillsError::PathNotFound(missing_paths).into();
    }

    Ok(())
}

pub(super) fn resolve_commit_sha(
    agent: &ureq::Agent,
    github_url: &GitHubUrl,
) -> SkillsResult<Option<String>> {
    let url = github_url.commits_url();
    match config_github_request(agent.get(&url))
        .header("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(response) => {
            let json: serde_json::Value = response
                .into_body()
                .read_json()
                .map_err(|e| SkillsError::NetworkError(e.to_string()))?;
            let sha = json
                .get(0)
                .and_then(|x| x.get("sha"))
                .and_then(|x| x.as_str())
                .ok_or_else(|| SkillsError::NetworkError("Missing sha in response".to_string()))?
                .to_string();
            Ok(Some(sha))
        }
        Err(ureq::Error::StatusCode(status)) => match status {
            404 | 422 => Ok(None),
            403 => Err(SkillsError::Forbidden { url }),
            429 => Err(SkillsError::RateLimited),
            _ => Err(SkillsError::HttpError {
                status,
                message: url,
            }),
        },
        Err(e) => Err(SkillsError::NetworkError(e.to_string())),
    }
}

pub(crate) fn resolve(
    agent: &ureq::Agent,
    spec: &GitHubUrlSpec,
) -> SkillsResult<Option<GitHubUrl>> {
    for candidate in spec.candidates() {
        let sha = resolve_commit_sha(agent, &candidate)?;
        if let Some(sha) = sha {
            return Ok(Some(candidate.with_sha(sha)));
        }
    }
    Ok(None)
}

#[derive(Debug)]
pub enum SkillDetectionResult {
    Single,             // Path is a single skill
    Batch(Vec<String>), // Path contains multiple sub-skills (directory names)
}

#[derive(Debug, Deserialize)]
struct ContentItem {
    name: String,
    #[serde(rename = "type")]
    item_type: String, // "file" or "dir"
}

/// Lists directory contents using the GitHub Contents API
fn list_directory_contents(
    agent: &ureq::Agent,
    github_url: &GitHubUrl,
) -> SkillsResult<Vec<ContentItem>> {
    let url = github_url.contents_url();

    match config_github_request(agent.get(&url))
        .header("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(response) => {
            let items: Vec<ContentItem> = response
                .into_body()
                .read_json()
                .map_err(|e| SkillsError::NetworkError(e.to_string()))?;
            Ok(items)
        }
        Err(ureq::Error::StatusCode(status)) => match status {
            404 => Err(SkillsError::PathNotFound(vec![github_url.path.clone()])),
            403 => Err(SkillsError::Forbidden { url }),
            429 => Err(SkillsError::RateLimited),
            _ => Err(SkillsError::HttpError {
                status,
                message: url,
            }),
        },
        Err(e) => Err(SkillsError::NetworkError(e.to_string())),
    }
}

/// Detects whether the GitHub path points to a single skill or a batch of skills
pub(super) fn detect_skill_type(
    agent: &ureq::Agent,
    github_url: &GitHubUrl,
) -> SkillsResult<SkillDetectionResult> {
    let contents = list_directory_contents(agent, github_url)?;

    // Check if SKILL.md exists in the current directory (case-insensitive)
    let has_skill_manifest = contents
        .iter()
        .any(|item| item.item_type == "file" && (item.name.eq_ignore_ascii_case("SKILL.md")));

    if has_skill_manifest {
        return Ok(SkillDetectionResult::Single);
    }

    // No SKILL.md found, check subdirectories for skills
    let subdirs: Vec<&ContentItem> = contents
        .iter()
        .filter(|item| item.item_type == "dir")
        .collect();

    let mut skill_dirs = Vec::new();

    for subdir in subdirs {
        let child_url = GitHubUrl {
            slug: github_url.slug.clone(),
            r#ref: github_url.r#ref.clone(),
            sha: github_url.sha.clone(),
            path: format!("{}/{}", github_url.path, subdir.name),
        };

        let child_contents = list_directory_contents(agent, &child_url)?;
        let has_skill = child_contents
            .iter()
            .any(|item| item.item_type == "file" && (item.name.eq_ignore_ascii_case("SKILL.md")));

        if has_skill {
            skill_dirs.push(subdir.name.clone());
        }
    }

    if skill_dirs.is_empty() {
        return Err(SkillsError::NoSkillsFound(github_url.path.clone()));
    }

    Ok(SkillDetectionResult::Batch(skill_dirs))
}

pub(super) fn resolve_install_plan(
    agent: &ureq::Agent,
    source_url: &str,
    spec: &GitHubUrlSpec,
) -> SkillsResult<Option<InstallPlan>> {
    let Some(resolved) = resolve(agent, spec)? else {
        return Ok(None);
    };

    let plan = match detect_skill_type(agent, &resolved)? {
        SkillDetectionResult::Single => InstallPlan {
            tarball_url: resolved.tarball_url(),
            is_batch: false,
            skills: vec![ResolvedSkill {
                name: spec.directory_name().to_string(),
                source_url: source_url.to_string(),
                slug: resolved.slug,
                sha: resolved.sha,
                path: resolved.path,
            }],
        },
        SkillDetectionResult::Batch(subdirs) => {
            let mut skills = Vec::new();
            for subdir in subdirs {
                let child_source_url = format!("{}/{}", source_url, subdir);
                let child_candidate = resolved.child(&subdir);
                let Some(child_sha) = resolve_commit_sha(agent, &child_candidate)? else {
                    return Err(SkillsError::PathNotFound(vec![child_source_url]));
                };
                let child_resolved = child_candidate.with_sha(child_sha);

                skills.push(ResolvedSkill {
                    name: subdir,
                    source_url: child_source_url,
                    slug: child_resolved.slug,
                    sha: child_resolved.sha,
                    path: child_resolved.path,
                });
            }

            InstallPlan {
                tarball_url: resolved.tarball_url(),
                is_batch: true,
                skills,
            }
        }
    };

    Ok(Some(plan))
}
