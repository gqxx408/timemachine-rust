pub mod sqlite;

use async_trait::async_trait;
use crate::error::Result;
use crate::model::*;
use std::collections::HashMap;

#[async_trait]
pub trait MetadataStore: Send + Sync {
    async fn load_backup_roots(&self) -> Result<Vec<BackupRoot>>;

    async fn load_backup_targets(&self) -> Result<Vec<BackupTarget>>;

    async fn begin_session(&self) -> Result<i64>;

    async fn finish_session(
        &self,
        session_id: i64,
        file_count: u64,
        data_bytes: u64,
        status: &str,
    ) -> Result<()>;

    async fn load_tracked_files_map(
        &self,
        root_id: i64,
    ) -> Result<HashMap<String, i64>>;

    async fn insert_tracked_file(&self, root_id: i64, file_path: &str) -> Result<i64>;

    async fn load_latest_versions(
        &self,
        file_ids: &[i64],
    ) -> Result<HashMap<i64, FileVersion>>;

    async fn upsert_content_block(
        &self,
        hash: &str,
        algo: HashAlgo,
        size: i64,
        target_path: &str,
        target_id: i64,
    ) -> Result<bool>;

    async fn insert_file_version(
        &self,
        tracked_file_id: i64,
        session_id: i64,
        hash: &str,
        mtime: i64,
        size: i64,
    ) -> Result<()>;

    async fn decrement_block_ref(&self, hash: &str) -> Result<bool>;

    async fn load_all_blocks(&self) -> Result<Vec<(String, String, i64, String, i64)>>;

    async fn load_versions_by_root(
        &self,
        root_id: i64,
    ) -> Result<Vec<(i64, String, i64, String, i64)>>;

    async fn delete_version(&self, version_id: i64) -> Result<()>;

    async fn delete_tracked_files_by_root(&self, root_id: i64) -> Result<()>;

    async fn add_backup_root(
        &self,
        root_path: &str,
        source_type: &str,
        label: Option<&str>,
    ) -> Result<i64>;

    async fn add_backup_target(
        &self,
        target_path: &str,
        subdir_name: &str,
        max_quota: Option<i64>,
    ) -> Result<i64>;

    async fn apply_migrations(&self) -> Result<()>;
}
