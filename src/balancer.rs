use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use crate::model::BackupTarget;

pub struct TargetStats {
    pub target_id: i64,
    pub active_writes: AtomicU32,
    pub session_bytes: AtomicU64,
}

impl TargetStats {
    pub fn new(target_id: i64) -> Self {
        Self {
            target_id,
            active_writes: AtomicU32::new(0),
            session_bytes: AtomicU64::new(0),
        }
    }

    pub fn acquire(&self) {
        self.active_writes.fetch_add(1, Ordering::Relaxed);
    }

    pub fn release(&self, bytes: u64) {
        self.active_writes.fetch_sub(1, Ordering::Relaxed);
        self.session_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn active_count(&self) -> u32 {
        self.active_writes.load(Ordering::Relaxed)
    }

    pub fn session_written(&self) -> u64 {
        self.session_bytes.load(Ordering::Relaxed)
    }
}

pub struct DiskBalancer {
    entries: Vec<(BackupTarget, Arc<TargetStats>)>,
}

impl DiskBalancer {
    pub fn new(targets: Vec<BackupTarget>) -> Self {
        let entries = targets
            .into_iter()
            .map(|t| {
                let stats = Arc::new(TargetStats::new(t.id));
                (t, stats)
            })
            .collect();
        Self { entries }
    }

    pub fn target_count(&self) -> usize {
        self.entries.len()
    }

    pub fn select_target(&self, needed: u64) -> Option<(&BackupTarget, Arc<TargetStats>)> {
        let mut candidates: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(target, stats)| {
                let path = std::path::Path::new(&target.target_path);
                let free = path
                    .exists()
                    .then(|| disk_free_space(path))
                    .flatten()
                    .unwrap_or(0);

                let available = match target.max_quota {
                    Some(quota) => {
                        let used = stats.session_written();
                        quota.saturating_sub(used as i64).max(0) as u64
                    }
                    None => free,
                };

                if available >= needed {
                    let max_active = self.entries.len() as u32;
                    let space_factor = if available > 0 {
                        (available as f64) / (available.max(1) as f64)
                    } else {
                        0.0
                    };
                    let load_factor = 1.0 - (stats.active_count() as f32 / (max_active as f32 + 1.0)) as f64;
                    let score = space_factor * 0.6 + load_factor * 0.4;
                    Some((score, target, stats.clone(), available))
                } else {
                    None
                }
            })
            .collect();

        candidates.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        candidates
            .into_iter()
            .next()
            .map(|(_, target, stats, _)| (target, stats))
    }

    pub fn snapshot(&self) -> Vec<TargetSnapshot> {
        self.entries
            .iter()
            .map(|(target, stats)| {
                let path = std::path::Path::new(&target.target_path);
                let free = disk_free_space(path).unwrap_or(0);
                TargetSnapshot {
                    target_id: target.id,
                    target_path: target.target_path.clone(),
                    active_writes: stats.active_count(),
                    session_bytes: stats.session_written(),
                    free_space: free,
                }
            })
            .collect()
    }
}

#[derive(Debug)]
pub struct TargetSnapshot {
    pub target_id: i64,
    pub target_path: String,
    pub active_writes: u32,
    pub session_bytes: u64,
    pub free_space: u64,
}

fn disk_free_space(path: &std::path::Path) -> Option<u64> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        let path_str = path.to_str()?;
        let drive = if path_str.len() >= 2 && path_str.as_bytes()[1] == b':' {
            format!("{}\\", &path_str[..2])
        } else {
            path_str.to_string()
        };
        let wide: Vec<u16> = OsStr::new(&drive)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut avail: u64 = 0;
        let mut total: u64 = 0;
        let mut free: u64 = 0;
        extern "system" {
            fn GetDiskFreeSpaceExW(
                dir: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64,
            ) -> i32;
        }
        let ret = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, &mut total, &mut free) };
        if ret != 0 { Some(avail) } else { None }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}
