/// Spent UTXO Tracking Module
/// 
/// Implements tracking of spent transaction outputs to enable:
/// 1. UTXO resurrection during blockchain reorganizations
/// 2. Input value calculation without RPC calls
/// 3. Transaction fee calculation
/// 4. Complete transaction history
/// 
/// PIVX CORE EQUIVALENT: CCoinsViewCache with undo data
/// 
/// This module follows PIVX Core's approach of storing "undo" data for each block,
/// allowing spent UTXOs to be restored during reorgs.

use std::sync::Arc;
use rocksdb::DB;
use serde::{Deserialize, Serialize};

/// Represents a spent UTXO with all information needed for resurrection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpentUtxo {
    /// Transaction ID that created this output (internal format - reversed bytes)
    pub txid: Vec<u8>,
    
    /// Output index (vout)
    pub vout: u64,
    
    /// Output value in satoshis
    pub value: i64,
    
    /// ScriptPubKey bytes
    pub script_pubkey: Vec<u8>,
    
    /// Block height where this output was created
    pub created_height: i32,
    
    /// Block height where this output was spent
    pub spent_height: i32,
    
    /// Transaction ID that spent this output (internal format)
    pub spending_txid: Vec<u8>,
    
    /// Input index in spending transaction
    pub spending_vin: u32,
}

impl SpentUtxo {
    /// Create new SpentUtxo record
    pub fn new(
        txid: Vec<u8>,
        vout: u64,
        value: i64,
        script_pubkey: Vec<u8>,
        created_height: i32,
        spent_height: i32,
        spending_txid: Vec<u8>,
        spending_vin: u32,
    ) -> Self {
        Self {
            txid,
            vout,
            value,
            script_pubkey,
            created_height,
            spent_height,
            spending_txid,
            spending_vin,
        }
    }
    
    /// Serialize to bytes for database storage
    pub fn to_bytes(&self) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(bincode::serialize(self)?)
    }
    
    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(bincode::deserialize(bytes)?)
    }
    
    /// Get UTXO key format ('u' + txid_internal + vout)
    pub fn get_utxo_key(&self) -> Vec<u8> {
        let mut key = vec![b'u'];
        key.extend_from_slice(&self.txid);
        key.extend_from_slice(&self.vout.to_le_bytes());
        key
    }
}

/// Block-level undo data for reorg handling
/// 
/// This struct aggregates all UTXOs spent in a block, allowing
/// complete rollback during reorganizations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockUndoData {
    /// Block height
    pub height: i32,
    
    /// Block hash (internal format)
    pub block_hash: Vec<u8>,
    
    /// All UTXOs spent in this block
    pub spent_utxos: Vec<SpentUtxo>,
}

impl BlockUndoData {
    /// Create new block undo data
    pub fn new(height: i32, block_hash: Vec<u8>) -> Self {
        Self {
            height,
            block_hash,
            spent_utxos: Vec::new(),
        }
    }
    
    /// Add spent UTXO to block undo data
    pub fn add_spent_utxo(&mut self, utxo: SpentUtxo) {
        self.spent_utxos.push(utxo);
    }
    
    /// Serialize to bytes
    pub fn to_bytes(&self) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(bincode::serialize(self)?)
    }
    
    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(bincode::deserialize(bytes)?)
    }
    
    /// Get storage key for block undo data ('U' + height as 4-byte LE)
    pub fn get_storage_key(height: i32) -> Vec<u8> {
        let mut key = vec![b'U'];
        key.extend_from_slice(&height.to_le_bytes());
        key
    }
}

/// Store a spent UTXO in the utxo_undo column family
/// 
/// This should be called BEFORE deleting a UTXO from the utxo CF.
/// The spent UTXO data allows resurrection during reorgs.
/// 
/// # Arguments
/// * `db` - RocksDB instance
/// * `utxo` - Spent UTXO to store
/// 
/// # Storage Format
/// Key: 'S' + txid_internal + vout (8 bytes)
/// Value: Serialized SpentUtxo
pub async fn store_spent_utxo(
    db: Arc<DB>,
    utxo: SpentUtxo,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Key: 'S' + txid + vout
    let mut key = vec![b'S'];
    key.extend_from_slice(&utxo.txid);
    key.extend_from_slice(&utxo.vout.to_le_bytes());
    
    let value = utxo.to_bytes()?;
    
    let db_clone = db.clone();
    tokio::task::spawn_blocking(move || {
        let cf_undo = db_clone.cf_handle("utxo_undo")
            .ok_or("utxo_undo column family not found")?;
        
        db_clone.put_cf(&cf_undo, &key, &value)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    })
    .await??;
    
    Ok(())
}

