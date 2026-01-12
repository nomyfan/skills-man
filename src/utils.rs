use std::{fs, io, path::Path};

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
