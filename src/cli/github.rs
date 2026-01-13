use crate::{
    errors::{SkillsError, SkillsResult},
    models::{GitHubUrl, GitHubUrlSpec},
};
use flate2::read::GzDecoder;
use std::{env, fs, path::Path};
use tar::Archive;
use ureq::config::Config;

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

pub(super) fn build_agent() -> SkillsResult<ureq::Agent> {
    if let Some(proxy_url) = proxy_from_env() {
        let proxy =
            ureq::Proxy::new(&proxy_url).map_err(|e| SkillsError::NetworkError(e.to_string()))?;
        let config = Config::builder().proxy(Some(proxy)).build();
        Ok(ureq::Agent::new_with_config(config))
    } else {
        Ok(ureq::Agent::new_with_defaults())
    }
}

fn download_and_extract(github_url: &GitHubUrl, dest_dir: &Path) -> SkillsResult<()> {
    let agent = build_agent()?;
    let url = github_url.tarball_url();
    let response = match agent
        .get(&url)
        .header("User-Agent", "skills-man")
        .header("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(status)) => {
            return Err(match status {
                404 => SkillsError::NotFound { url },
                403 => SkillsError::Forbidden,
                429 => SkillsError::RateLimited,
                _ => SkillsError::HttpError {
                    status,
                    message: url,
                },
            });
        }
        Err(e) => return SkillsError::NetworkError(e.to_string()).into(),
    };

    let decoder = GzDecoder::new(response.into_body().into_reader());
    let mut archive = Archive::new(decoder);

    let mut found_any = false;
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
            let expected_prefix = format!("{}/{}/", top_dir, github_url.path);
            if entry_str.starts_with(&expected_prefix) {
                let relative = &entry_str[expected_prefix.len()..];
                if !relative.is_empty() {
                    found_any = true;
                    let dest_path = dest_dir.join(relative);
                    if let Some(parent) = dest_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    entry.unpack(&dest_path)?;
                }
            }
        }
    }

    if !found_any {
        return SkillsError::PathNotFound(github_url.path.clone()).into();
    }

    Ok(())
}

/// Resolves a ref to its commit SHA using the GitHub commits API.
/// Returns Ok(Some(sha)) if the ref exists, Ok(None) if not found.
pub(super) fn resolve_ref_to_sha(
    agent: &ureq::Agent,
    github_url: &GitHubUrl,
) -> SkillsResult<Option<String>> {
    let url = github_url.commits_url();
    match agent
        .get(&url)
        .header("User-Agent", "skills-man")
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
            403 => Err(SkillsError::Forbidden),
            429 => Err(SkillsError::RateLimited),
            _ => Err(SkillsError::HttpError {
                status,
                message: url,
            }),
        },
        Err(e) => Err(SkillsError::NetworkError(e.to_string())),
    }
}

pub(super) fn download_with_candidates(
    spec: &GitHubUrlSpec,
    dest_dir: &Path,
) -> SkillsResult<GitHubUrl> {
    let agent = build_agent()?;
    let mut last_retryable: Option<SkillsError> = None;

    for candidate in spec.candidates() {
        let sha = match resolve_ref_to_sha(&agent, &candidate)? {
            None => {
                last_retryable = Some(SkillsError::NotFound {
                    url: candidate.commits_url(),
                });
                continue;
            }
            Some(sha) => sha,
        };

        let resolved = GitHubUrl {
            r#ref: sha,
            ..candidate
        };

        match download_and_extract(&resolved, dest_dir) {
            Ok(_) => return Ok(resolved),
            Err(err) => match err {
                SkillsError::NotFound { .. } | SkillsError::PathNotFound(_) => {
                    last_retryable = Some(err);
                }
                _ => return Err(err),
            },
        }
    }

    Err(last_retryable.unwrap_or_else(|| {
        SkillsError::InvalidUrl("No valid ref/path candidates found".to_string())
    }))
}
