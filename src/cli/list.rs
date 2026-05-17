use crate::{errors::SkillsResult, models::SkillsConfig};
use std::path::Path;

pub fn list_skills(base_dir: &Path) -> SkillsResult<()> {
    let config_path = base_dir.join("skills.toml");
    let config = SkillsConfig::from_file(&config_path)?;
    let skills_dir = base_dir.join("skills");

    if config.skills.is_empty() {
        println!("No skills installed.");
        return Ok(());
    }

    println!("Installed skills:");
    println!();

    for (name, entry) in &config.skills {
        println!("  {}", name);
        println!("    Source:    {}", entry.source_url);
        println!("    Repo:      {}", entry.slug);
        println!("    SHA:       {}", &entry.sha[..7.min(entry.sha.len())]);
        println!("    Installed: {}", skills_dir.join(name).display());
        println!();
    }

    println!("Total: {} skill(s)", config.skills.len());

    Ok(())
}
