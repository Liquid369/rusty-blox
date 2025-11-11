use serde::Serialize;
use serde::Deserialize;
use std::fmt;
use std::hash::Hash as StdHash;
use std::sync::Arc;
use rocksdb::DB;

/// Production-ready error type with context
#[derive(Debug, Clone)]
pub struct MyError {
    pub message: String,
}

impl MyError {
    /// Create a new error
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for MyError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for MyError {}

pub struct Hash(pub [u8; 32]);

impl fmt::LowerHex for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0.iter().rev() {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Byte33(pub [u8; 33]);

impl std::borrow::Borrow<Byte33> for [u8; 33] {
    fn borrow(&self) -> &Byte33 {
        unsafe { &*(self.as_ptr() as *const Byte33) }
    }
}

#[derive(Clone)]
pub enum AddressType {
    CoinStakeTx,
    CoinBaseTx,
    Nonstandard,
    P2PKH(String),
    P2PK(String),
    P2SH(String),
    ZerocoinMint,
    ZerocoinSpend,
    ZerocoinPublicSpend,
    Staking(String, String),
    Sapling,
}

pub struct CBlockHeader {
    pub n_version: u32,
    pub block_hash: [u8; 32],
    pub block_height: Option<i32>,
    pub hash_prev_block: [u8; 32],
    pub hash_merkle_root: [u8; 32],
    pub n_time: u32,
    pub n_bits: u32,
    pub n_nonce: u32,
    pub n_accumulator_checkpoint: Option<[u8; 32]>,
    pub hash_final_sapling_root: Option<[u8; 32]>,
}

impl std::fmt::Debug for CBlockHeader {
    // Formatting for CBlockHeader
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Block Header {{")?;
        if let Some(block_height) = &self.block_height {
            writeln!(f, "Block Height: {}", block_height)?;
        } else {
            writeln!(f, "Block Height: None")?;
        }
        writeln!(f, "Block Version: {}", self.n_version)?;
        writeln!(f, "Previous Block Hash: {:x}", Hash(self.hash_prev_block))?;
        writeln!(f, "Merkle Root: {:x}", Hash(self.hash_merkle_root))?;
        writeln!(f, "Block Time: {}", self.n_time)?;
        writeln!(f, "Block Bits: {:x}", self.n_bits)?;
        writeln!(f, "Block Nonce: {:?}", self.n_nonce)?;
        if let Some(accumulator_checkpoint) = &self.n_accumulator_checkpoint {
            writeln!(
                f,
                "Accumulator Checkpoint: {:?}",
                hex::encode(accumulator_checkpoint)
            )?;
        } else {
            writeln!(f, "Accumulator Checkpoint: None")?;
        }

        if let Some(final_sapling_root) = &self.hash_final_sapling_root {
            writeln!(f, "Final Sapling Root: {:x}", Hash(*final_sapling_root))?;
        } else {
            writeln!(f, "Final Sapling Root: None")?;
        }
        write!(f, "}}")
    }
}

#[derive(Clone)]
pub struct CTransaction {
    pub txid: String,
    pub version: i16,
    pub inputs: Vec<CTxIn>,
    pub outputs: Vec<CTxOut>,
    pub lock_time: u32,
}

impl std::fmt::Debug for CTransaction {
    // Formatting for CTransaction
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Transaction {{")?;
        writeln!(f, "    txid: {}", self.txid)?;
        writeln!(f, "    version: {}", self.version)?;
        writeln!(f, "    inputs: {:?}", self.inputs)?;
        writeln!(f, "    outputs: {:?}", self.outputs)?;
        writeln!(f, "    lock_time: {}", self.lock_time)?;
        write!(f, "}}")
    }
}

#[derive(Clone)]
pub struct CTxIn {
    pub prevout: Option<COutPoint>,
    pub script_sig: CScript,
    pub sequence: u32,
    pub index: u64,
    pub coinbase: Option<Vec<u8>>,
}

impl std::fmt::Debug for CTxIn {
    // Formatting for CTxIn
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{{")?;
        writeln!(f, "    prevout: {:?}", self.prevout)?;
        writeln!(f, "    script_sig: {:?}", self.script_sig)?;
        writeln!(f, "    sequence: {}", self.sequence)?;
        writeln!(f, "    coinbase: {:?}", self.coinbase)?;
        write!(f, "}}")
    }
}

#[derive(Clone)]
pub struct CTxOut {
    pub value: i64,
    pub script_length: i32,
    pub script_pubkey: CScript,
    pub index: u64,
    pub address: Vec<String>,
}

impl fmt::Debug for CTxOut {
    // Formatting for CTxOut
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{{")?;
        writeln!(f, "    value: {:?}", self.value)?;
        writeln!(f, "    script_pubkey: {:?}", self.script_pubkey)?;
        writeln!(f, "    script_length: {:?}", self.script_length)?;
        writeln!(f, "    address: {:?}", self.address)?;
        writeln!(f, "}}")
    }
}

#[derive(Clone, Debug, Default)]
pub struct COutPoint {
    pub hash: String,
    pub n: u32,
}

#[derive(Clone)]
pub struct CScript {
    pub script: Vec<u8>,
}

impl std::fmt::Debug for CScript {
    // Formatting for CScript
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", hex::encode(&self.script))
    }
}

#[derive(Serialize)]
pub struct SaplingTxData {
    pub value: i64,
    pub vshield_spend: Vec<VShieldSpend>,
    pub vshield_output: Vec<VShieldOutput>,
    pub binding_sig: Vec<u8>,
}

impl std::fmt::Debug for SaplingTxData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Formatting SaplingTxData
        writeln!(f, "SaplingTxData {{")?;
        writeln!(f, "    value: {}", self.value)?;
        writeln!(f, "    vshield_spend: {:?}", self.vshield_spend)?;
        writeln!(f, "    vshield_output: {:?}", self.vshield_output)?;
        writeln!(f, "    binding_sig: {:?}", hex::encode(&self.binding_sig))?;
        write!(f, "}}")
    }
}

#[derive(Serialize)]
pub struct VShieldSpend {
    pub cv: Vec<u8>,
    pub anchor: Vec<u8>,
    pub nullifier: Vec<u8>,
    pub rk: Vec<u8>,
    pub proof: Vec<u8>,
    pub spend_auth_sig: Vec<u8>,
}

impl fmt::Debug for VShieldSpend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Formatting VShieldSpend
        writeln!(f, "{{")?;
        writeln!(f, "    cv: {:?}", hex::encode(&self.cv))?;
        writeln!(f, "    anchor: {:?}", hex::encode(&self.anchor))?;
        writeln!(f, "    nullifier: {:?}", hex::encode(&self.nullifier))?;
        writeln!(f, "    rk: {:?}", hex::encode(&self.rk))?;
        writeln!(f, "    proof: {:?}", hex::encode(&self.proof))?;
        writeln!(
            f,
            "    spend_auth_sig: {:?}",
            hex::encode(&self.spend_auth_sig)
        )?;
        write!(f, "}}")
    }
}

#[derive(Serialize)]
pub struct VShieldOutput {
    pub cv: Vec<u8>,
    pub cmu: Vec<u8>,
    pub ephemeral_key: Vec<u8>,
    pub enc_ciphertext: Vec<u8>,
    pub out_ciphertext: Vec<u8>,
    pub proof: Vec<u8>,
}

impl std::fmt::Debug for VShieldOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Formatting VShieldOutput
        writeln!(f, "{{")?;
        writeln!(f, "    cv: {:?}", hex::encode(&self.cv))?;
        writeln!(f, "    cmu: {:?}", hex::encode(&self.cmu))?;
        writeln!(
            f,
            "    ephemeral_key: {:?}",
            hex::encode(&self.ephemeral_key)
        )?;
        writeln!(
            f,
            "    enc_ciphertext: {:?}",
            hex::encode(&self.enc_ciphertext)
        )?;
        writeln!(
            f,
            "    out_ciphertext: {:?}",
            hex::encode(&self.out_ciphertext)
        )?;
        writeln!(f, "    proof: {:?}", hex::encode(&self.proof))?;
        write!(f, "}}")
    }
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<DB>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub hash: String,
    pub height: u32,
    pub version: u32,
    pub merkleroot: String,
    pub time: u32,
    pub nonce: u32,
    pub bits: String,
    pub difficulty: f64,
    pub tx: Vec<String>,
    pub previousblockhash: Option<String>,
}
