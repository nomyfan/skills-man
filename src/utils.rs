use std::{fs, io, path::Path};

use crate::errors::{SkillsError, SkillsResult};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

pub fn calculate_checksum(dir: &Path) -> Result<String, io::Error> {
    let mut hasher = Sha256::new();
    let mut paths: Vec<_> = WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().to_path_buf())
        .collect();

    paths.sort();

    for path in paths {
        let relative = path.strip_prefix(dir).unwrap();
        hasher.update(relative.to_string_lossy().as_bytes());

        let contents = fs::read(&path)?;
        hasher.update(&contents);
    }

    Ok(format!("sha256:{:x}", hasher.finalize()))
}

pub fn ensure_skill_manifest(dir: &Path) -> SkillsResult<()> {
    let entries = fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.eq_ignore_ascii_case("SKILL.md") {
            return Ok(());
        }
    }

    Err(SkillsError::MissingSkillManifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_calculate_checksum() {
        let temp_dir = std::env::temp_dir().join("skills_test_checksum");
        fs::create_dir_all(&temp_dir).unwrap();

        fs::write(temp_dir.join("file1.txt"), b"content1").unwrap();
        fs::write(temp_dir.join("file2.txt"), b"content2").unwrap();

        let checksum1 = calculate_checksum(&temp_dir).unwrap();

        let checksum2 = calculate_checksum(&temp_dir).unwrap();
        assert_eq!(checksum1, checksum2);

        assert!(checksum1.starts_with("sha256:"));

        fs::write(temp_dir.join("file1.txt"), b"modified").unwrap();
        let checksum3 = calculate_checksum(&temp_dir).unwrap();
        assert_ne!(checksum1, checksum3);

        fs::remove_dir_all(&temp_dir).unwrap();
    }
}
