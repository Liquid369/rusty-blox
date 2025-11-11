use std::sync::Arc;
use std::collections::HashMap;
use rocksdb::DB;
use crate::db_utils::batch_put_cf;

/// Accumulates database writes and flushes them in batches for better performance
pub struct BatchWriter {
    db: Arc<DB>,
    batches: HashMap<String, Vec<(Vec<u8>, Vec<u8>)>>,
    batch_size_limit: usize,
    total_operations: usize,
}

impl BatchWriter {
    pub fn new(db: Arc<DB>, batch_size_limit: usize) -> Self {
        Self {
            db,
            batches: HashMap::new(),
            batch_size_limit,
            total_operations: 0,
        }
    }

    /// Add a put operation to the batch
    pub fn put(&mut self, cf_name: &str, key: Vec<u8>, value: Vec<u8>) {
        self.batches
            .entry(cf_name.to_string())
            .or_insert_with(Vec::new)
            .push((key, value));
        
        self.total_operations += 1;
    }

    /// Check if batch should be flushed based on size
    pub fn should_flush(&self) -> bool {
        self.total_operations >= self.batch_size_limit
    }

    /// Flush all accumulated writes to database
    pub async fn flush(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if self.total_operations == 0 {
            return Ok(());
        }

        // Process each column family's batch
        for (cf_name, items) in self.batches.drain() {
            if !items.is_empty() {
                batch_put_cf(self.db.clone(), &cf_name, items).await?;
            }
        }

        self.total_operations = 0;
        Ok(())
    }

    /// Get total pending operations
    pub fn pending_count(&self) -> usize {
        self.total_operations
    }
}
