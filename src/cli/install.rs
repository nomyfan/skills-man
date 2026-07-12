use crate::{
    errors::{SkillsError, SkillsResult},
    models::{SkillEntry, SkillsConfig},
    providers::{ExtractTarget, InstallPlan, ProviderRegistry, ResolvedSkill, SkillProvider},
    utils::{calculate_checksum, ensure_skill_manifest},
};
use std::{collections::HashSet, fs, path::Path};

use super::prompt::confirm_action_or_yes;

pub fn install_skill(
    url: &str,
    base_dir: &Path,
    yes: bool,
    registry: &ProviderRegistry,
) -> SkillsResult<()> {
    let provider = registry.get(url)?;
    let plan = provider.resolve_install_plan(url)?;
    install_plan(provider, plan, base_dir, yes)
}

fn install_plan(
    provider: &dyn SkillProvider,
    plan: InstallPlan,
    base_dir: &Path,
    yes: bool,
) -> SkillsResult<()> {
    let skills_dir = base_dir.join("skills");
    let config_path = base_dir.join("skills.toml");

    let mut config = SkillsConfig::from_file(&config_path)?;
    let InstallPlan {
        archive_url,
        is_batch,
        skills,
    } = plan;

    if is_batch {
        println!("Found {} skills in directory:", skills.len());
        for skill in &skills {
            println!("  - {}", skill.name);
        }
        println!();

        if !confirm_action_or_yes("Install all these skills?", yes) {
            println!("Installation cancelled.");
            return Ok(());
        }
        println!();
    }

    let per_skill_yes = yes || is_batch;
    let remote_names: HashSet<String> = skills.iter().map(|skill| skill.name.clone()).collect();
    let collection_url = skills
        .first()
        .and_then(|skill| skill.collection_url.clone());

    let mut pending = Vec::new();

    for skill in skills {
        if should_install_skill(&skill, &mut config, &skills_dir, per_skill_yes) {
            pending.push(skill);
        }
    }

    let mut removal_failures = Vec::new();
    if is_batch {
        removal_failures = remove_orphaned_collection_skills(
            collection_url.as_deref(),
            &remote_names,
            &mut config,
            &skills_dir,
            yes,
        );
    }

    // Persist removals (and any source/collection URL touch-ups from
    // `should_install_skill`) before the fallible download steps below, so a
    // download failure can't leave skills.toml out of sync with the filesystem.
    config.save(&config_path)?;

    if pending.is_empty() {
        if !removal_failures.is_empty() {
            return Err(SkillsError::BatchRemovalFailed {
                failed: removal_failures,
            });
        }
        return Ok(());
    }

    let temp_root = skills_dir.join(".install.tmp");
    if temp_root.exists() {
        fs::remove_dir_all(&temp_root)?;
    }
    fs::create_dir_all(&temp_root)?;

    println!("Downloading {} skill(s)...", pending.len());
    let targets: Vec<_> = pending
        .iter()
        .map(|skill| ExtractTarget {
            path: skill.path.clone(),
            dest_dir: temp_root.join(&skill.name),
        })
        .collect();

    if let Err(e) = provider.fetch_and_extract(&archive_url, &targets) {
        fs::remove_dir_all(&temp_root).ok();
        return Err(e);
    }

    let mut successful = 0;
    let mut failed = Vec::new();

    for skill in pending {
        match finalize_skill_install(&skill, &mut config, &skills_dir, &temp_root) {
            Ok(_) => successful += 1,
            Err(e) => {
                eprintln!("Failed to install '{}': {}", skill.name, e);
                failed.push(skill.name);
            }
        }
    }

    fs::remove_dir_all(&temp_root).ok();
    config.save(&config_path)?;

    if !failed.is_empty() {
        return Err(SkillsError::BatchInstallationFailed { successful, failed });
    }
    if !removal_failures.is_empty() {
        return Err(SkillsError::BatchRemovalFailed {
            failed: removal_failures,
        });
    }

    Ok(())
}

