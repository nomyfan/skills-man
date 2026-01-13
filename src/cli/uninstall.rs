use crate::{errors::SkillsResult, models::SkillsConfig};
use std::{fs, path::Path};

pub fn uninstall_skill(name: &str, base_dir: &Path) -> SkillsResult<()> {
    let config_path = base_dir.join("skills.toml");
    let mut config = SkillsConfig::from_file(&config_path)?;

    let skills_dir = base_dir.join("skills");
    let skill_dir = skills_dir.join(name);

    let mut removed_any = false;
    if skill_dir.exists() {
        fs::remove_dir_all(&skill_dir)?;
        removed_any = true;
    }

    if config.skills.remove(name).is_some() {
        removed_any = true;
        config.save(&config_path)?;
    }

    if removed_any {
        println!("Successfully uninstalled skill '{}'.", name);
    } else {
        println!("Skill '{}' is not installed.", name);
    }

    Ok(())
}