/// Retrieve a spent UTXO from the utxo_undo column family
/// 
/// Used during reorgs to resurrect previously spent outputs.
pub async fn get_spent_utxo(
    db: Arc<DB>,
    txid: &[u8],
    vout: u64,
) -> Result<Option<SpentUtxo>, Box<dyn std::error::Error + Send + Sync>> {
    let mut key = vec![b'S'];
    key.extend_from_slice(txid);
    key.extend_from_slice(&vout.to_le_bytes());
    
    let db_clone = db.clone();
    let key_clone = key.clone();
    let result = tokio::task::spawn_blocking(move || {
        let cf_undo = db_clone.cf_handle("utxo_undo")
            .ok_or("utxo_undo column family not found")?;
        
        db_clone.get_cf(&cf_undo, &key_clone)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    })
    .await??;
    
    match result {
        Some(bytes) => Ok(Some(SpentUtxo::from_bytes(&bytes)?)),
        None => Ok(None),
    }
}

/// Store block-level undo data
/// 
/// This aggregates all spent UTXOs in a block for efficient reorg handling.
/// Should be called at the end of block processing.
pub async fn store_block_undo_data(
    db: Arc<DB>,
    undo_data: BlockUndoData,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let key = BlockUndoData::get_storage_key(undo_data.height);
    let value = undo_data.to_bytes()?;
    
    let db_clone = db.clone();
    tokio::task::spawn_blocking(move || {
        let cf_undo = db_clone.cf_handle("utxo_undo")
            .ok_or("utxo_undo column family not found")?;
        
        db_clone.put_cf(&cf_undo, &key, &value)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    })
    .await??;
    
    Ok(())
}

/// Retrieve block-level undo data
/// 
/// Used during reorgs to get all UTXOs that need to be restored for a block.
pub async fn get_block_undo_data(
    db: Arc<DB>,
    height: i32,
) -> Result<Option<BlockUndoData>, Box<dyn std::error::Error + Send + Sync>> {
    let key = BlockUndoData::get_storage_key(height);
    
    let db_clone = db.clone();
    let result = tokio::task::spawn_blocking(move || {
        let cf_undo = db_clone.cf_handle("utxo_undo")
            .ok_or("utxo_undo column family not found")?;
        
        db_clone.get_cf(&cf_undo, &key)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    })
    .await??;
    
    match result {
        Some(bytes) => Ok(Some(BlockUndoData::from_bytes(&bytes)?)),
        None => Ok(None),
    }
}

/// Delete block undo data (after successful reorg or when pruning old data)
pub async fn delete_block_undo_data(
    db: Arc<DB>,
    height: i32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let key = BlockUndoData::get_storage_key(height);
    
    let db_clone = db.clone();
    tokio::task::spawn_blocking(move || {
        let cf_undo = db_clone.cf_handle("utxo_undo")
            .ok_or("utxo_undo column family not found")?;
        
        db_clone.delete_cf(&cf_undo, &key)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    })
    .await??;
    
    Ok(())
}

/// Calculate total input value for a transaction
/// 
/// Uses spent UTXO data to calculate input values without RPC calls.
/// This is much faster than querying PIVX Core for each input.
/// 
/// # Arguments
/// * `db` - RocksDB instance
/// * `inputs` - List of (txid_internal, vout) tuples
/// 
/// # Returns
/// Total input value in satoshis, or None if any input not found
pub async fn calculate_input_value(
    db: Arc<DB>,
    inputs: &[(Vec<u8>, u64)],
) -> Result<Option<i64>, Box<dyn std::error::Error + Send + Sync>> {
    let mut total_value = 0i64;
    
    for (txid, vout) in inputs {
        // First check if UTXO still exists (unspent)
        let mut utxo_key = vec![b'u'];
        utxo_key.extend_from_slice(txid);
        utxo_key.extend_from_slice(&vout.to_le_bytes());
        
        let db_clone = db.clone();
        let key_clone = utxo_key.clone();
        let utxo_exists = tokio::task::spawn_blocking(move || {
            let cf_utxo = db_clone.cf_handle("utxo")
                .ok_or("utxo column family not found")?;
            
            db_clone.get_cf(&cf_utxo, &key_clone)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        })
        .await??;
        
        if utxo_exists.is_some() {
            // UTXO still unspent - this shouldn't happen in normal flow
            // Would need to parse UTXO data to get value
            // For now, return None to indicate we need RPC fallback
            return Ok(None);
        }
        
        // Check spent UTXO tracking
        if let Some(spent_utxo) = get_spent_utxo(db.clone(), txid, *vout).await? {
            total_value += spent_utxo.value;
        } else {
            // Input not found - this is an error condition
            return Ok(None);
        }
    }
    
    Ok(Some(total_value))
}