/// Remove skills that were previously installed from `collection_url` but are
/// no longer present in the collection's current remote listing. Returns the
/// names of any skills whose directory could not be removed.
fn remove_orphaned_collection_skills(
    collection_url: Option<&str>,
    remote_names: &HashSet<String>,
    config: &mut SkillsConfig,
    skills_dir: &Path,
    yes: bool,
) -> Vec<String> {
    let Some(collection_url) = collection_url else {
        return Vec::new();
    };

    let orphaned: Vec<String> = config
        .skills
        .iter()
        .filter(|(name, entry)| {
            entry.collection_url.as_deref() == Some(collection_url)
                && !remote_names.contains(name.as_str())
        })
        .map(|(name, _)| name.clone())
        .collect();

    if orphaned.is_empty() {
        return Vec::new();
    }

    println!("The following skills were removed from the collection upstream:");
    for name in &orphaned {
        println!("  - {}", name);
    }

    if !confirm_action_or_yes("Remove them locally?", yes) {
        println!("Skipping removal of orphaned skills.");
        return Vec::new();
    }

    let mut failed = Vec::new();
    for name in orphaned {
        let skill_dir = skills_dir.join(&name);
        if skill_dir.exists()
            && let Err(e) = fs::remove_dir_all(&skill_dir)
        {
            eprintln!("Failed to remove '{}': {}", name, e);
            failed.push(name);
            continue;
        }
        config.skills.remove(&name);
        println!("Removed '{}'", name);
    }
    failed
}

fn should_install_skill(
    skill: &ResolvedSkill,
    config: &mut SkillsConfig,
    skills_dir: &Path,
    yes: bool,
) -> bool {
    let skill_dir = skills_dir.join(&skill.name);

    if let Some(existing) = config.skills.get(&skill.name)
        && existing.source_url != skill.source_url
    {
        println!(
            "Skill '{}' is already installed from a different source:",
            skill.name
        );
        println!("  Current: {}", existing.source_url);
        println!("  New:     {}", skill.source_url);

        if !confirm_action_or_yes("Continue to install with new source?", yes) {
            println!("Installation cancelled.");
            return false;
        }
    }

    if let Some(existing) = config.skills.get(&skill.name)
        && skill_dir.exists()
        && let Ok(checksum) = calculate_checksum(&skill_dir)
        && checksum == existing.checksum
    {
        let old_sha = existing.sha.clone();

        if skill.sha == old_sha {
            if let Some(entry) = config.skills.get_mut(&skill.name) {
                if entry.source_url != skill.source_url {
                    entry.source_url = skill.source_url.clone();
                }
                if entry.collection_url != skill.collection_url {
                    entry.collection_url = skill.collection_url.clone();
                }
            }
            println!(
                "'{}' is already up to date ({})",
                skill.name,
                &old_sha[..7.min(old_sha.len())]
            );
            return false;
        }

        println!(
            "Updating '{}': {} → {}",
            skill.name,
            &old_sha[..7.min(old_sha.len())],
            &skill.sha[..7.min(skill.sha.len())]
        );
    }

    true
}

