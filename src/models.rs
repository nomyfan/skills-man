use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::OnceLock};

use crate::errors::SkillsError;
use crate::errors::SkillsResult;

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

#[derive(Debug, Clone)]
pub struct GitHubUrlSpec {
    pub slug: String,
    pub tail: Vec<String>,
}

impl GitHubUrlSpec {
    pub fn parse(url: &str) -> SkillsResult<Self> {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| {
            Regex::new(r"^https://github\.com/([^/]+/[^/]+)/tree/(.+?)/?$").unwrap()
        });

        let url = url.trim_end_matches('/');
        let captures = re
            .captures(url)
            .ok_or_else(|| SkillsError::InvalidUrl(url.to_string()))?;

        let tail: Vec<String> = captures[2]
            .split('/')
            .filter(|part| !part.is_empty())
            .map(|part| part.to_string())
            .collect();

        if tail.len() < 2 {
            return SkillsError::InvalidUrl(url.to_string()).into();
        }

        Ok(Self {
            slug: captures[1].to_string(),
            tail,
        })
    }

    pub fn directory_name(&self) -> &str {
        self.tail.last().map(String::as_str).unwrap()
    }

    pub fn candidates(&self) -> Vec<GitHubUrl> {
        (1..self.tail.len())
            .map(|split| GitHubUrl {
                slug: self.slug.clone(),
                r#ref: self.tail[..split].join("/"),
                path: self.tail[split..].join("/"),
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct GitHubUrl {
    pub slug: String,
    pub r#ref: String,
    pub path: String,
}

impl GitHubUrl {
    pub fn tarball_url(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/tarball/{}",
            self.slug, self.r#ref
        )
    }

    pub fn commits_url(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/commits?ref={}&path={}&per_page=1",
            self.slug,
            urlencoding::encode(&self.r#ref),
            urlencoding::encode(&self.path)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};

    #[test]
    fn test_parse_valid_url_basic() {
        let url = "https://github.com/anthropics/skills/tree/main/skills/frontend-design";
        let result = GitHubUrlSpec::parse(url).unwrap();

        assert_eq!(result.slug, "anthropics/skills");
        assert_eq!(result.tail, vec!["main", "skills", "frontend-design"]);
    }

    #[test]
    fn test_parse_valid_url_commit_hash() {
        let url = "https://github.com/owner/repo/tree/00756142ab04c82a447693cf373c4e0c554d1005/path/to/dir";
        let result = GitHubUrlSpec::parse(url).unwrap();

        assert_eq!(result.slug, "owner/repo");
        assert_eq!(
            result.tail,
            vec![
                "00756142ab04c82a447693cf373c4e0c554d1005",
                "path",
                "to",
                "dir"
            ]
        );
    }

    #[test]
    fn test_parse_valid_url_trailing_slash() {
        let url = "https://github.com/owner/repo/tree/main/path/";
        let result = GitHubUrlSpec::parse(url).unwrap();

        assert_eq!(result.slug, "owner/repo");
        assert_eq!(result.tail, vec!["main", "path"]);
    }

    #[test]
    fn test_parse_valid_url_ref_with_slash() {
        let url = "https://github.com/owner/repo/tree/feature/foo/path/to/dir";
        let result = GitHubUrlSpec::parse(url).unwrap();

        assert_eq!(result.slug, "owner/repo");
        assert_eq!(result.tail, vec!["feature", "foo", "path", "to", "dir"]);
    }

    #[test]
    fn test_candidates_include_slash_ref() {
        let candidates =
            GitHubUrlSpec::parse("https://github.com/owner/repo/tree/release/v1.0/hotfix/skill")
                .unwrap()
                .candidates();
        assert!(candidates.iter().any(|candidate| {
            candidate.r#ref == "release/v1.0" && candidate.path == "hotfix/skill"
        }));
    }

    #[test]
    fn test_parse_invalid_url_missing_tree() {
        let url = "https://github.com/owner/repo";
        let result = GitHubUrlSpec::parse(url);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SkillsError::InvalidUrl(_)));
    }

    #[test]
    fn test_parse_invalid_url_wrong_protocol() {
        let url = "http://github.com/owner/repo/tree/main/path";
        let result = GitHubUrlSpec::parse(url);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SkillsError::InvalidUrl(_)));
    }

    #[test]
    fn test_parse_invalid_url_missing_path() {
        let url = "https://github.com/owner/repo/tree/main";
        let result = GitHubUrlSpec::parse(url);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SkillsError::InvalidUrl(_)));
    }

    #[test]
    fn test_directory_name() {
        let github_url = GitHubUrlSpec {
            slug: "owner/repo".to_string(),
            tail: vec![
                "main".to_string(),
                "skills".to_string(),
                "frontend-design".to_string(),
            ],
        };

        assert_eq!(github_url.directory_name(), "frontend-design");
    }

    #[test]
    fn test_directory_name_single_component() {
        let github_url = GitHubUrlSpec {
            slug: "owner/repo".to_string(),
            tail: vec!["main".to_string(), "skill".to_string()],
        };

        assert_eq!(github_url.directory_name(), "skill");
    }

    #[test]
    fn test_tarball_url() {
        let github_url = GitHubUrl {
            slug: "anthropics/skills".to_string(),
            r#ref: "main".to_string(),
            path: "skills/frontend-design".to_string(),
        };

        assert_eq!(
            github_url.tarball_url(),
            "https://api.github.com/repos/anthropics/skills/tarball/main"
        );
    }

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
        assert!(matches!(result.unwrap_err(), SkillsError::ConfigParseError(_)));

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