/// Resurrect spent UTXOs for a block during reorg
/// 
/// This is called during rollback to restore UTXOs that were spent
/// in orphaned blocks back to the UTXO set.
/// 
/// # Arguments
/// * `db` - RocksDB instance
/// * `height` - Block height to resurrect UTXOs for
/// 
/// # Returns
/// Number of UTXOs resurrected
pub async fn resurrect_block_utxos(
    db: Arc<DB>,
    height: i32,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    // Get block undo data
    let undo_data = match get_block_undo_data(db.clone(), height).await? {
        Some(data) => data,
        None => {
            // No undo data for this block - this is expected for blocks
            // indexed before spent UTXO tracking was enabled
            return Ok(0);
        }
    };
    
    let count = undo_data.spent_utxos.len();
    
    // Restore each spent UTXO
    for spent_utxo in &undo_data.spent_utxos {
        let utxo_key = spent_utxo.get_utxo_key();
        
        // Recreate UTXO entry
        // Format: same as current UTXO storage (serialized vec of (txid, vout))
        let utxo_data = vec![(spent_utxo.txid.clone(), spent_utxo.vout)];
        let serialized = bincode::serialize(&utxo_data)?;
        
        let db_clone = db.clone();
        let key_clone = utxo_key.clone();
        tokio::task::spawn_blocking(move || {
            let cf_utxo = db_clone.cf_handle("utxo")
                .ok_or("utxo column family not found")?;
            
            db_clone.put_cf(&cf_utxo, &key_clone, &serialized)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        })
        .await??;
    }
    
    // Delete undo data after successful resurrection
    delete_block_undo_data(db.clone(), height).await?;
    
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_spent_utxo_creation() {
        let utxo = SpentUtxo::new(
            vec![1, 2, 3, 4],
            0,
            1000000,
            vec![0x76, 0xa9, 0x14],
            100,
            200,
            vec![5, 6, 7, 8],
            0,
        );
        
        assert_eq!(utxo.txid, vec![1, 2, 3, 4]);
        assert_eq!(utxo.vout, 0);
        assert_eq!(utxo.value, 1000000);
        assert_eq!(utxo.created_height, 100);
        assert_eq!(utxo.spent_height, 200);
    }
    
    #[test]
    fn test_spent_utxo_serialization() {
        let utxo = SpentUtxo::new(
            vec![1, 2, 3, 4],
            0,
            1000000,
            vec![0x76, 0xa9, 0x14],
            100,
            200,
            vec![5, 6, 7, 8],
            0,
        );
        
        let bytes = utxo.to_bytes().unwrap();
        let deserialized = SpentUtxo::from_bytes(&bytes).unwrap();
        
        assert_eq!(deserialized.txid, utxo.txid);
        assert_eq!(deserialized.value, utxo.value);
        assert_eq!(deserialized.spent_height, utxo.spent_height);
    }
    
    #[test]
    fn test_block_undo_data() {
        let mut undo_data = BlockUndoData::new(100, vec![0xaa, 0xbb]);
        
        let utxo1 = SpentUtxo::new(
            vec![1, 2, 3, 4],
            0,
            1000000,
            vec![0x76],
            99,
            100,
            vec![5, 6, 7, 8],
            0,
        );
        
        let utxo2 = SpentUtxo::new(
            vec![9, 10, 11, 12],
            1,
            2000000,
            vec![0xa9],
            98,
            100,
            vec![13, 14, 15, 16],
            1,
        );
        
        undo_data.add_spent_utxo(utxo1);
        undo_data.add_spent_utxo(utxo2);
        
        assert_eq!(undo_data.spent_utxos.len(), 2);
        assert_eq!(undo_data.height, 100);
    }
    
    #[test]
    fn test_block_undo_serialization() {
        let mut undo_data = BlockUndoData::new(100, vec![0xaa, 0xbb]);
        
        let utxo = SpentUtxo::new(
            vec![1, 2, 3, 4],
            0,
            1000000,
            vec![0x76],
            99,
            100,
            vec![5, 6, 7, 8],
            0,
        );
        
        undo_data.add_spent_utxo(utxo);
        
        let bytes = undo_data.to_bytes().unwrap();
        let deserialized = BlockUndoData::from_bytes(&bytes).unwrap();
        
        assert_eq!(deserialized.height, 100);
        assert_eq!(deserialized.spent_utxos.len(), 1);
        assert_eq!(deserialized.spent_utxos[0].value, 1000000);
    }
    
    #[test]
    fn test_storage_key_format() {
        let key = BlockUndoData::get_storage_key(100);
        assert_eq!(key[0], b'U');
        assert_eq!(key.len(), 5); // 'U' + 4 bytes for i32
    }
}
