/// Request Caching Module
/// 
/// Provides in-memory LRU cache for frequently accessed blockchain data
/// to reduce database load and improve API response times.
/// 
/// Cached items:
/// - Blocks (by height and hash)
/// - Transactions (by txid)
/// - Address info (by address string)
/// 
/// Cache sizes are configurable but default to reasonable values based
/// on typical memory constraints (targeting ~100-200MB total cache).

use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use serde::{Serialize, Deserialize};
use serde::de::DeserializeOwned;

/// Cache entry with TTL
#[derive(Debug, Clone)]
struct CachedEntry<T> {
    value: T,
    expires_at: Instant,
}

impl<T> CachedEntry<T> {
    fn new(value: T, ttl: Duration) -> Self {
        Self {
            value,
            expires_at: Instant::now() + ttl,
        }
    }
    
    fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }
    
    fn value(&self) -> &T {
        &self.value
    }
}

/// Block data cached in memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedBlock {
    pub height: i32,
    pub hash: String,
    pub version: u32,
    pub time: u32,
    pub nonce: u32,
    pub bits: u32,
    pub tx_count: usize,
    pub size: usize,
}

/// Transaction data cached in memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedTransaction {
    pub txid: String,
    pub version: u32,
    pub height: i32,
    pub time: u32,
    pub vin_count: usize,
    pub vout_count: usize,
    pub value_out: i64,
}

/// Address info cached in memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedAddressInfo {
    pub address: String,
    pub balance: i64,
    pub total_received: i64,
    pub total_sent: i64,
    pub unconfirmed_balance: i64,
    pub tx_count: usize,
    pub unconfirmed_tx_count: usize,
}

/// Global cache manager holding all LRU caches
pub struct CacheManager {
    /// Block cache by height (most common lookup pattern)
    blocks_by_height: Arc<RwLock<LruCache<i32, CachedBlock>>>,
    
    /// Block cache by hash (for block detail endpoint)
    blocks_by_hash: Arc<RwLock<LruCache<String, CachedBlock>>>,
    
    /// Transaction cache by txid
    transactions: Arc<RwLock<LruCache<String, CachedTransaction>>>,
    
    /// Address info cache
    addresses: Arc<RwLock<LruCache<String, CachedAddressInfo>>>,
    
    /// Generic JSON cache with TTL support for API responses
    json_cache: Arc<RwLock<LruCache<String, CachedEntry<serde_json::Value>>>>,
}

impl CacheManager {
    /// Create a new cache manager with default sizes
    /// 
    /// Default sizes (approximate memory usage):
    /// - 1000 blocks by height (~500KB)
    /// - 1000 blocks by hash (~500KB)
    /// - 10000 transactions (~5MB)
    /// - 5000 addresses (~2MB)
    /// - 5000 JSON responses (~10MB)
    /// 
    /// Total: ~18MB
    pub fn new() -> Self {
        Self::with_capacities(1000, 1000, 10000, 5000, 5000)
    }
    
    /// Create cache manager with custom capacities
    pub fn with_capacities(
        blocks_by_height_cap: usize,
        blocks_by_hash_cap: usize,
        transactions_cap: usize,
        addresses_cap: usize,
        json_cap: usize,
    ) -> Self {
        Self {
            blocks_by_height: Arc::new(RwLock::new(
                LruCache::new(NonZeroUsize::new(blocks_by_height_cap).unwrap())
            )),
            blocks_by_hash: Arc::new(RwLock::new(
                LruCache::new(NonZeroUsize::new(blocks_by_hash_cap).unwrap())
            )),
            transactions: Arc::new(RwLock::new(
                LruCache::new(NonZeroUsize::new(transactions_cap).unwrap())
            )),
            addresses: Arc::new(RwLock::new(
                LruCache::new(NonZeroUsize::new(addresses_cap).unwrap())
            )),
            json_cache: Arc::new(RwLock::new(
                LruCache::new(NonZeroUsize::new(json_cap).unwrap())
            )),
        }
    }
    
    // ========== Block Cache Methods ==========
    
    /// Get block from cache by height
    pub async fn get_block_by_height(&self, height: i32) -> Option<CachedBlock> {
        let mut cache = self.blocks_by_height.write().await;
        cache.get(&height).cloned()
    }
    
    /// Get block from cache by hash
    pub async fn get_block_by_hash(&self, hash: &str) -> Option<CachedBlock> {
        let mut cache = self.blocks_by_hash.write().await;
        cache.get(hash).cloned()
    }
    
