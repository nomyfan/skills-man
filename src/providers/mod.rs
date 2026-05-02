pub mod github;

use crate::{
    errors::{SkillsError, SkillsResult},
    models::SkillEntry,
};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ResolvedSkill {
    pub name: String,
    pub source_url: String,
    pub slug: String,
    pub sha: String,
    pub path: String,
}

#[derive(Debug)]
pub struct InstallPlan {
    pub archive_url: String,
    pub is_batch: bool,
    pub skills: Vec<ResolvedSkill>,
}

pub struct ExtractTarget {
    pub path: String,
    pub dest_dir: PathBuf,
}

pub trait SkillProvider: Send + Sync {
    fn handles(&self, url: &str) -> bool;

    fn resolve_install_plan(&self, url: &str) -> SkillsResult<InstallPlan>;

    fn fetch_and_extract(&self, archive_url: &str, targets: &[ExtractTarget]) -> SkillsResult<()>;

    fn archive_url_for_entry(&self, entry: &SkillEntry) -> String;
}

pub struct ProviderRegistry {
    providers: Vec<Box<dyn SkillProvider>>,
}

impl ProviderRegistry {
    pub fn new(providers: Vec<Box<dyn SkillProvider>>) -> Self {
        Self { providers }
    }

    pub fn get(&self, url: &str) -> SkillsResult<&dyn SkillProvider> {
        self.providers
            .iter()
            .find(|p| p.handles(url))
            .map(Box::as_ref)
            .ok_or_else(|| SkillsError::UnsupportedProvider(url.to_string()))
    }
}
