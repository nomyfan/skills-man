use crate::{
    cli::github::{
        ContentsEntry, ContentsResponse, build_agent, download_and_extract, fetch_contents_sha,
        resolve_commit_sha,
    },
    errors::{SkillsError, SkillsResult},
    models::{GitHubUrl, GitHubUrlSpec, GitRef, SkillEntry, SkillsConfig},
    utils::calculate_checksum,
};
use std::{
    fs, io,
    io::{IsTerminal, Write as IoWrite},
    path::Path,
};

/// Prompt user for confirmation
/// Returns true if yes flag is set, or if user confirms in interactive mode
/// Returns false if stdin is not a TTY (non-interactive context)
fn confirm_action(prompt: &str, yes: bool) -> bool {
    if yes {
        return true;
    }

    if !io::stdin().is_terminal() {
        eprintln!("Non-interactive mode detected. Use --yes flag to auto-confirm.");
        return false;
    }

    print!("{} (y/N): ", prompt);
    io::stdout().flush().ok();

    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();
    let answer = input.trim().to_lowercase();

    answer == "y" || answer == "yes"
}

pub fn install_skill(url: &str, base_dir: &Path, yes: bool) -> SkillsResult<()> {
    let url = url.trim_end_matches('/');
    let spec = GitHubUrlSpec::parse(url)?;
    let agent = build_agent()?;

    let mut resolved: Option<(GitHubUrl, ContentsResponse)> = None;
    for candidate in spec.candidates() {
        match fetch_contents_sha(&agent, &candidate) {
            Ok(contents) => {
                resolved = Some((candidate, contents));
                break;
            }
            Err(SkillsError::NotFound { .. }) => continue,
            Err(e) => return Err(e),
        }
    }
    let (resolved, contents) =
        resolved.ok_or_else(|| SkillsError::PathNotFound(url.to_string()))?;

    if contents.r#type != "dir" {
        return Err(SkillsError::InvalidResponse(format!(
            "Expected a directory but got a {}",
            contents.r#type
        )));
    }

    let entries = contents.entries.unwrap_or_default();
    let is_single = entries
        .iter()
        .any(|e| e.name.eq_ignore_ascii_case("SKILL.md") && e.r#type == "file");

    if is_single {
        let skill_name = spec.directory_name();
        install_single_skill(
            &agent,
            url,
            resolved,
            base_dir,
            skill_name,
            yes,
            &contents.sha,
        )
    } else {
        install_batch_skills(&agent, url, resolved, base_dir, yes, &entries)
    }
}

fn install_single_skill(
    agent: &ureq::Agent,
    source_url: &str,
    resolved: GitHubUrl,
    base_dir: &Path,
    skill_name: &str,
    yes: bool,
    sha: &str,
) -> SkillsResult<()> {
    let skills_dir = base_dir.join("skills");
    let skill_dir = skills_dir.join(skill_name);
    let config_path = base_dir.join("skills.toml");

    let mut config = SkillsConfig::from_file(&config_path)?;

    // Check if a skill with the same name but different source URL already exists
    if let Some(existing) = config.skills.get(skill_name)
        && existing.source_url != source_url
    {
        println!(
            "Skill '{}' is already installed from a different source:",
            skill_name
        );
        println!("  Current: {}", existing.source_url);
        println!("  New:     {}", source_url);

        if !confirm_action("Continue to install with new source?", yes) {
            println!("Installation cancelled.");
            return Ok(());
        }
    }

    // Check if already installed and up to date
    if let Some(existing) = config.skills.get(skill_name)
        && skill_dir.exists()
        && sha == existing.sha
        && let Ok(checksum) = calculate_checksum(&skill_dir)
        && checksum == existing.checksum
    {
        if existing.source_url != source_url {
            if let Some(entry) = config.skills.get_mut(skill_name) {
                entry.source_url = source_url.to_string();
            }
            config.save(&config_path)?;
        }
        println!(
            "Skill '{}' is already installed and up to date.",
            skill_name
        );
        return Ok(());
    }

    // Need to download
    println!("Installing skill '{}'...", skill_name);

    let temp_dir = skills_dir.join(".download.tmp");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir)?;
    }
    fs::create_dir_all(&temp_dir)?;

    if let Err(e) = download_and_extract(agent, &resolved, &temp_dir) {
        fs::remove_dir_all(&temp_dir).ok();
        return Err(e);
    }

    // Move temp directory to final location
    if skill_dir.exists() {
        fs::remove_dir_all(&skill_dir)?;
    }
    fs::rename(&temp_dir, &skill_dir)?;

    let checksum = calculate_checksum(&skill_dir)?;

    let commit_sha = match &resolved.r#ref {
        GitRef::CommitSHA(sha) => sha.clone(),
        GitRef::Other(_) => resolve_commit_sha(agent, &resolved)?,
    };
    let entry = SkillEntry {
        source_url: source_url.to_string(),
        slug: resolved.slug,
        commit: commit_sha,
        sha: sha.to_string(),
        path: resolved.path,
        checksum,
    };

    config.skills.insert(skill_name.to_string(), entry);
    config.save(&config_path)?;

    println!("Successfully installed skill '{}'.", skill_name);
    Ok(())
}

