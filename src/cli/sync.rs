use crate::{
    errors::SkillsResult,
    models::{GitHubUrl, SkillsConfig},
    utils::{calculate_checksum, ensure_skill_manifest},
};
use std::{fs, io, io::Write as IoWrite, path::Path};

use super::github::{build_agent, download_and_extract};

pub fn sync_skills(base_dir: &Path) -> SkillsResult<()> {
    let config_path = base_dir.join("skills.toml");
    let mut config = SkillsConfig::from_file(&config_path)?;
    let agent = build_agent()?;

    let skills_dir = base_dir.join("skills");

    if skills_dir.exists() {
        for entry in fs::read_dir(&skills_dir)? {
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
            println!("[{}] Downloading...", name);
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
            let github_url = GitHubUrl {
                slug: entry.slug.clone(),
                r#ref: entry.sha.clone(),
                path: entry.path.clone(),
            };

            let temp_dir = skills_dir.join(format!(".{}.tmp", name));
            if temp_dir.exists() {
                fs::remove_dir_all(&temp_dir).ok();
            }
            if let Err(e) = fs::create_dir_all(&temp_dir) {
                eprintln!("[{}] Failed to create temp directory: {}", name, e);
                continue;
            }

            match download_and_extract(&agent, &github_url, &temp_dir) {
                Ok(_) => {
                    if let Err(e) = ensure_skill_manifest(&temp_dir) {
                        eprintln!("[{}] Downloaded but invalid skill: {}", name, e);
                        fs::remove_dir_all(&temp_dir).ok();
                        continue;
                    }

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

    config.save(&config_path)?;

    Ok(())
}
