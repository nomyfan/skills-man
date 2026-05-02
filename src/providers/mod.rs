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