    /// Put block in both caches (by height and hash)
    pub async fn put_block(&self, block: CachedBlock) {
        let height = block.height;
        let hash = block.hash.clone();
        
        {
            let mut cache = self.blocks_by_height.write().await;
            cache.put(height, block.clone());
        }
        
        {
            let mut cache = self.blocks_by_hash.write().await;
            cache.put(hash, block);
        }
    }
    
    /// Invalidate block from both caches (used during reorgs)
    pub async fn invalidate_block(&self, height: i32, hash: &str) {
        {
            let mut cache = self.blocks_by_height.write().await;
            cache.pop(&height);
        }
        
        {
            let mut cache = self.blocks_by_hash.write().await;
            cache.pop(&hash.to_string());
        }
    }
    
    // ========== Transaction Cache Methods ==========
    
    /// Get transaction from cache by txid
    pub async fn get_transaction(&self, txid: &str) -> Option<CachedTransaction> {
        let mut cache = self.transactions.write().await;
        cache.get(txid).cloned()
    }
    
    /// Put transaction in cache
    pub async fn put_transaction(&self, tx: CachedTransaction) {
        let txid = tx.txid.clone();
        let mut cache = self.transactions.write().await;
        cache.put(txid, tx);
    }
    
    /// Invalidate transaction from cache (used during reorgs)
    pub async fn invalidate_transaction(&self, txid: &str) {
        let mut cache = self.transactions.write().await;
        cache.pop(&txid.to_string());
    }
    
    // ========== Address Cache Methods ==========
    
    /// Get address info from cache
    pub async fn get_address(&self, address: &str) -> Option<CachedAddressInfo> {
        let mut cache = self.addresses.write().await;
        cache.get(address).cloned()
    }
    
    /// Put address info in cache
    pub async fn put_address(&self, info: CachedAddressInfo) {
        let address = info.address.clone();
        let mut cache = self.addresses.write().await;
        cache.put(address, info);
    }
    
    /// Invalidate address from cache (when new tx arrives)
    pub async fn invalidate_address(&self, address: &str) {
        let mut cache = self.addresses.write().await;
        cache.pop(&address.to_string());
    }
    
    /// Invalidate multiple addresses (used when block arrives)
    pub async fn invalidate_addresses(&self, addresses: &[String]) {
        let mut cache = self.addresses.write().await;
        for address in addresses {
            cache.pop(address);
        }
    }
    
    // ========== Generic JSON Cache with TTL ==========
    
    /// Get JSON value from cache (with TTL check)
    pub async fn get_json<T>(&self, key: &str) -> Option<T>
    where
        T: DeserializeOwned,
    {
        let mut cache = self.json_cache.write().await;
        if let Some(entry) = cache.get(&key.to_string()) {
            if !entry.is_expired() {
                // Try to deserialize from JSON
                if let Ok(value) = serde_json::from_value::<T>(entry.value().clone()) {
                    return Some(value);
                }
            } else {
                // Expired, remove it
                cache.pop(&key.to_string());
            }
        }
        None
    }
    
    /// Set JSON value in cache with TTL
    pub async fn set_json<T>(&self, key: &str, value: &T, ttl: Duration)
    where
        T: Serialize,
    {
        if let Ok(json_value) = serde_json::to_value(value) {
            let entry = CachedEntry::new(json_value, ttl);
            let mut cache = self.json_cache.write().await;
            cache.put(key.to_string(), entry);
        }
    }
    
    /// Get raw serde_json::Value from cache (with TTL check)
    pub async fn get_json_value(&self, key: &str) -> Option<serde_json::Value> {
        let mut cache = self.json_cache.write().await;
        if let Some(entry) = cache.get(&key.to_string()) {
            if !entry.is_expired() {
                return Some(entry.value().clone());
            } else {
                // Expired, remove it
                cache.pop(&key.to_string());
            }
        }
        None
    }
    
    /// Set raw serde_json::Value in cache with TTL
    pub async fn set_json_value(&self, key: &str, value: serde_json::Value, ttl: Duration) {
        let entry = CachedEntry::new(value, ttl);
        let mut cache = self.json_cache.write().await;
        cache.put(key.to_string(), entry);
    }
    
    /// Invalidate specific cache key
    pub async fn invalidate(&self, key: &str) {
        let mut cache = self.json_cache.write().await;
        cache.pop(&key.to_string());
    }
    
