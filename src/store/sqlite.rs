use async_trait::async_trait;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use sqlx::Row;
use std::collections::HashMap;
use std::str::FromStr;
use crate::error::Result;
use crate::model::*;
use super::MetadataStore;

pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    pub async fn new(url: &str) -> Result<Self> {
        let opts = SqliteConnectOptions::from_str(url)?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal);

        let pool = SqlitePool::connect_with(opts).await?;
        Ok(Self { pool })
    }
}

#[async_trait]
impl MetadataStore for SqliteStore {
    async fn load_backup_roots(&self) -> Result<Vec<BackupRoot>> {
        let rows = sqlx::query_as(
            "SELECT id, root_path, source_type, label, enabled FROM backup_root WHERE enabled = 1"
        ).fetch_all(&self.pool).await?;
        Ok(rows)
    }

    async fn load_backup_targets(&self) -> Result<Vec<BackupTarget>> {
        let rows = sqlx::query_as(
            "SELECT id, target_path, subdir_name, max_quota, enabled FROM backup_target WHERE enabled = 1"
        ).fetch_all(&self.pool).await?;
        Ok(rows)
    }

    async fn begin_session(&self) -> Result<i64> {
        let row = sqlx::query(
            "INSERT INTO backup_session (status) VALUES ('running')"
        ).execute(&self.pool).await?;
        Ok(row.last_insert_rowid())
    }

    async fn finish_session(
        &self,
        session_id: i64,
        file_count: u64,
        data_bytes: u64,
        status: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE backup_session SET end_time = datetime('now'), file_copy_count = ?, data_copy_bytes = ?, status = ? WHERE id = ?"
        )
        .bind(file_count as i64)
        .bind(data_bytes as i64)
        .bind(status)
        .bind(session_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn load_tracked_files_map(&self, root_id: i64) -> Result<HashMap<String, i64>> {
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT id, file_path FROM tracked_file WHERE backup_root_id = ?"
        )
        .bind(root_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|(id, path)| (path, id)).collect())
    }

    async fn insert_tracked_file(&self, root_id: i64, file_path: &str) -> Result<i64> {
        let result = sqlx::query(
            "INSERT INTO tracked_file (backup_root_id, file_path) VALUES (?, ?)"
        )
        .bind(root_id)
        .bind(file_path)
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    async fn load_latest_versions(&self, file_ids: &[i64]) -> Result<HashMap<i64, FileVersion>> {
        if file_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let mut map = HashMap::new();
        for chunk in file_ids.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT v.id, v.tracked_file_id, v.session_id, v.content_hash, v.mtime, v.file_size
                 FROM file_version v
                 INNER JOIN (
                     SELECT tracked_file_id, MAX(id) as max_id
                     FROM file_version
                     WHERE tracked_file_id IN ({})
                     GROUP BY tracked_file_id
                 ) latest ON v.id = latest.max_id",
                placeholders
            );
            let mut q = sqlx::query(&sql);
            for id in chunk {
                q = q.bind(id);
            }
            let rows = q.fetch_all(&self.pool).await?;
            for row in rows {
                let fv = FileVersion {
                    id: row.get(0),
                    tracked_file_id: row.get(1),
                    session_id: row.get(2),
                    content_hash: row.get(3),
                    mtime: row.get(4),
                    file_size: row.get(5),
                };
                map.insert(fv.tracked_file_id, fv);
            }
        }
        Ok(map)
    }

    async fn upsert_content_block(
        &self,
        hash: &str,
        algo: HashAlgo,
        size: i64,
        target_path: &str,
        target_id: i64,
    ) -> Result<bool> {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO content_block (hash, hash_algo, file_size, target_path, target_id, ref_count)
             VALUES (?, ?, ?, ?, ?, 1)"
        )
        .bind(hash)
        .bind(algo.as_str())
        .bind(size)
        .bind(target_path)
        .bind(target_id)
        .execute(&self.pool)
        .await?;

        let is_new = result.rows_affected() > 0;
        if !is_new {
            sqlx::query("UPDATE content_block SET ref_count = ref_count + 1 WHERE hash = ?")
                .bind(hash)
                .execute(&self.pool)
                .await?;
        }
        Ok(is_new)
    }

    async fn insert_file_version(
        &self,
        tracked_file_id: i64,
        session_id: i64,
        hash: &str,
        mtime: i64,
        size: i64,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO file_version (tracked_file_id, session_id, content_hash, mtime, file_size, copy_start, copy_end)
             VALUES (?, ?, ?, ?, ?, datetime('now'), datetime('now'))"
        )
        .bind(tracked_file_id)
        .bind(session_id)
        .bind(hash)
        .bind(mtime)
        .bind(size)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn decrement_block_ref(&self, hash: &str) -> Result<bool> {
        let row = sqlx::query(
            "UPDATE content_block SET ref_count = ref_count - 1 WHERE hash = ? AND ref_count > 0"
        )
        .bind(hash)
        .execute(&self.pool)
        .await?;

        let updated = row.rows_affected() > 0;
        if updated {
            let count: (i64,) = sqlx::query_as("SELECT ref_count FROM content_block WHERE hash = ?")
                .bind(hash)
                .fetch_one(&self.pool)
                .await?;
            if count.0 <= 0 {
                sqlx::query("DELETE FROM content_block WHERE hash = ?")
                    .bind(hash)
                    .execute(&self.pool)
                    .await?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn load_all_blocks(&self) -> Result<Vec<(String, String, i64, String, i64)>> {
        let rows = sqlx::query_as(
            "SELECT hash, hash_algo, file_size, target_path, target_id FROM content_block"
        ).fetch_all(&self.pool).await?;
        Ok(rows)
    }

    async fn load_versions_by_root(
        &self,
        root_id: i64,
    ) -> Result<Vec<(i64, String, i64, String, i64)>> {
        let rows = sqlx::query_as(
            "SELECT fv.id, fv.content_hash, fv.file_size,
                    bt.target_path || '/' || cb.target_path,
                    fv.tracked_file_id
             FROM file_version fv
             INNER JOIN tracked_file tf ON fv.tracked_file_id = tf.id
             INNER JOIN content_block cb ON fv.content_hash = cb.hash
             INNER JOIN backup_target bt ON cb.target_id = bt.id
             WHERE tf.backup_root_id = ?"
        )
        .bind(root_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn delete_version(&self, version_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM file_version WHERE id = ?")
            .bind(version_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn delete_tracked_files_by_root(&self, root_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM tracked_file WHERE backup_root_id = ?")
            .bind(root_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn add_backup_root(
        &self,
        root_path: &str,
        source_type: &str,
        label: Option<&str>,
    ) -> Result<i64> {
        let result = sqlx::query(
            "INSERT INTO backup_root (root_path, source_type, label) VALUES (?, ?, ?)"
        )
        .bind(root_path)
        .bind(source_type)
        .bind(label)
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    async fn add_backup_target(
        &self,
        target_path: &str,
        subdir_name: &str,
        max_quota: Option<i64>,
    ) -> Result<i64> {
        let result = sqlx::query(
            "INSERT INTO backup_target (target_path, subdir_name, max_quota) VALUES (?, ?, ?)"
        )
        .bind(target_path)
        .bind(subdir_name)
        .bind(max_quota)
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    async fn apply_migrations(&self) -> Result<()> {
        let sql = include_str!("../../migrations/001_init.sql");
        sqlx::query(sql).execute(&self.pool).await?;
        tracing::info!("数据库迁移完成");
        Ok(())
    }
}
