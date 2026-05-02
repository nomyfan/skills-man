pub mod github;

use crate::{
    errors::{SkillsError, SkillsResult},
    models::SkillEntry,
};
use std::path::PathBuf;

/// A resolved skill ready to be installed.
#[derive(Debug, Clone)]
pub struct ResolvedSkill {
    /// The skill name, derived from the last path segment of the source URL.
    pub name: String,
    /// The user-provided URL that points to this skill (or the batch root).
    pub source_url: String,
    /// Repository identifier in `owner/repo` format.
    pub slug: String,
    /// Pinned commit SHA for reproducible installs.
    pub sha: String,
    /// Path within the repository archive where the skill files live.
    pub path: String,
}

/// The result of resolving a source URL into one or more skills.
#[derive(Debug)]
pub struct InstallPlan {
    /// URL of the archive to download.
    pub archive_url: String,
    /// Whether this plan covers multiple skills from a single archive.
    pub is_batch: bool,
    /// Skills to install from the archive.
    pub skills: Vec<ResolvedSkill>,
}

/// Describes which path to extract from an archive and where to put it.
pub struct ExtractTarget {
    /// Path prefix inside the archive to extract.
    pub path: String,
    /// Local destination directory for the extracted files.
    pub dest_dir: PathBuf,
}

/// A registered skill source provider (e.g. GitHub, GitLab).
pub trait SkillProvider: Send + Sync {
    /// Returns `true` if this provider supports the given URL.
    fn handles(&self, url: &str) -> bool;

    /// Parse the URL, resolve refs to SHAs, detect single vs batch skill layout,
    /// and return an [`InstallPlan`] ready for download.
    fn resolve_install_plan(&self, url: &str) -> SkillsResult<InstallPlan>;

    /// Download `archive_url` and extract each target into its destination.
    /// `archive_url` is opaque to callers — only the provider that produced it
    /// knows how to fetch it.
    fn fetch_and_extract(&self, archive_url: &str, targets: &[ExtractTarget]) -> SkillsResult<()>;

    /// Reconstruct the archive URL from a stored [`SkillEntry`] for sync.
    fn archive_url_for_entry(&self, entry: &SkillEntry) -> String;
}

/// Holds all registered [`SkillProvider`] instances and routes URLs to the
/// appropriate one.
pub struct ProviderRegistry {
    providers: Vec<Box<dyn SkillProvider>>,
}

impl ProviderRegistry {
    pub fn new(providers: Vec<Box<dyn SkillProvider>>) -> Self {
        Self { providers }
    }

    /// Return the first provider whose [`SkillProvider::handles`] returns `true`
    /// for `url`, or [`SkillsError::UnsupportedProvider`] if none match.
    pub fn get(&self, url: &str) -> SkillsResult<&dyn SkillProvider> {
        self.providers
            .iter()
            .find(|p| p.handles(url))
            .map(Box::as_ref)
            .ok_or_else(|| SkillsError::UnsupportedProvider(url.to_string()))
    }
}
