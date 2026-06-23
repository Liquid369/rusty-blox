use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use rocksdb::{DB, WriteBatch, WriteOptions};

pub const IN_PROGRESS: &str = "in-progress";

/// Build a `WriteOptions` configured for bulk-ingest writes.
///
/// During the INITIAL full reindex the entire database is reconstructible from
/// PIVX Core's `.blk` files + LevelDB block index, so the RocksDB WAL is pure
/// fsync overhead on the hot write path. Disabling it makes bulk writes land in
/// the memtable without a per-batch WAL append/sync.
///
/// SAFETY: only ever call this on the initial bulk-sync path. The live / RPC
/// catch-up path MUST keep the WAL (a crash there must be recoverable), so it
/// uses `WriteOptions::default()` (WAL on). After the bulk phase completes the
/// caller flushes all memtables to disk so a later crash starts from a durable
/// state even though no WAL was written.
///
/// This only changes durability/commit granularity — it does NOT change any
/// key/value bytes, so the resulting DB is byte-identical to a WAL-on sync.
pub fn bulk_write_options() -> WriteOptions {
    let mut wo = WriteOptions::new();
    wo.disable_wal(true);
    wo
}
pub const COMPLETED: &str = "completed";
pub const INCOMPLETE: &str = "incomplete";

pub fn load_processed_files_from_db(db: &DB) -> Result<HashMap<PathBuf, String>, String> {
    let cf = db
        .cf_handle("chain_metadata")
        .ok_or("Chain metadata column family not found.")?;
    let mut file_states = HashMap::new();

    for result in db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
        let (key, value) = result.map_err(|e| e.to_string())?;
        let key_path = PathBuf::from(String::from_utf8_lossy(&key).into_owned());
        let state = String::from_utf8(value.to_vec())
            .map_err(|e| format!("Error converting value to String: {e}"))?;
        file_states.insert(key_path, state);
    }

    Ok(file_states)
}

pub async fn save_file_as_in_progress(db: &DB, file_path: &PathBuf) -> Result<(), String> {
    save_file_state(db, file_path, IN_PROGRESS).await
}

pub async fn save_file_as_completed(db: &DB, file_path: &PathBuf) -> Result<(), String> {
    save_file_state(db, file_path, COMPLETED).await
}

pub async fn save_file_as_incomplete(db: &DB, file_path: &PathBuf) -> Result<(), String> {
    save_file_state(db, file_path, INCOMPLETE).await
}

async fn save_file_state(db: &DB, file_path: &PathBuf, state: &str) -> Result<(), String> {
    let cf = db
        .cf_handle("chain_metadata")
        .ok_or("Chain metadata column family not found.")?;
    let key = file_path_to_key(file_path);
    db.put_cf(&cf, key.as_bytes(), state.as_bytes())
        .map_err(|e| e.to_string())
}

fn file_path_to_key(file_path: &PathBuf) -> String {
    file_path.to_string_lossy().into_owned()
}

// RocksDB operations - standalone functions
pub async fn perform_rocksdb_put(
    db: Arc<DB>,
    cf_name: &str,
    key: Vec<u8>,
    value: Vec<u8>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cf_name = cf_name.to_string();
    tokio::task::spawn_blocking(move || {
        let cf = db.cf_handle(&cf_name).ok_or("Column family not found")?;
        db.put_cf(&cf, key, value)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    })
    .await
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
}

pub async fn perform_rocksdb_get(
    db: Arc<DB>,
    cf_name: &str,
    key: Vec<u8>,
) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
    let cf_name = cf_name.to_string();
    tokio::task::spawn_blocking(move || {
        let cf = db.cf_handle(&cf_name).ok_or("Column family not found")?;
        db.get_cf(&cf, key)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    })
    .await
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
}

pub async fn perform_rocksdb_del(
    db: Arc<DB>,
    cf_name: &str,
    key: Vec<u8>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cf_name = cf_name.to_string();
    tokio::task::spawn_blocking(move || {
        let cf = db.cf_handle(&cf_name).ok_or("Column family not found")?;
        db.delete_cf(&cf, key)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    })
    .await
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
}

// Non-column-family helper functions for API handlers
pub async fn db_get_blocking(db: Arc<DB>, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let key = key.to_vec();
    tokio::task::spawn_blocking(move || {
        db.get(&key).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task join error: {e}"))?
}

pub async fn db_put_blocking(db: Arc<DB>, key: &[u8], value: &[u8]) -> Result<(), String> {
    let key = key.to_vec();
    let value = value.to_vec();
    tokio::task::spawn_blocking(move || {
        db.put(&key, &value).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task join error: {e}"))?
}

pub async fn db_delete_blocking(db: Arc<DB>, key: &[u8]) -> Result<(), String> {
    let key = key.to_vec();
    tokio::task::spawn_blocking(move || {
        db.delete(&key).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task join error: {e}"))?
}

/// Batch write multiple key-value pairs to a column family in a single atomic operation
///
/// `bulk` selects the durability mode: on the initial full-reindex path pass
/// `true` to disable the WAL (the DB is fully reconstructible, so the WAL is
/// pure fsync overhead); on the live/RPC catch-up path pass `false` so writes
/// remain WAL-recoverable. Either way the key/value bytes written are identical.
pub async fn batch_put_cf(
    db: Arc<DB>,
    cf_name: &str,
    batch_items: Vec<(Vec<u8>, Vec<u8>)>,
    bulk: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cf_name = cf_name.to_string();
    tokio::task::spawn_blocking(move || {
        let cf = db.cf_handle(&cf_name).ok_or("Column family not found")?;
        let mut batch = WriteBatch::default();
        for (key, value) in batch_items {
            batch.put_cf(&cf, key, value);
        }
        let result = if bulk {
            db.write_opt(batch, &bulk_write_options())
        } else {
            db.write(batch)
        };
        result.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    })
    .await
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
}