    /// Get or compute value with caching
    pub async fn get_or_compute<F, Fut, T, E>(
        &self,
        key: &str,
        ttl: Duration,
        compute: F,
    ) -> Result<T, E>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
        T: Clone + Serialize + DeserializeOwned,
    {
        // Try cache first
        if let Some(cached) = self.get_json::<T>(key).await {
            return Ok(cached);
        }
        
        // Compute value
        let value = compute().await?;
        
        // Store in cache
        self.set_json(key, &value, ttl).await;
        
        Ok(value)
    }
    
    // ========== Cache Statistics ==========
    
    /// Get cache statistics for monitoring
    pub async fn get_stats(&self) -> CacheStats {
        let blocks_height_len = self.blocks_by_height.read().await.len();
        let blocks_hash_len = self.blocks_by_hash.read().await.len();
        let transactions_len = self.transactions.read().await.len();
        let addresses_len = self.addresses.read().await.len();
        let json_len = self.json_cache.read().await.len();
        
        CacheStats {
            blocks_by_height_count: blocks_height_len,
            blocks_by_hash_count: blocks_hash_len,
            transactions_count: transactions_len,
            addresses_count: addresses_len,
            json_cache_count: json_len,
        }
    }
    
    /// Clear all caches (for testing or after major reorg)
    pub async fn clear_all(&self) {
        self.blocks_by_height.write().await.clear();
        self.blocks_by_hash.write().await.clear();
        self.transactions.write().await.clear();
        self.addresses.write().await.clear();
        self.json_cache.write().await.clear();
    }
}

/// Cache statistics for monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub blocks_by_height_count: usize,
    pub blocks_by_hash_count: usize,
    pub transactions_count: usize,
    pub addresses_count: usize,
    pub json_cache_count: usize,
}

impl Default for CacheManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_block_cache() {
        let cache = CacheManager::new();
        
        let block = CachedBlock {
            height: 100,
            hash: "abc123".to_string(),
            version: 1,
            time: 1234567890,
            nonce: 0,
            bits: 0,
            tx_count: 5,
            size: 1000,
        };
        
        // Initially not in cache
        assert!(cache.get_block_by_height(100).await.is_none());
        assert!(cache.get_block_by_hash("abc123").await.is_none());
        
        // Put in cache
        cache.put_block(block.clone()).await;
        
        // Now available via both lookups
        assert!(cache.get_block_by_height(100).await.is_some());
        assert!(cache.get_block_by_hash("abc123").await.is_some());
        
        // Invalidate
        cache.invalidate_block(100, "abc123").await;
        assert!(cache.get_block_by_height(100).await.is_none());
        assert!(cache.get_block_by_hash("abc123").await.is_none());
    }
    
    #[tokio::test]
    async fn test_transaction_cache() {
        let cache = CacheManager::new();
        
        let tx = CachedTransaction {
            txid: "tx123".to_string(),
            version: 2,
            height: 100,
            time: 1234567890,
            vin_count: 2,
            vout_count: 2,
            value_out: 100000000,
        };
        
        assert!(cache.get_transaction("tx123").await.is_none());
        cache.put_transaction(tx.clone()).await;
        assert!(cache.get_transaction("tx123").await.is_some());
    }
    
    #[tokio::test]
    async fn test_address_cache() {
        let cache = CacheManager::new();
        
        let info = CachedAddressInfo {
            address: "D12345".to_string(),
            balance: 100000000,
            total_received: 200000000,
            total_sent: 100000000,
            unconfirmed_balance: 0,
            tx_count: 10,
            unconfirmed_tx_count: 0,
        };
        
        assert!(cache.get_address("D12345").await.is_none());
        cache.put_address(info.clone()).await;
        assert!(cache.get_address("D12345").await.is_some());
    }
    
    #[tokio::test]
    async fn test_cache_stats() {
        let cache = CacheManager::with_capacities(100, 100, 100, 100, 100);
        
        // Add some items
        let block = CachedBlock {
            height: 1,
            hash: "h1".to_string(),
            version: 1,
            time: 0,
            nonce: 0,
            bits: 0,
            tx_count: 0,
            size: 0,
        };
        cache.put_block(block).await;
        
        let tx = CachedTransaction {
            txid: "tx1".to_string(),
            version: 1,
            height: 1,
            time: 0,
            vin_count: 1,
            vout_count: 1,
            value_out: 0,
        };
        cache.put_transaction(tx).await;
        
        let stats = cache.get_stats().await;
        assert_eq!(stats.blocks_by_height_count, 1);
        assert_eq!(stats.blocks_by_hash_count, 1);
        assert_eq!(stats.transactions_count, 1);
        assert_eq!(stats.addresses_count, 0);
    }
}
