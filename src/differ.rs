use crate::model::{FileVersion, ScannedFile};

pub struct DiffResult {
    pub needs_backup: bool,
    pub reason: String,
    pub hash_may_match: bool,
}

pub fn check_changed(scanned: &ScannedFile, latest: Option<&FileVersion>) -> DiffResult {
    match latest {
        None => DiffResult {
            needs_backup: true,
            reason: "新文件".to_string(),
            hash_may_match: false,
        },
        Some(v) => {
            if v.mtime == scanned.mtime && v.file_size == scanned.size as i64 {
                DiffResult {
                    needs_backup: false,
                    reason: "未变更".to_string(),
                    hash_may_match: false,
                }
            } else if v.file_size == scanned.size as i64 {
                DiffResult {
                    needs_backup: true,
                    reason: "修改时间变化, 大小相同".to_string(),
                    hash_may_match: true,
                }
            } else {
                DiffResult {
                    needs_backup: true,
                    reason: "已变更".to_string(),
                    hash_may_match: false,
                }
            }
        }
    }
}
