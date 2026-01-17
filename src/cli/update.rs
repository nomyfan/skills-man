use crate::{errors::SkillsResult, models::SkillsConfig};
use std::path::Path;

use super::install::install_skill;

pub fn update_skill(name: &str, base_dir: &Path, yes: bool) -> SkillsResult<()> {
    let config_path = base_dir.join("skills.toml");
    let config = SkillsConfig::from_file(&config_path)?;

    let Some(entry) = config.skills.get(name) else {
        println!("Skill '{}' is not installed.", name);
        return Ok(());
    };

    install_skill(&entry.source_url, base_dir, yes)
}
