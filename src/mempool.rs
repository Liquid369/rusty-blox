/// Mempool Service
/// 
/// Monitors unconfirmed transactions:
/// - Polls RPC getrawmempool
/// - Tracks pending transactions
/// - Provides fee estimates
/// - Notifies of new transactions

use std::sync::Arc;
use std::time::Duration;
use std::collections::HashMap;
use tokio::sync::RwLock;
use pivx_rpc_rs::PivxRpcClient;
use serde::{Serialize, Deserialize};
use tracing::{error, warn};

use crate::config::get_global_config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MempoolTransaction {
    pub txid: String,
    pub size: Option<usize>,
    pub fee: Option<f64>,
    pub time: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MempoolInfo {
    pub size: usize,
    pub bytes: usize,
    pub usage: Option<usize>,
    pub transactions: Vec<MempoolTransaction>,
}

/// Shared mempool state
pub struct MempoolState {
    pub transactions: RwLock<HashMap<String, MempoolTransaction>>,
}

impl Default for MempoolState {
    fn default() -> Self {
        Self::new()
    }
}

impl MempoolState {
    pub fn new() -> Self {
        Self {
            transactions: RwLock::new(HashMap::new()),
        }
    }
    
    pub async fn get_info(&self) -> MempoolInfo {
        let txs = self.transactions.read().await;
        
        MempoolInfo {
            size: txs.len(),
            bytes: txs.values().map(|tx| tx.size.unwrap_or(0)).sum(),
            usage: None,
            transactions: txs.values().cloned().collect(),
        }
    }
    
    pub async fn get_transaction(&self, txid: &str) -> Option<MempoolTransaction> {
        let txs = self.transactions.read().await;
        txs.get(txid).cloned()
    }
}

/// Monitor mempool for new transactions
pub async fn run_mempool_monitor(
    mempool_state: Arc<MempoolState>,
    poll_interval_secs: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    
    // Initialize RPC client
    let config = get_global_config();
    let rpc_host = config.get_string("rpc.host")?;
    let rpc_user = config.get_string("rpc.user")?;
    let rpc_pass = config.get_string("rpc.pass")?;
    
    // Create RPC client in a completely separate OS thread
    // PivxRpcClient uses reqwest::blocking which creates its own runtime
    let (tx, rx) = std::sync::mpsc::channel();
    let rpc_host_clone = rpc_host.clone();
    
    std::thread::spawn(move || {
        let client = PivxRpcClient::new(
            rpc_host_clone,
            Some(rpc_user),
            Some(rpc_pass),
            3,
            10,
            1000,
        );
        
        // Test connection
        let result = match client.getblockcount() {
            Ok(_) => Ok(Arc::new(client)),
            Err(e) => Err(e),
        };
        let _ = tx.send(result);
    });
    
    let rpc_client = match rx.recv_timeout(std::time::Duration::from_secs(10)) {
        Ok(Ok(client)) => client,
        Ok(Err(e)) => {
            error!(error = ?e, "Mempool RPC connection failed");
            return Ok(());
        }
        Err(_) => {
            error!("Mempool RPC connection timed out");
            return Ok(());
        }
    };
    
    loop {
        tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;
        
        // Get raw mempool - must use separate thread
        let client = Arc::clone(&rpc_client);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = client.getrawmempool(false);
            let _ = tx.send(result);
        });
        
        let mempool_result = match rx.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => {
                error!(error = ?e, "Failed to get mempool");
                continue;
            }
            Err(_) => {
                error!("Mempool RPC call timed out");
                continue;
            }
        };
        
        // Extract txids based on RawMemPool variant
        let txids: Vec<String> = match mempool_result {
            pivx_rpc_rs::RawMemPool::TxIds(txid_list) => txid_list,
            pivx_rpc_rs::RawMemPool::Verbose(_) => {
                warn!("Unexpected verbose mempool response");
                continue;
            }
        };
        
        let mut txs = mempool_state.transactions.write().await;
        
        // Remove confirmed transactions (keep only those still in mempool)
        let current_txids: std::collections::HashSet<String> = txids.iter().cloned().collect();
        txs.retain(|txid, _| current_txids.contains(txid));
        
        // Add new transactions
        for txid in txids {
            if !txs.contains_key(&txid) {
                txs.insert(txid.clone(), MempoolTransaction {
                    txid: txid.clone(),
                    size: None,
                    fee: None,
                    time: Some(std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or(std::time::Duration::from_secs(0))
                        .as_secs()),
                });
            }
        }
    }
}
