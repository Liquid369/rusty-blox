use std::sync::Arc;
use rocksdb::DB;
use crate::atomic_writer::AtomicBatchWriter;

/// Accumulates database writes and flushes them in batches for better performance
/// 
/// NOW WITH ATOMIC SEMANTICS: All writes across all column families are committed
/// atomically. Either all succeed together, or none do. This prevents database
/// corruption on crashes.
pub struct BatchWriter {
    atomic_writer: AtomicBatchWriter,
}

impl BatchWriter {
    pub fn new(db: Arc<DB>, batch_size_limit: usize) -> Self {
        Self {
            atomic_writer: AtomicBatchWriter::new(db, batch_size_limit),
        }
    }

    /// Add a put operation to the batch
    pub fn put(&mut self, cf_name: &str, key: Vec<u8>, value: Vec<u8>) {
        self.atomic_writer.put(cf_name, key, value);
    }

    /// Add a delete operation to the batch
    pub fn delete(&mut self, cf_name: &str, key: Vec<u8>) {
        self.atomic_writer.delete(cf_name, key);
    }

    /// Check if batch should be flushed based on size
    pub fn should_flush(&self) -> bool {
        self.atomic_writer.should_flush()
    }

    /// Flush all accumulated writes to database ATOMICALLY
    /// 
    /// CRITICAL CHANGE: This now commits ALL writes across ALL column families
    /// in a single atomic operation. Previously, each CF was written separately.
    pub async fn flush(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.atomic_writer.flush().await
    }

    /// Get total pending operations
    pub fn pending_count(&self) -> usize {
        self.atomic_writer.pending_count()
    }
}
