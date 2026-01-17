use crate::{
    cli::github::{download_and_extract, resolve},
    errors::{SkillsError, SkillsResult},
    models::{GitHubUrlSpec, SkillEntry, SkillsConfig},
    utils::{calculate_checksum, ensure_skill_manifest},
};
use std::{
    fs, io,
    io::{IsTerminal, Write as IoWrite},
    path::Path,
};

/// Prompt user for confirmation
/// Returns true if yes flag is set, or if user confirms in interactive mode
/// Returns false if stdin is not a TTY (non-interactive context)
fn confirm_action(prompt: &str, yes: bool) -> bool {
    if yes {
        return true;
    }

    if !io::stdin().is_terminal() {
        eprintln!("Non-interactive mode detected. Use --yes flag to auto-confirm.");
        return false;
    }

    print!("{} (y/N): ", prompt);
    io::stdout().flush().ok();

    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();
    let answer = input.trim().to_lowercase();

    answer == "y" || answer == "yes"
}

pub fn install_skill(url: &str, base_dir: &Path, yes: bool) -> SkillsResult<()> {
    let url = url.trim_end_matches('/');
    let spec = GitHubUrlSpec::parse(url)?;

    let skill_name = spec.directory_name();
    let skills_dir = base_dir.join("skills");
    let skill_dir = skills_dir.join(skill_name);
    let config_path = base_dir.join("skills.toml");

    let mut config = SkillsConfig::from_file(&config_path)?;

    // Check if a skill with the same name but different source URL already exists
    if let Some(existing) = config.skills.get(skill_name)
        && existing.source_url != url
    {
        println!(
            "Skill '{}' is already installed from a different source:",
            skill_name
        );
        println!("  Current: {}", existing.source_url);
        println!("  New:     {}", url);

        if !confirm_action("Continue to install with new source?", yes) {
            println!("Installation cancelled.");
            return Ok(());
        }
    }

    let Some(resolved) = resolve(&spec)? else {
        return Err(SkillsError::PathNotFound(url.to_string()));
    };

    if let Some(existing) = config.skills.get(skill_name)
        && skill_dir.exists()
        && let Ok(checksum) = calculate_checksum(&skill_dir)
        && checksum == existing.checksum
    {
        if resolved.r#ref == existing.sha {
            if existing.source_url != url {
                if let Some(entry) = config.skills.get_mut(skill_name) {
                    entry.source_url = url.to_string();
                }
                config.save(&config_path)?;
            }
            println!(
                "Skill '{}' is already installed and up to date.",
                skill_name
            );
            return Ok(());
        } else {
            println!(
                "Skill '{}' is already installed. Upstream ref has moved to new commit, updating...",
                skill_name
            );
        }
    }

    let temp_dir = skills_dir.join(format!(".{}.tmp", skill_name));
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir)?;
    }
    fs::create_dir_all(&temp_dir)?;

    println!("Downloading skill '{}'...", skill_name);
    match download_and_extract(&resolved, &temp_dir) {
        Ok(_) => {
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
                slug: resolved.slug,
                sha: resolved.r#ref,
                path: resolved.path,
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