/// Check if a skill needs update by comparing SHA
fn skill_needs_update(
    config: &SkillsConfig,
    skill_name: &str,
    skill_dir: &Path,
    sha: &str,
) -> bool {
    let Some(existing) = config.skills.get(skill_name) else {
        return true; // Not installed
    };
    if !skill_dir.exists() {
        return true; // Directory missing
    }
    if sha != existing.sha {
        return true; // SHA changed
    }
    // Check local modifications
    let Ok(checksum) = calculate_checksum(skill_dir) else {
        return true;
    };
    checksum != existing.checksum
}

fn install_batch_skills(
    agent: &ureq::Agent,
    base_url: &str,
    resolved: GitHubUrl,
    base_dir: &Path,
    yes: bool,
    entries: &[ContentsEntry],
) -> SkillsResult<()> {
    let skills_dir = base_dir.join("skills");
    let config_path = base_dir.join("skills.toml");
    let mut config = SkillsConfig::from_file(&config_path)?;

    let skill_entries: Vec<&ContentsEntry> = entries.iter().filter(|e| e.r#type == "dir").collect();

    if skill_entries.is_empty() {
        return Err(SkillsError::NoSkillsFound(resolved.path.clone()));
    }

    // Check which skills need update
    let skills_to_update: Vec<(&str, &str)> = skill_entries
        .iter()
        .filter_map(|e| {
            let skill_name = &e.name;
            let skill_dir = skills_dir.join(skill_name);
            if skill_needs_update(&config, skill_name, &skill_dir, &e.sha) {
                Some((skill_name.as_ref(), e.sha.as_str()))
            } else {
                None
            }
        })
        .collect();

    if skills_to_update.is_empty() {
        println!("All {} skills are already up to date.", skill_entries.len());
        return Ok(());
    }

    println!("Found {} skills to install/update:", skills_to_update.len());
    for (subdir, _) in &skills_to_update {
        println!("  - {}", subdir);
    }
    println!();

    if !confirm_action("Install these skills?", yes) {
        println!("Installation cancelled.");
        return Ok(());
    }

    println!();

    // Download the entire top-level directory once
    let temp_dir = skills_dir.join(".download.tmp");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir)?;
    }
    fs::create_dir_all(&temp_dir)?;

    if let Err(e) = download_and_extract(agent, &resolved, &temp_dir) {
        fs::remove_dir_all(&temp_dir).ok();
        return Err(e);
    }

    // Move each sub-skill directory to final location
    let mut successful = 0;
    let mut failed = Vec::new();

    for (subdir, sha) in &skills_to_update {
        let source_url = format!("{}/{}", base_url, subdir);
        let temp_skill_dir = temp_dir.join(subdir);
        let skill_dir = skills_dir.join(*subdir);

        if !temp_skill_dir.exists() {
            eprintln!(
                "Failed to install '{}': directory not found in download",
                subdir
            );
            failed.push(subdir.to_string());
            continue;
        }

        // Move to final location
        if skill_dir.exists()
            && let Err(e) = fs::remove_dir_all(&skill_dir)
        {
            eprintln!("Failed to install '{}': {}", subdir, e);
            failed.push(subdir.to_string());
            continue;
        }

        if let Err(e) = fs::rename(&temp_skill_dir, &skill_dir) {
            eprintln!("Failed to install '{}': {}", subdir, e);
            failed.push(subdir.to_string());
            continue;
        }

        let checksum = match calculate_checksum(&skill_dir) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to install '{}': {}", subdir, e);
                failed.push(subdir.to_string());
                continue;
            }
        };

        let commit_sha = match &resolved.r#ref {
            GitRef::CommitSHA(sha) => sha.clone(),
            GitRef::Other(_) => {
                let skill_github_url = GitHubUrl {
                    slug: resolved.slug.clone(),
                    r#ref: resolved.r#ref.clone(),
                    path: format!("{}/{}", resolved.path, subdir),
                };
                resolve_commit_sha(agent, &skill_github_url)?
            }
        };

        let entry = SkillEntry {
            source_url,
            slug: resolved.slug.clone(),
            commit: commit_sha,
            sha: (*sha).to_string(),
            path: format!("{}/{}", resolved.path, subdir),
            checksum,
        };

        config.skills.insert((*subdir).to_string(), entry);
        config.save(&config_path)?;
        println!("Successfully installed skill '{}'.", subdir);
        successful += 1;
    }

    // Clean up temp directory
    fs::remove_dir_all(&temp_dir).ok();

    println!();
    println!(
        "Successfully installed: {}/{}",
        successful,
        skills_to_update.len()
    );

    if !failed.is_empty() {
        println!("Failed skills:");
        for skill in &failed {
            println!("  - {}", skill);
        }
        println!();
        return Err(SkillsError::BatchInstallationFailed { successful, failed });
    }

    Ok(())
}
