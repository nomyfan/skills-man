use crate::{
    errors::SkillsResult,
    models::SkillsConfig,
    providers::{ExtractTarget, ProviderRegistry},
    utils::{calculate_checksum, ensure_skill_manifest},
};
use std::{fs, path::Path};

use super::prompt::confirm_action;

pub fn sync_skills(base_dir: &Path, registry: &ProviderRegistry) -> SkillsResult<()> {
    let config_path = base_dir.join("skills.toml");
    let mut config = SkillsConfig::from_file(&config_path)?;

    let skills_dir = base_dir.join("skills");

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

                    confirm_action("Overwrite local changes?")
                }
                Err(e) => {
                    eprintln!("[{}] Error calculating checksum: {}", name, e);
                    true
                }
            }
        };

        if needs_download {
            let Ok(provider) = registry.get(&entry.source_url) else {
                eprintln!("[{}] No provider available for: {}", name, entry.source_url);
                continue;
            };

            let temp_dir = skills_dir.join(format!(".{}.tmp", name));
            if temp_dir.exists() {
                fs::remove_dir_all(&temp_dir).ok();
            }
            if let Err(e) = fs::create_dir_all(&temp_dir) {
                eprintln!("[{}] Failed to create temp directory: {}", name, e);
                continue;
            }

            let archive_url = provider.archive_url_for_entry(entry);
            let target = ExtractTarget {
                path: entry.path.clone(),
                dest_dir: temp_dir.clone(),
            };

            match provider.fetch_and_extract(&archive_url, &[target]) {
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
