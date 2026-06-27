use crate::atomic_writer::AtomicBatchWriter;
use rocksdb::DB;
use std::sync::Arc;

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

    /// Construct a batch writer for a given durability mode.
    ///
    /// Pass `bulk = true` only on the initial full-reindex path (disables the
    /// WAL — the DB is reconstructible from the `.blk` files). Pass `false` on
    /// the live/RPC catch-up path so writes stay WAL-recoverable. Bytes written
    /// are identical regardless.
    pub fn new_with_bulk(db: Arc<DB>, batch_size_limit: usize, bulk: bool) -> Self {
        let mut atomic_writer = AtomicBatchWriter::new(db, batch_size_limit);
        atomic_writer.set_disable_wal(bulk);
        Self { atomic_writer }
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
