use crate::{
    errors::{SkillsError, SkillsResult},
    models::GitHubUrl,
};
use flate2::read::GzDecoder;
use serde::Deserialize;
use std::{env, fs, path::Path};
use tar::Archive;
use ureq::config::Config;

#[derive(Debug, Deserialize)]
pub(super) struct ContentsEntry {
    pub name: String,
    pub sha: String,
    #[serde(rename = "type")]
    pub r#type: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ContentsResponse {
    pub sha: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub entries: Option<Vec<ContentsEntry>>,
}

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

pub(super) fn download_and_extract(
    agent: &ureq::Agent,
    github_url: &GitHubUrl,
    dest_dir: &Path,
) -> SkillsResult<()> {
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
                403 => SkillsError::Forbidden { url },
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

/// Fetches directory SHA and entries from GitHub Contents API.
pub(super) fn fetch_contents_sha(
    agent: &ureq::Agent,
    github_url: &GitHubUrl,
) -> SkillsResult<ContentsResponse> {
    let url = format!(
        "https://api.github.com/repos/{}/contents/{}?ref={}",
        github_url.slug,
        github_url.path,
        urlencoding::encode(&github_url.r#ref)
    );
    match agent
        .get(&url)
        .header("User-Agent", "skills-man")
        .header("Accept", "application/vnd.github.object")
        .call()
    {
        Ok(response) => response
            .into_body()
            .read_json::<ContentsResponse>()
            .map_err(|e| SkillsError::InvalidResponse(e.to_string())),
        Err(ureq::Error::StatusCode(status)) => Err(match status {
            404 => SkillsError::NotFound { url },
            403 => SkillsError::Forbidden { url },
            429 => SkillsError::RateLimited,
            _ => SkillsError::HttpError {
                status,
                message: url,
            },
        }),
        Err(e) => Err(SkillsError::NetworkError(e.to_string())),
    }
}
