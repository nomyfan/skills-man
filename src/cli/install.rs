use crate::{
    errors::{SkillsError, SkillsResult},
    models::{SkillEntry, SkillsConfig},
    providers::{ExtractTarget, InstallPlan, ProviderRegistry, ResolvedSkill, SkillProvider},
    utils::{calculate_checksum, ensure_skill_manifest},
};
use std::{fs, path::Path};

use super::prompt::confirm_action_or_yes;

pub fn install_skill(
    url: &str,
    base_dir: &Path,
    yes: bool,
    registry: &ProviderRegistry,
) -> SkillsResult<()> {
    let provider = registry.get(url)?;
    let plan = provider.resolve_install_plan(url)?;
    install_plan(provider, plan, base_dir, yes)
}

fn install_plan(
    provider: &dyn SkillProvider,
    plan: InstallPlan,
    base_dir: &Path,
    yes: bool,
) -> SkillsResult<()> {
    let skills_dir = base_dir.join("skills");
    let config_path = base_dir.join("skills.toml");

    let mut config = SkillsConfig::from_file(&config_path)?;
    let InstallPlan {
        archive_url,
        is_batch,
        skills,
    } = plan;

    if is_batch {
        println!("Found {} skills in directory:", skills.len());
        for skill in &skills {
            println!("  - {}", skill.name);
        }
        println!();

        if !confirm_action_or_yes("Install all these skills?", yes) {
            println!("Installation cancelled.");
            return Ok(());
        }
        println!();
    }

    let per_skill_yes = yes || is_batch;
    let mut pending = Vec::new();

    for skill in skills {
        if should_install_skill(&skill, &mut config, &skills_dir, per_skill_yes) {
            pending.push(skill);
        }
    }

    if pending.is_empty() {
        config.save(&config_path)?;
        return Ok(());
    }

    let temp_root = skills_dir.join(".install.tmp");
    if temp_root.exists() {
        fs::remove_dir_all(&temp_root)?;
    }
    fs::create_dir_all(&temp_root)?;

    println!("Downloading {} skill(s)...", pending.len());
    let targets: Vec<_> = pending
        .iter()
        .map(|skill| ExtractTarget {
            path: skill.path.clone(),
            dest_dir: temp_root.join(&skill.name),
        })
        .collect();

    if let Err(e) = provider.fetch_and_extract(&archive_url, &targets) {
        fs::remove_dir_all(&temp_root).ok();
        return Err(e);
    }

    let mut successful = 0;
    let mut failed = Vec::new();

    for skill in pending {
        match finalize_skill_install(&skill, &mut config, &skills_dir, &temp_root) {
            Ok(_) => successful += 1,
            Err(e) => {
                eprintln!("Failed to install '{}': {}", skill.name, e);
                failed.push(skill.name);
            }
        }
    }

    fs::remove_dir_all(&temp_root).ok();
    config.save(&config_path)?;

    if !failed.is_empty() {
        return Err(SkillsError::BatchInstallationFailed { successful, failed });
    }

    Ok(())
}

fn should_install_skill(
    skill: &ResolvedSkill,
    config: &mut SkillsConfig,
    skills_dir: &Path,
    yes: bool,
) -> bool {
    let skill_dir = skills_dir.join(&skill.name);

    if let Some(existing) = config.skills.get(&skill.name)
        && existing.source_url != skill.source_url
    {
        println!(
            "Skill '{}' is already installed from a different source:",
            skill.name
        );
        println!("  Current: {}", existing.source_url);
        println!("  New:     {}", skill.source_url);

        if !confirm_action_or_yes("Continue to install with new source?", yes) {
            println!("Installation cancelled.");
            return false;
        }
    }

    if let Some(existing) = config.skills.get(&skill.name)
        && skill_dir.exists()
        && let Ok(checksum) = calculate_checksum(&skill_dir)
        && checksum == existing.checksum
    {
        if skill.sha == existing.sha {
            if existing.source_url != skill.source_url
                && let Some(entry) = config.skills.get_mut(&skill.name)
            {
                entry.source_url = skill.source_url.clone();
            }
            println!(
                "Skill '{}' is already installed and up to date.",
                skill.name
            );
            return false;
        }

        println!(
            "Skill '{}' is already installed. Upstream ref has moved to new commit, updating...",
            skill.name
        );
    }

    true
}

fn finalize_skill_install(
    skill: &ResolvedSkill,
    config: &mut SkillsConfig,
    skills_dir: &Path,
    temp_root: &Path,
) -> SkillsResult<()> {
    let temp_dir = temp_root.join(&skill.name);
    let skill_dir = skills_dir.join(&skill.name);

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
        source_url: skill.source_url.clone(),
        slug: skill.slug.clone(),
        sha: skill.sha.clone(),
        path: skill.path.clone(),
        checksum,
    };

    config.skills.insert(skill.name.clone(), entry);
    println!("Successfully installed skill '{}'.", skill.name);

    Ok(())
}
