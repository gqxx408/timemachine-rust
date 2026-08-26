use std::path::{Path, PathBuf};
use crate::error::Result;
use crate::model::BackupTarget;

pub fn ensure_target_dir(target: &BackupTarget) -> Result<PathBuf> {
    let target_root = Path::new(&target.target_path);
    let subdir = target_root.join(&target.subdir_name);
    std::fs::create_dir_all(&subdir)?;
    Ok(subdir)
}

pub fn build_rel_path(target: &BackupTarget, hash: &str) -> String {
    let timestamp = chrono::Utc::now().timestamp_millis();
    format!("{}/{}_{}", target.subdir_name, hash, timestamp)
}

pub fn build_dest_path(target: &BackupTarget, rel_path: &str) -> PathBuf {
    Path::new(&target.target_path).join(rel_path)
}

pub fn copy_file_local(src: &Path, dest: &Path) -> Result<u64> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = std::fs::copy(src, dest)?;
    Ok(bytes)
}
