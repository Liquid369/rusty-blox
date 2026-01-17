/// Atomic Multi-CF Database Writer
/// 
/// Provides atomic write operations across multiple RocksDB column families.
/// This ensures that either ALL writes succeed together, or NONE do - critical
/// for maintaining database consistency during crashes or errors.
/// 
/// WITHOUT atomic writes:
/// - Crash during sync → partial state (blocks written, metadata missing)
/// - Database corruption requiring full resync
/// - Reorg rollback impossible to implement safely
/// 
/// WITH atomic writes:
/// - All-or-nothing commits across CFs
/// - Safe crash recovery
/// - Foundation for reorg handling
/// - Guaranteed consistency

use std::sync::Arc;
use std::collections::HashMap;
use rocksdb::{DB, WriteBatch};
use tracing::{debug, warn, error, info};
use crate::metrics;

/// Atomic batch writer that commits writes across multiple column families atomically
pub struct AtomicBatchWriter {
    db: Arc<DB>,
    operations: Vec<Operation>,
    batch_size_limit: usize,
}

/// Represents a single database operation
#[derive(Clone)]
enum Operation {
    Put {
        cf_name: String,
        key: Vec<u8>,
        value: Vec<u8>,
    },
    Delete {
        cf_name: String,
        key: Vec<u8>,
    },
}

impl AtomicBatchWriter {
    /// Create a new atomic batch writer
    /// 
    /// # Arguments
    /// * `db` - RocksDB instance
    /// * `batch_size_limit` - Maximum number of operations before auto-flush
    pub fn new(db: Arc<DB>, batch_size_limit: usize) -> Self {
        Self {
            db,
            operations: Vec::new(),
            batch_size_limit,
        }
    }

    /// Add a put operation to the batch
    /// 
    /// # Arguments
    /// * `cf_name` - Column family name
    /// * `key` - Key bytes
    /// * `value` - Value bytes
    pub fn put(&mut self, cf_name: &str, key: Vec<u8>, value: Vec<u8>) {
        self.operations.push(Operation::Put {
            cf_name: cf_name.to_string(),
            key,
            value,
        });
    }

    /// Add a delete operation to the batch
    /// 
    /// # Arguments
    /// * `cf_name` - Column family name
    /// * `key` - Key bytes to delete
    pub fn delete(&mut self, cf_name: &str, key: Vec<u8>) {
        self.operations.push(Operation::Delete {
            cf_name: cf_name.to_string(),
            key,
        });
    }

    /// Check if batch should be flushed based on size
    pub fn should_flush(&self) -> bool {
        self.operations.len() >= self.batch_size_limit
    }

    /// Get number of pending operations
    pub fn pending_count(&self) -> usize {
        self.operations.len()
    }

