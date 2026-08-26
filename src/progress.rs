use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq)]
pub enum BackupState {
    Idle,
    Running,
    Completed { files: u64, bytes: u64 },
    Failed(String),
}

pub struct ProgressTracker {
    state: Mutex<BackupState>,
    total_files: AtomicU64,
    processed_files: AtomicU64,
    total_bytes: AtomicU64,
    processed_bytes: AtomicU64,
    current_file: Mutex<String>,
    current_root: Mutex<String>,
    logs: Mutex<VecDeque<String>>,
}

impl ProgressTracker {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(BackupState::Idle),
            total_files: AtomicU64::new(0),
            processed_files: AtomicU64::new(0),
            total_bytes: AtomicU64::new(0),
            processed_bytes: AtomicU64::new(0),
            current_file: Mutex::new(String::new()),
            current_root: Mutex::new(String::new()),
            logs: Mutex::new(VecDeque::with_capacity(200)),
        }
    }

    pub fn start(&self) {
        self.total_files.store(0, Ordering::Relaxed);
        self.processed_files.store(0, Ordering::Relaxed);
        self.total_bytes.store(0, Ordering::Relaxed);
        self.processed_bytes.store(0, Ordering::Relaxed);
        *self.state.lock().unwrap() = BackupState::Running;
    }

    pub fn set_total_files(&self, total: u64) {
        self.total_files.store(total, Ordering::Relaxed);
    }

    pub fn add_total_bytes(&self, bytes: u64) {
        self.total_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn file_done(&self, bytes: u64) {
        self.processed_files.fetch_add(1, Ordering::Relaxed);
        self.processed_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn set_current_file(&self, path: &str) {
        *self.current_file.lock().unwrap() = path.to_string();
    }

    pub fn set_current_root(&self, root: &str) {
        *self.current_root.lock().unwrap() = root.to_string();
    }

    pub fn complete(&self, files: u64, bytes: u64) {
        *self.state.lock().unwrap() = BackupState::Completed { files, bytes };
    }

    pub fn fail(&self, error: String) {
        *self.state.lock().unwrap() = BackupState::Failed(error);
    }

    pub fn log(&self, msg: String) {
        let mut logs = self.logs.lock().unwrap();
        if logs.len() >= 200 {
            logs.pop_front();
        }
        logs.push_back(msg);
    }

    pub fn is_running(&self) -> bool {
        *self.state.lock().unwrap() == BackupState::Running
    }

    pub fn state(&self) -> BackupState {
        self.state.lock().unwrap().clone()
    }

    pub fn progress_percent(&self) -> f32 {
        let total = self.total_files.load(Ordering::Relaxed);
        if total == 0 {
            0.0
        } else {
            let processed = self.processed_files.load(Ordering::Relaxed);
            processed as f32 / total as f32
        }
    }

    pub fn total_files(&self) -> u64 {
        self.total_files.load(Ordering::Relaxed)
    }

    pub fn processed_files(&self) -> u64 {
        self.processed_files.load(Ordering::Relaxed)
    }

    pub fn processed_bytes(&self) -> u64 {
        self.processed_bytes.load(Ordering::Relaxed)
    }

    pub fn total_bytes(&self) -> u64 {
        self.total_bytes.load(Ordering::Relaxed)
    }

    pub fn current_file(&self) -> String {
        self.current_file.lock().unwrap().clone()
    }

    pub fn current_root(&self) -> String {
        self.current_root.lock().unwrap().clone()
    }

    pub fn logs(&self) -> Vec<String> {
        self.logs.lock().unwrap().iter().cloned().collect()
    }
}

impl Default for ProgressTracker {
    fn default() -> Self {
        Self::new()
    }
}
