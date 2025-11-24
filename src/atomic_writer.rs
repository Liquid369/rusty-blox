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

        // Move operations out to process
        let operations = std::mem::take(&mut self.operations);
        let db = self.db.clone();

        // Perform atomic commit in blocking task
        tokio::task::spawn_blocking(move || {
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

            // Add all operations to the single WriteBatch
            for (cf_name, ops) in cf_operations {
                let cf = db.cf_handle(&cf_name)
                    .ok_or_else(|| format!("Column family not found: {}", cf_name))?;
                
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
            db.write(batch)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        })
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)??;

        Ok(())
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
