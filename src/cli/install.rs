use crate::{
    errors::SkillsResult,
    models::{GitHubUrlSpec, SkillEntry, SkillsConfig},
    utils::{calculate_checksum, ensure_skill_manifest},
};
use std::{fs, path::Path};

use super::github::{build_agent, download_with_candidates, resolve_ref_to_sha};

pub fn install_skill(url: &str, base_dir: &Path) -> SkillsResult<()> {
    let spec = GitHubUrlSpec::parse(url)?;

    let skill_name = spec.directory_name();
    let skills_dir = base_dir.join("skills");
    let skill_dir = skills_dir.join(skill_name);
    let config_path = base_dir.join("skills.toml");

    let mut config = SkillsConfig::from_file(&config_path)?;

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
            None
        };

        match match_upstream_sha(build_agent()?) {
            Some(true) => {
                println!(
                    "Skill '{}' is already installed and up to date.",
                    skill_name
                );
                return Ok(());
            }
            Some(false) => {
                println!(
                    "Upstream ref has moved to new commit, updating skill '{}'...",
                    skill_name
                );
            }
            None => {
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
            if let Err(e) = ensure_skill_manifest(&temp_dir) {
                fs::remove_dir_all(&temp_dir).ok();
                return Err(e);
            }

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
            config.save(&config_path)?;

            println!("Successfully installed skill '{}'.", skill_name);
            Ok(())
        }
        Err(e) => {
            fs::remove_dir_all(&temp_dir).ok();
            Err(e)
        }
    }
}
