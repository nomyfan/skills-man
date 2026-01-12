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
    pub checksum: String,
}

#[derive(Debug, Clone)]
pub struct GitHubUrlSpec {
    pub owner: String,
    pub repo: String,
    pub tail: Vec<String>,
}

impl GitHubUrlSpec {
    pub fn parse(url: &str) -> SkillsResult<Self> {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| {
            Regex::new(r"^https://github\.com/([^/]+)/([^/]+)/tree/(.+?)/?$").unwrap()
        });

        let url = url.trim_end_matches('/');
        let captures = re
            .captures(url)
            .ok_or_else(|| SkillsError::InvalidUrl(url.to_string()))?;

        let tail: Vec<String> = captures[3]
            .split('/')
            .filter(|part| !part.is_empty())
            .map(|part| part.to_string())
            .collect();

        if tail.len() < 2 {
            return SkillsError::InvalidUrl(url.to_string()).into();
        }

        Ok(Self {
            owner: captures[1].to_string(),
            repo: captures[2].to_string(),
            tail,
        })
    }

    pub fn directory_name(&self) -> &str {
        self.tail.last().map(String::as_str).unwrap()
    }

    pub fn candidates(&self) -> Vec<GitHubUrl> {
        (1..self.tail.len())
            .map(|split| GitHubUrl {
                owner: self.owner.clone(),
                repo: self.repo.clone(),
                r#ref: self.tail[..split].join("/"),
                path: self.tail[split..].join("/"),
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct GitHubUrl {
    pub owner: String,
    pub repo: String,
    pub r#ref: String,
    pub path: String,
}

impl GitHubUrl {
    pub fn tarball_url(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/{}/tarball/{}",
            self.owner, self.repo, self.r#ref
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_url_basic() {
        let url = "https://github.com/anthropics/skills/tree/main/skills/frontend-design";
        let result = GitHubUrlSpec::parse(url).unwrap();

        assert_eq!(result.owner, "anthropics");
        assert_eq!(result.repo, "skills");
        assert_eq!(result.tail, vec!["main", "skills", "frontend-design"]);
    }

    #[test]
    fn test_parse_valid_url_commit_hash() {
        let url = "https://github.com/owner/repo/tree/00756142ab04c82a447693cf373c4e0c554d1005/path/to/dir";
        let result = GitHubUrlSpec::parse(url).unwrap();

        assert_eq!(result.owner, "owner");
        assert_eq!(result.repo, "repo");
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

        assert_eq!(result.owner, "owner");
        assert_eq!(result.repo, "repo");
        assert_eq!(result.tail, vec!["main", "path"]);
    }

    #[test]
    fn test_parse_valid_url_ref_with_slash() {
        let url = "https://github.com/owner/repo/tree/feature/foo/path/to/dir";
        let result = GitHubUrlSpec::parse(url).unwrap();

        assert_eq!(result.owner, "owner");
        assert_eq!(result.repo, "repo");
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
            owner: "owner".to_string(),
            repo: "repo".to_string(),
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
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            tail: vec!["main".to_string(), "skill".to_string()],
        };

        assert_eq!(github_url.directory_name(), "skill");
    }

    #[test]
    fn test_tarball_url() {
        let github_url = GitHubUrl {
            owner: "anthropics".to_string(),
            repo: "skills".to_string(),
            r#ref: "main".to_string(),
            path: "skills/frontend-design".to_string(),
        };

        assert_eq!(
            github_url.tarball_url(),
            "https://api.github.com/repos/anthropics/skills/tarball/main"
        );
    }
}
