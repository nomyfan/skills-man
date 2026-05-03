use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::errors::SkillsError;
use crate::errors::SkillsResult;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl AppConfig {
    pub fn from_file<P: AsRef<std::path::Path>>(path: P) -> SkillsResult<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(AppConfig::default());
        }
        let bytes = std::fs::read(path)?;
        let config: AppConfig =
            toml::from_slice(&bytes).map_err(|e| SkillsError::ConfigParseError(e.to_string()))?;
        Ok(config)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SkillsConfig {
    #[serde(default)]
    pub skills: BTreeMap<String, SkillEntry>,
}

impl SkillsConfig {
    pub fn from_file<P: AsRef<std::path::Path>>(path: P) -> SkillsResult<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(SkillsConfig::default());
        }

        let bytes = std::fs::read(path)?;
        let config: SkillsConfig =
            toml::from_slice(&bytes).map_err(|e| SkillsError::ConfigParseError(e.to_string()))?;
        Ok(config)
    }

    pub fn save<P: AsRef<std::path::Path>>(&self, path: P) -> SkillsResult<()> {
        let content = toml::to_string_pretty(self)
            .map_err(|e| SkillsError::ConfigParseError(e.to_string()))?;
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEntry {
    pub source_url: String,
    pub slug: String,
    pub path: String,
    pub sha: String,
    pub checksum: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};

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
