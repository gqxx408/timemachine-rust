-- TimeMachine Plus Rust - 数据库初始化

CREATE TABLE IF NOT EXISTS backup_root (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    root_path   TEXT     NOT NULL,
    source_type TEXT     NOT NULL DEFAULT 'local',  -- local | remote
    label       TEXT,
    created_at  TEXT     NOT NULL DEFAULT (datetime('now')),
    enabled     INTEGER  NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS backup_target (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    target_path  TEXT     NOT NULL,
    subdir_name  TEXT     NOT NULL DEFAULT 'BACKUPDATABASE',
    max_quota    INTEGER,
    enabled      INTEGER  NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS backup_session (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    begin_time      TEXT     NOT NULL DEFAULT (datetime('now')),
    end_time        TEXT,
    file_copy_count INTEGER  NOT NULL DEFAULT 0,
    data_copy_bytes INTEGER  NOT NULL DEFAULT 0,
    status          TEXT     NOT NULL DEFAULT 'running'  -- running|completed|failed
);

CREATE TABLE IF NOT EXISTS tracked_file (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    backup_root_id  INTEGER NOT NULL REFERENCES backup_root(id),
    file_path       TEXT     NOT NULL,
    UNIQUE(backup_root_id, file_path)
);
CREATE INDEX IF NOT EXISTS idx_tracked_root ON tracked_file(backup_root_id);

CREATE TABLE IF NOT EXISTS content_block (
    hash          TEXT     PRIMARY KEY,
    hash_algo     TEXT     NOT NULL DEFAULT 'blake3',
    file_size     INTEGER  NOT NULL,
    target_path   TEXT     NOT NULL,
    target_id     INTEGER  NOT NULL REFERENCES backup_target(id),
    created_at    TEXT     NOT NULL DEFAULT (datetime('now')),
    ref_count     INTEGER  NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS file_version (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    tracked_file_id  INTEGER NOT NULL REFERENCES tracked_file(id),
    session_id       INTEGER NOT NULL REFERENCES backup_session(id),
    content_hash     TEXT     NOT NULL REFERENCES content_block(hash),
    mtime            INTEGER  NOT NULL,
    file_size        INTEGER  NOT NULL,
    copy_start       TEXT,
    copy_end         TEXT
);
CREATE INDEX IF NOT EXISTS idx_version_file ON file_version(tracked_file_id);
CREATE INDEX IF NOT EXISTS idx_version_session ON file_version(session_id);
