use crate::{errors::SkillsResult, models::SkillsConfig, providers::ProviderRegistry};
use std::path::Path;

use super::install::install_skill;

pub fn update_skill(
    name: &str,
    base_dir: &Path,
    yes: bool,
    registry: &ProviderRegistry,
) -> SkillsResult<()> {
    let config_path = base_dir.join("skills.toml");
    let config = SkillsConfig::from_file(&config_path)?;

    let Some(entry) = config.skills.get(name) else {
        println!("Skill '{}' is not installed.", name);
        return Ok(());
    };

    install_skill(&entry.source_url, base_dir, yes, registry)
}

pub fn update_collection_for_skill(
    name: &str,
    base_dir: &Path,
    yes: bool,
    registry: &ProviderRegistry,
) -> SkillsResult<()> {
    let config_path = base_dir.join("skills.toml");
    let config = SkillsConfig::from_file(&config_path)?;

    let Some(entry) = config.skills.get(name) else {
        println!("Skill '{}' is not installed.", name);
        return Ok(());
    };

    let Some(collection_url) = &entry.collection_url else {
        println!(
            "Skill '{}' has no collection metadata. Reinstall its collection to enable collection updates.",
            name
        );
        return Ok(());
    };

    install_skill(collection_url, base_dir, yes, registry)
}