    /// Flush all accumulated writes to database ATOMICALLY
    /// 
    /// This is the critical function that ensures all-or-nothing semantics.
    /// All operations across all column families are committed in a single
    /// atomic RocksDB WriteBatch.
    /// 
    /// # Returns
    /// Ok(()) if all writes succeeded, Err if any write failed (none committed)
    pub async fn flush(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if self.operations.is_empty() {
            return Ok(());
        }

        let pending_ops = self.operations.len();
        let timer = metrics::Timer::new();
        
        debug!(pending_ops = pending_ops, "Batch flush start");

        // Move operations out to process
        let operations = std::mem::take(&mut self.operations);
        let db = self.db.clone();

        // Perform atomic commit in blocking task  
        let task_result = tokio::task::spawn_blocking(move || -> (Result<(), String>, HashMap<String, usize>) {
            // Create single WriteBatch for ALL operations across ALL CFs
            let mut batch = WriteBatch::default();
            
            // Group operations by CF for efficient handle lookup
            let mut cf_operations: HashMap<String, Vec<&Operation>> = HashMap::new();
            for op in &operations {
                let cf_name = match op {
                    Operation::Put { cf_name, .. } => cf_name,
                    Operation::Delete { cf_name, .. } => cf_name,
                };
                cf_operations.entry(cf_name.clone())
                    .or_default()
                    .push(op);
            }
            
            // Track batch sizes per CF for metrics
            let mut cf_batch_sizes: HashMap<String, usize> = HashMap::new();

            // Add all operations to the single WriteBatch
            for (cf_name, ops) in cf_operations {
                let cf = match db.cf_handle(&cf_name) {
                    Some(cf) => cf,
                    None => {
                        let err_msg = format!("Column family not found: {}", cf_name);
                        return (Err(err_msg), HashMap::new());
                    }
                };
                
                cf_batch_sizes.insert(cf_name.clone(), ops.len());
                
                for op in ops {
                    match op {
                        Operation::Put { key, value, .. } => {
                            batch.put_cf(&cf, key, value);
                        }
                        Operation::Delete { key, .. } => {
                            batch.delete_cf(&cf, key);
                        }
                    }
                }
            }

            // CRITICAL: Single atomic commit for ALL column families
            // Either everything succeeds, or nothing does
            let write_result = db.write(batch)
                .map_err(|e| e.to_string());
            
            // Return both the result and metadata regardless of success/failure
            (write_result, cf_batch_sizes)
        })
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        
        let elapsed_secs = timer.elapsed_secs();
        let elapsed_ms = elapsed_secs * 1000.0;
        
        let (write_result, cf_batch_sizes) = task_result;
        
        match write_result {
            Ok(_) => {
                // Record successful flush metrics for each CF
                for (cf_name, batch_size) in &cf_batch_sizes {
                    metrics::record_db_flush_duration(cf_name, elapsed_secs);
                    metrics::BATCH_FLUSH_COUNT.with_label_values(&[cf_name]).inc();
                    metrics::DB_BATCH_SIZE_ENTRIES.with_label_values(&[cf_name]).set(*batch_size as i64);
                }
                
                // Warn on slow flushes
                if elapsed_secs > 10.0 {
                    let cf_list: Vec<String> = cf_batch_sizes.keys().cloned().collect();
                    warn!(
                        cf = cf_list.join(","),
                        batch_size = pending_ops,
                        duration_secs = elapsed_secs,
                        "Slow database flush"
                    );
                } else {
                    info!(
                        cf_count = cf_batch_sizes.len(),
                        batch_size = pending_ops,
                        duration_ms = format!("{:.2}", elapsed_ms),
                        "Flush complete"
                    );
                }
                
                Ok(())
            }
            Err(db_error) => {
                // Record error metrics for each CF
                for (cf_name, batch_size) in &cf_batch_sizes {
                    error!(
                        cf = cf_name,
                        batch_size = batch_size,
                        error = db_error,
                        "Flush error"
                    );
                    metrics::increment_db_errors("flush", cf_name);
                }
                
                Err(Box::from(db_error))
            }
        }
    }

    /// Clear all pending operations without writing
    pub fn clear(&mut self) {
        self.operations.clear();
    }
}

/// Convenience function for atomic batch writes across column families
/// 
/// This is a simplified API for one-off atomic writes without needing
/// to create and manage an AtomicBatchWriter instance.
/// 
/// # Arguments
/// * `db` - RocksDB instance
/// * `operations` - List of (cf_name, key, value) tuples to write
/// 
/// # Returns
/// Ok(()) if all writes succeeded atomically, Err otherwise
pub async fn atomic_batch_write(
    db: Arc<DB>,
    operations: Vec<(String, Vec<u8>, Vec<u8>)>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if operations.is_empty() {
        return Ok(());
    }

    let db_clone = db.clone();
    tokio::task::spawn_blocking(move || {
        let mut batch = WriteBatch::default();
        
        for (cf_name, key, value) in operations {
            let cf = db_clone.cf_handle(&cf_name)
                .ok_or_else(|| format!("Column family not found: {}", cf_name))?;
            batch.put_cf(&cf, key, value);
        }

        db_clone.write(batch)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    })
    .await
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)??;

    Ok(())
}