fn finalize_skill_install(
    skill: &ResolvedSkill,
    config: &mut SkillsConfig,
    skills_dir: &Path,
    temp_root: &Path,
) -> SkillsResult<()> {
    let temp_dir = temp_root.join(&skill.name);
    let skill_dir = skills_dir.join(&skill.name);

    if let Err(e) = ensure_skill_manifest(&temp_dir) {
        fs::remove_dir_all(&temp_dir).ok();
        return Err(e);
    }

    if skill_dir.exists() {
        fs::remove_dir_all(&skill_dir)?;
    }

    fs::rename(&temp_dir, &skill_dir)?;

    let checksum = calculate_checksum(&skill_dir)?;

    let entry = SkillEntry {
        source_url: skill.source_url.clone(),
        collection_url: skill.collection_url.clone(),
        slug: skill.slug.clone(),
        sha: skill.sha.clone(),
        path: skill.path.clone(),
        checksum,
    };

    config.skills.insert(skill.name.clone(), entry);
    println!("Installed '{}'", skill.name);
    println!("  SHA:    {}", &skill.sha[..7.min(skill.sha.len())]);
    println!("  Source: {}", skill.source_url);
    println!("  Path:   {}", skill_dir.display());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeProvider {
        fetch: fn(&str, &[ExtractTarget]) -> SkillsResult<()>,
    }

    impl SkillProvider for FakeProvider {
        fn handles(&self, _url: &str) -> bool {
            true
        }

        fn resolve_install_plan(&self, _url: &str) -> SkillsResult<InstallPlan> {
            unimplemented!("not exercised in these tests")
        }

        fn fetch_and_extract(
            &self,
            archive_url: &str,
            targets: &[ExtractTarget],
        ) -> SkillsResult<()> {
            (self.fetch)(archive_url, targets)
        }

        fn archive_url_for_entry(&self, _entry: &SkillEntry) -> String {
            unimplemented!("not exercised in these tests")
        }
    }

    fn fake_fetch_fails(_archive_url: &str, _targets: &[ExtractTarget]) -> SkillsResult<()> {
        Err(SkillsError::NetworkError("boom".to_string()))
    }

    fn fake_fetch_succeeds(_archive_url: &str, targets: &[ExtractTarget]) -> SkillsResult<()> {
        for target in targets {
            fs::create_dir_all(&target.dest_dir).unwrap();
            fs::write(target.dest_dir.join("SKILL.md"), b"# skill").unwrap();
        }
        Ok(())
    }

    fn orphan_entry(collection_url: &str, sha: &str, checksum: &str) -> SkillEntry {
        SkillEntry {
            source_url: format!("{collection_url}/orphan-skill"),
            collection_url: Some(collection_url.to_string()),
            slug: "owner/repo".to_string(),
            path: "skills/orphan-skill".to_string(),
            sha: sha.to_string(),
            checksum: checksum.to_string(),
        }
    }

    #[test]
    fn test_remove_orphaned_collection_skills_reports_failed_removal() {
        let temp_dir = std::env::temp_dir().join("skills_test_orphan_removal_fails");
        let skills_dir = temp_dir.join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        // Force `fs::remove_dir_all` to fail deterministically by making the
        // "directory" a regular file instead.
        let orphan_path = skills_dir.join("orphan-skill");
        fs::write(&orphan_path, b"not a directory").unwrap();

        let collection_url = "https://github.com/owner/repo/tree/main/skills";
        let mut config = SkillsConfig::default();
        config.skills.insert(
            "orphan-skill".to_string(),
            orphan_entry(collection_url, "orphan-sha", "sha256:deadbeef"),
        );

        let remote_names: HashSet<String> = HashSet::new();

        let failed = remove_orphaned_collection_skills(
            Some(collection_url),
            &remote_names,
            &mut config,
            &skills_dir,
            true,
        );

        assert_eq!(failed, vec!["orphan-skill".to_string()]);
        // Deletion failed, so the entry must be retained to match the filesystem.
        assert!(config.skills.contains_key("orphan-skill"));

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_install_plan_persists_orphan_removal_before_download_failure() {
        let base_dir = std::env::temp_dir().join("skills_test_install_persist_before_failure");
        let skills_dir = base_dir.join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let collection_url = "https://github.com/owner/repo/tree/main/skills";

        let orphan_dir = skills_dir.join("orphan-skill");
        fs::create_dir_all(&orphan_dir).unwrap();
        fs::write(orphan_dir.join("SKILL.md"), b"# orphan").unwrap();
        let orphan_checksum = calculate_checksum(&orphan_dir).unwrap();

        let mut config = SkillsConfig::default();
        config.skills.insert(
            "orphan-skill".to_string(),
            orphan_entry(collection_url, "orphan-sha", &orphan_checksum),
        );
        config.save(base_dir.join("skills.toml")).unwrap();

        let plan = InstallPlan {
            archive_url: "archive://fake".to_string(),
            is_batch: true,
            skills: vec![ResolvedSkill {
                name: "new-skill".to_string(),
                source_url: format!("{collection_url}/new-skill"),
                collection_url: Some(collection_url.to_string()),
                slug: "owner/repo".to_string(),
                sha: "new-sha".to_string(),
                path: "skills/new-skill".to_string(),
            }],
        };

        let provider = FakeProvider {
            fetch: fake_fetch_fails,
        };

        let result = install_plan(&provider, plan, &base_dir, true);

        match result {
            Err(SkillsError::NetworkError(msg)) => assert_eq!(msg, "boom"),
            other => panic!("expected NetworkError, got {other:?}"),
        }

        // The orphan removal must be durable even though the download that
        // follows it failed.
        assert!(!orphan_dir.exists());
        let reloaded = SkillsConfig::from_file(base_dir.join("skills.toml")).unwrap();
        assert!(!reloaded.skills.contains_key("orphan-skill"));

        fs::remove_dir_all(&base_dir).unwrap();
    }

    #[test]
    fn test_install_plan_reports_batch_removal_failed_when_pending_is_empty() {
        let base_dir = std::env::temp_dir().join("skills_test_install_removal_failed_no_pending");
        let skills_dir = base_dir.join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let collection_url = "https://github.com/owner/repo/tree/main/skills";

        // Already up to date locally, so it won't need downloading.
        let existing_dir = skills_dir.join("existing-skill");
        fs::create_dir_all(&existing_dir).unwrap();
        fs::write(existing_dir.join("SKILL.md"), b"# existing").unwrap();
        let existing_checksum = calculate_checksum(&existing_dir).unwrap();

        // Orphan whose directory can't be removed.
        let orphan_path = skills_dir.join("orphan-skill");
        fs::write(&orphan_path, b"not a directory").unwrap();

        let mut config = SkillsConfig::default();
        config.skills.insert(
            "existing-skill".to_string(),
            SkillEntry {
                source_url: format!("{collection_url}/existing-skill"),
                collection_url: Some(collection_url.to_string()),
                slug: "owner/repo".to_string(),
                path: "skills/existing-skill".to_string(),
                sha: "same-sha".to_string(),
                checksum: existing_checksum,
            },
        );
        config.skills.insert(
            "orphan-skill".to_string(),
            orphan_entry(collection_url, "orphan-sha", "sha256:deadbeef"),
        );
        config.save(base_dir.join("skills.toml")).unwrap();

        let plan = InstallPlan {
            archive_url: "archive://fake".to_string(),
            is_batch: true,
            skills: vec![ResolvedSkill {
                name: "existing-skill".to_string(),
                source_url: format!("{collection_url}/existing-skill"),
                collection_url: Some(collection_url.to_string()),
                slug: "owner/repo".to_string(),
                sha: "same-sha".to_string(),
                path: "skills/existing-skill".to_string(),
            }],
        };

        // Must never be called: nothing should end up pending.
        let provider = FakeProvider {
            fetch: fake_fetch_fails,
        };

        let result = install_plan(&provider, plan, &base_dir, true);

        match result {
            Err(SkillsError::BatchRemovalFailed { failed }) => {
                assert_eq!(failed, vec!["orphan-skill".to_string()]);
            }
            other => panic!("expected BatchRemovalFailed, got {other:?}"),
        }

        fs::remove_dir_all(&base_dir).unwrap();
    }

    #[test]
    fn test_install_plan_reports_batch_removal_failed_after_successful_install() {
        let base_dir =
            std::env::temp_dir().join("skills_test_install_removal_failed_after_success");
        let skills_dir = base_dir.join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let collection_url = "https://github.com/owner/repo/tree/main/skills";

        // Orphan whose directory can't be removed.
        let orphan_path = skills_dir.join("orphan-skill");
        fs::write(&orphan_path, b"not a directory").unwrap();

        let mut config = SkillsConfig::default();
        config.skills.insert(
            "orphan-skill".to_string(),
            orphan_entry(collection_url, "orphan-sha", "sha256:deadbeef"),
        );
        config.save(base_dir.join("skills.toml")).unwrap();

        let plan = InstallPlan {
            archive_url: "archive://fake".to_string(),
            is_batch: true,
            skills: vec![ResolvedSkill {
                name: "new-skill".to_string(),
                source_url: format!("{collection_url}/new-skill"),
                collection_url: Some(collection_url.to_string()),
                slug: "owner/repo".to_string(),
                sha: "new-sha".to_string(),
                path: "skills/new-skill".to_string(),
            }],
        };

        let provider = FakeProvider {
            fetch: fake_fetch_succeeds,
        };

        let result = install_plan(&provider, plan, &base_dir, true);

        match result {
            Err(SkillsError::BatchRemovalFailed { failed }) => {
                assert_eq!(failed, vec!["orphan-skill".to_string()]);
            }
            other => panic!("expected BatchRemovalFailed, got {other:?}"),
        }

        // The unrelated install still completed despite the removal failure.
        let reloaded = SkillsConfig::from_file(base_dir.join("skills.toml")).unwrap();
        assert!(reloaded.skills.contains_key("new-skill"));
        assert!(reloaded.skills.contains_key("orphan-skill"));

        fs::remove_dir_all(&base_dir).unwrap();
    }
}
