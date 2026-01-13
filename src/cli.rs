use crate::{
    errors::{SkillsError, SkillsResult},
    models::{GitHubUrl, GitHubUrlSpec, SkillEntry, SkillsConfig},
    utils::calculate_checksum,
};
use flate2::read::GzDecoder;
use std::{collections::HashSet, env, fs, io, io::Write as IoWrite, path::Path};
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

fn build_agent() -> SkillsResult<ureq::Agent> {
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
fn resolve_ref_to_sha(agent: &ureq::Agent, github_url: &GitHubUrl) -> SkillsResult<Option<String>> {
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

fn download_with_candidates(spec: &GitHubUrlSpec, dest_dir: &Path) -> SkillsResult<GitHubUrl> {
    let agent = build_agent()?;
    let mut last_retryable: Option<SkillsError> = None;

    for candidate in spec.candidates() {
        // First resolve the ref to SHA using the commits API
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

pub fn install_skill(url: &str) -> SkillsResult<()> {
    let spec = GitHubUrlSpec::parse(url)?;

    let skill_name = spec.directory_name();
    let skills_dir = Path::new("./skills");
    let skill_dir = skills_dir.join(skill_name);
    let config_path = Path::new("skills.toml");

    let mut config = SkillsConfig::from_file(config_path)?;

    if let Some(existing) = config.skills.get(skill_name)
        && skill_dir.exists()
        && let Ok(checksum) = calculate_checksum(&skill_dir)
        && checksum == existing.checksum
    {
        let match_upstream_sha = |agent| {
            for candidate in spec.candidates() {
                if let Ok(Some(current_sha)) = resolve_ref_to_sha(&agent, &candidate) {
                    return Some(current_sha == existing.sha);
                }
            }
            // Unable to resolve SHA (network error or 404)
            None
        };

        match match_upstream_sha(build_agent()?) {
            Some(true) => {
                // Upstream SHA matches - truly up to date
                println!(
                    "Skill '{}' is already installed and up to date.",
                    skill_name
                );
                return Ok(());
            }
            Some(false) => {
                // Upstream has moved to new SHA - proceed with installation
                println!(
                    "Upstream ref has moved to new commit, updating skill '{}'...",
                    skill_name
                );
                // Fall through to installation
            }
            None => {
                // SHA resolution failed (network error) - assume up to date
                println!(
                    "Skill '{}' is already installed (checksum matches, unable to verify upstream).",
                    skill_name
                );
                return Ok(());
            }
        }
    }

    let temp_dir = skills_dir.join(format!(".{}.tmp", skill_name));
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir)?;
    }
    fs::create_dir_all(&temp_dir)?;

    println!("Downloading skill '{}'...", skill_name);
    match download_with_candidates(&spec, &temp_dir) {
        Ok(github_url) => {
            if skill_dir.exists() {
                fs::remove_dir_all(&skill_dir)?;
            }

            fs::rename(&temp_dir, &skill_dir)?;

            let checksum = calculate_checksum(&skill_dir)?;

            let entry = SkillEntry {
                source_url: url.to_string(),
                slug: github_url.slug,
                sha: github_url.r#ref,
                path: github_url.path,
                checksum,
            };

            config.skills.insert(skill_name.to_string(), entry);
            config.save(config_path)?;

            println!("Successfully installed skill '{}'.", skill_name);
            Ok(())
        }
        Err(e) => {
            fs::remove_dir_all(&temp_dir).ok();
            Err(e)
        }
    }
}

pub fn sync_skills() -> SkillsResult<()> {
    let config_path = Path::new("skills.toml");
    let mut config = SkillsConfig::from_file(config_path)?;

    let skills_dir = Path::new("./skills");
    let configured: HashSet<String> = config.skills.keys().cloned().collect();

    if skills_dir.exists() {
        for entry in fs::read_dir(skills_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };

            if name.starts_with('.') {
                continue;
            }

            if !configured.contains(name) {
                fs::remove_dir_all(&path)?;
            }
        }
    }

    if config.skills.is_empty() {
        println!("No skills configured in skills.toml");
        return Ok(());
    }

    let skill_names: Vec<String> = config.skills.keys().cloned().collect();

    for name in skill_names {
        let entry = config.skills.get(&name).unwrap();
        let skill_dir = skills_dir.join(&name);

        let needs_download = if !skill_dir.exists() {
            println!("[{}] Needs download", name);
            true
        } else {
            match calculate_checksum(&skill_dir) {
                Ok(checksum) if checksum == entry.checksum => {
                    println!("[{}] Up to date", name);
                    false
                }
                Ok(_) => {
                    println!(
                        "[{}] Checksum mismatch - local modifications detected",
                        name
                    );

                    print!("Overwrite local changes? (y/N): ");
                    io::stdout().flush().ok();

                    let mut input = String::new();
                    io::stdin().read_line(&mut input).ok();
                    let answer = input.trim().to_lowercase();

                    answer == "y" || answer == "yes"
                }
                Err(e) => {
                    eprintln!("[{}] Error calculating checksum: {}", name, e);
                    true
                }
            }
        };

        if needs_download {
            let spec = match GitHubUrlSpec::parse(&entry.source_url) {
                Ok(url) => url,
                Err(e) => {
                    eprintln!("[{}] Invalid URL: {}", name, e);
                    continue;
                }
            };

            let temp_dir = skills_dir.join(format!(".{}.tmp", name));
            if temp_dir.exists() {
                fs::remove_dir_all(&temp_dir).ok();
            }
            if let Err(e) = fs::create_dir_all(&temp_dir) {
                eprintln!("[{}] Failed to create temp directory: {}", name, e);
                continue;
            }

            match download_with_candidates(&spec, &temp_dir) {
                Ok(_) => {
                    if skill_dir.exists() {
                        fs::remove_dir_all(&skill_dir).ok();
                    }
                    match fs::rename(&temp_dir, &skill_dir) {
                        Ok(_) => match calculate_checksum(&skill_dir) {
                            Ok(checksum) => {
                                if let Some(entry) = config.skills.get_mut(&name) {
                                    entry.checksum = checksum;
                                }
                                println!("[{}] Downloaded successfully", name);
                            }
                            Err(e) => {
                                eprintln!(
                                    "[{}] Downloaded but failed to calculate checksum: {}",
                                    name, e
                                );
                            }
                        },
                        Err(e) => {
                            eprintln!("[{}] Failed to move to final location: {}", name, e);
                            fs::remove_dir_all(&temp_dir).ok();
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[{}] Download failed: {}", name, e);
                    fs::remove_dir_all(&temp_dir).ok();
                }
            }
        }
    }

    config.save(config_path)?;

    Ok(())
}

pub fn uninstall_skill(name: &str) -> SkillsResult<()> {
    let config_path = Path::new("skills.toml");
    let mut config = SkillsConfig::from_file(config_path)?;

    let skills_dir = Path::new("./skills");
    let skill_dir = skills_dir.join(name);

    let mut removed_any = false;
    if skill_dir.exists() {
        fs::remove_dir_all(&skill_dir)?;
        removed_any = true;
    }

    if config.skills.remove(name).is_some() {
        removed_any = true;
        config.save(config_path)?;
    }

    if removed_any {
        println!("Successfully uninstalled skill '{}'.", name);
    } else {
        println!("Skill '{}' is not installed.", name);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_calculate_checksum() {
        let temp_dir = std::env::temp_dir().join("skills_test_checksum");
        fs::create_dir_all(&temp_dir).unwrap();

        fs::write(temp_dir.join("file1.txt"), b"content1").unwrap();
        fs::write(temp_dir.join("file2.txt"), b"content2").unwrap();

        let checksum1 = calculate_checksum(&temp_dir).unwrap();

        let checksum2 = calculate_checksum(&temp_dir).unwrap();
        assert_eq!(checksum1, checksum2);

        assert!(checksum1.starts_with("sha256:"));

        fs::write(temp_dir.join("file1.txt"), b"modified").unwrap();
        let checksum3 = calculate_checksum(&temp_dir).unwrap();
        assert_ne!(checksum1, checksum3);

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_load_config_empty_or_missing() {
        let config = SkillsConfig::from_file(Path::new("/nonexistent/skills.toml")).unwrap();
        assert!(config.skills.is_empty());

        let temp_dir = std::env::temp_dir().join("skills_test_empty_config");
        fs::create_dir_all(&temp_dir).unwrap();
        let config_path = temp_dir.join("skills.toml");

        fs::write(&config_path, "").unwrap();

        let config = SkillsConfig::from_file(&config_path).unwrap();
        assert!(config.skills.is_empty());

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_load_config_malformed() {
        let temp_dir = std::env::temp_dir().join("skills_test_malformed");
        fs::create_dir_all(&temp_dir).unwrap();
        let config_path = temp_dir.join("skills.toml");

        fs::write(&config_path, "invalid toml content [[[").unwrap();

        let result = SkillsConfig::from_file(&config_path);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SkillsError::ConfigParseError(_)
        ));

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_save_and_load_config() {
        let temp_dir = std::env::temp_dir().join("skills_test_config");
        fs::create_dir_all(&temp_dir).unwrap();
        let config_path = temp_dir.join("skills.toml");

        let mut config = SkillsConfig::default();
        config.skills.insert(
            "test-skill".to_string(),
            SkillEntry {
                source_url: "https://github.com/owner/repo/tree/main/path".to_string(),
                slug: "owner/repo".to_string(),
                sha: "main".to_string(),
                path: "path".to_string(),
                checksum: "sha256:abc123".to_string(),
            },
        );

        config.save(&config_path).unwrap();

        let loaded_config = SkillsConfig::from_file(&config_path).unwrap();
        assert_eq!(loaded_config.skills.len(), 1);
        assert!(loaded_config.skills.contains_key("test-skill"));

        let entry = &loaded_config.skills["test-skill"];
        assert_eq!(
            entry.source_url,
            "https://github.com/owner/repo/tree/main/path"
        );
        assert_eq!(entry.checksum, "sha256:abc123");

        fs::remove_dir_all(&temp_dir).unwrap();
    }
}