/// Convenience function for atomic batch deletes across column families
pub async fn atomic_batch_delete(
    db: Arc<DB>,
    operations: Vec<(String, Vec<u8>)>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if operations.is_empty() {
        return Ok(());
    }

    let db_clone = db.clone();
    tokio::task::spawn_blocking(move || {
        let mut batch = WriteBatch::default();
        
        for (cf_name, key) in operations {
            let cf = db_clone.cf_handle(&cf_name)
                .ok_or_else(|| format!("Column family not found: {}", cf_name))?;
            batch.delete_cf(&cf, key);
        }

        db_clone.write(batch)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    })
    .await
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)??;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocksdb::{Options, DB};
    use tempfile::TempDir;

    fn create_test_db() -> (Arc<DB>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        
        let cf_names = vec!["cf1", "cf2", "cf3"];
        let db = DB::open_cf(&opts, temp_dir.path(), &cf_names).unwrap();
        (Arc::new(db), temp_dir)
    }

    #[tokio::test]
    async fn test_atomic_write_all_succeed() {
        let (db, _temp) = create_test_db();
        let mut writer = AtomicBatchWriter::new(db.clone(), 1000);

        // Add writes to multiple CFs
        writer.put("cf1", b"key1".to_vec(), b"value1".to_vec());
        writer.put("cf2", b"key2".to_vec(), b"value2".to_vec());
        writer.put("cf3", b"key3".to_vec(), b"value3".to_vec());

        // Flush atomically
        writer.flush().await.unwrap();

        // Verify all writes succeeded
        let cf1 = db.cf_handle("cf1").unwrap();
        let cf2 = db.cf_handle("cf2").unwrap();
        let cf3 = db.cf_handle("cf3").unwrap();

        assert_eq!(db.get_cf(&cf1, b"key1").unwrap().unwrap(), b"value1");
        assert_eq!(db.get_cf(&cf2, b"key2").unwrap().unwrap(), b"value2");
        assert_eq!(db.get_cf(&cf3, b"key3").unwrap().unwrap(), b"value3");
    }

    #[tokio::test]
    async fn test_atomic_delete() {
        let (db, _temp) = create_test_db();
        let mut writer = AtomicBatchWriter::new(db.clone(), 1000);

        // Write initial data
        writer.put("cf1", b"key1".to_vec(), b"value1".to_vec());
        writer.flush().await.unwrap();

        // Delete atomically
        writer.delete("cf1", b"key1".to_vec());
        writer.flush().await.unwrap();

        // Verify deletion
        let cf1 = db.cf_handle("cf1").unwrap();
        assert!(db.get_cf(&cf1, b"key1").unwrap().is_none());
    }

    #[tokio::test]
    async fn test_pending_count() {
        let (db, _temp) = create_test_db();
        let mut writer = AtomicBatchWriter::new(db, 1000);

        assert_eq!(writer.pending_count(), 0);

        writer.put("cf1", b"key1".to_vec(), b"value1".to_vec());
        assert_eq!(writer.pending_count(), 1);

        writer.put("cf2", b"key2".to_vec(), b"value2".to_vec());
        assert_eq!(writer.pending_count(), 2);

        writer.flush().await.unwrap();
        assert_eq!(writer.pending_count(), 0);
    }

    #[tokio::test]
    async fn test_should_flush() {
        let (db, _temp) = create_test_db();
        let mut writer = AtomicBatchWriter::new(db, 2); // Small limit

        assert!(!writer.should_flush());

        writer.put("cf1", b"key1".to_vec(), b"value1".to_vec());
        assert!(!writer.should_flush());

        writer.put("cf2", b"key2".to_vec(), b"value2".to_vec());
        assert!(writer.should_flush());
    }
}
