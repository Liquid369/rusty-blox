use serde::Serialize;
use serde::Deserialize;
use serde::{Serializer, Deserializer};
use std::fmt;
use std::sync::Arc;
use rocksdb::DB;
use crate::cache::CacheManager;

// Helper functions to serialize large byte arrays
fn serialize_bytes<S, const N: usize>(bytes: &[u8; N], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_bytes(bytes)
}

#[allow(dead_code)] // Deserialization helper - paired with serialize_bytes for completeness
fn deserialize_bytes<'de, D, const N: usize>(deserializer: D) -> Result<[u8; N], D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    let bytes: &[u8] = Deserialize::deserialize(deserializer)?;
    if bytes.len() != N {
        return Err(Error::custom(format!("expected {} bytes, got {}", N, bytes.len())));
    }
    let mut array = [0u8; N];
    array.copy_from_slice(bytes);
    Ok(array)
}

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

/// PIVX Core-compatible script classification
/// Used for correct transaction attribution matching Core's IsMine() logic
#[derive(Debug, Clone)]
pub enum ScriptClassification {
    P2PKH(String),           // Single address receives value
    P2SH(String),            // Single address receives value
    P2PK(String),            // Single address receives value
    ColdStake {              // TWO addresses with different roles
        staker: String,      // Receives delegation (S-address) - gets the VALUE
        owner: String,       // Retains ownership (D-address) - can SPEND
    },
    OpReturn,                // OP_RETURN (no address, no value attribution)
    Coinbase,                // Coinbase marker
    Coinstake,               // Coinstake marker
    Nonstandard,             // Non-standard script
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
    pub sapling_data: Option<SaplingTxData>,  // Sapling-specific data for version >= 3
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
        if let Some(ref sapling) = self.sapling_data {
            writeln!(f, "    sapling_data: {:?}", sapling)?;
        }
        writeln!(f, "}}")?;
        Ok(())
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

/// Sapling transaction data (for version >= 3)
/// Contains all shielded transfer information
#[derive(Serialize, Clone)]
pub struct SaplingTxData {
    /// The net value of Sapling spends minus outputs (can be negative)
    /// Positive: shield -> transparent (unshielding)
    /// Negative: transparent -> shield (shielding)
    pub value_balance: i64,
    
    /// Shielded spends (inputs from shielded pool)
    pub vshielded_spend: Vec<SpendDescription>,
    
    /// Shielded outputs (new notes added to shielded pool)
    pub vshielded_output: Vec<OutputDescription>,
    
    /// Binding signature (64 bytes) - proves balance between spends and outputs
    #[serde(serialize_with = "serialize_bytes")]
    pub binding_sig: [u8; 64],
}

impl std::fmt::Debug for SaplingTxData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "SaplingTxData {{")?;
        writeln!(f, "    value_balance: {} satoshis ({} PIV)", self.value_balance, self.value_balance as f64 / 100_000_000.0)?;
        writeln!(f, "    vshielded_spend: {} spend(s)", self.vshielded_spend.len())?;
        writeln!(f, "    vshielded_output: {} output(s)", self.vshielded_output.len())?;
        writeln!(f, "    binding_sig: {}", hex::encode(self.binding_sig))?;
        write!(f, "}}")
    }
}

/// A shielded spend (input) - describes consumption of a note from the shielded pool
/// Total size: 384 bytes (SPENDDESCRIPTION_SIZE)
#[derive(Serialize, Clone)]
pub struct SpendDescription {
    /// Value commitment (32 bytes) - cryptographic commitment to the value being spent
    pub cv: [u8; 32],
    
    /// Merkle anchor (32 bytes) - root of the note commitment tree at a past block
    pub anchor: [u8; 32],
    
    /// Nullifier (32 bytes) - prevents double-spending of the same note
    pub nullifier: [u8; 32],
    
    /// Randomized public key (32 bytes) - used for spendAuthSig verification
    pub rk: [u8; 32],
    
    /// Zero-knowledge proof (192 bytes) - proves spend is valid without revealing details
    /// Groth16 proof: π_A (48) + π_B (96) + π_C (48)
    #[serde(serialize_with = "serialize_bytes")]
    pub zkproof: [u8; 192],
    
    /// Spend authorization signature (64 bytes) - authorizes this spend
    #[serde(serialize_with = "serialize_bytes")]
    pub spend_auth_sig: [u8; 64],
}

impl fmt::Debug for SpendDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "SpendDescription {{")?;
        writeln!(f, "    cv: {}", hex::encode(self.cv))?;
        writeln!(f, "    anchor: {}", hex::encode(self.anchor))?;
        writeln!(f, "    nullifier: {}", hex::encode(self.nullifier))?;
        writeln!(f, "    rk: {}", hex::encode(self.rk))?;
        writeln!(f, "    zkproof: {}... ({} bytes)", &hex::encode(&self.zkproof[..16]), self.zkproof.len())?;
        writeln!(f, "    spend_auth_sig: {}", hex::encode(self.spend_auth_sig))?;
        write!(f, "}}")
    }
}

/// A shielded output - describes creation of a new note in the shielded pool
/// Total size: 948 bytes (OUTPUTDESCRIPTION_SIZE)
#[derive(Serialize, Clone)]
pub struct OutputDescription {
    /// Value commitment (32 bytes) - cryptographic commitment to the output value
    pub cv: [u8; 32],
    
    /// Note commitment u-coordinate (32 bytes) - commitment to the new note
    pub cmu: [u8; 32],
    
    /// Ephemeral public key (32 bytes) - Jubjub public key for note encryption
    pub ephemeral_key: [u8; 32],
    
    /// Encrypted ciphertext (580 bytes) - encrypted note for recipient
    /// Contains: leading byte (1) + diversifier (11) + value (8) + rcm (32) + memo (512) + auth tag (16)
    #[serde(serialize_with = "serialize_bytes")]
    pub enc_ciphertext: [u8; 580],
    
    /// Outgoing ciphertext (80 bytes) - encrypted note for sender's outgoing viewing key
    /// Contains: pk_d (32) + esk (32) + auth tag (16)
    #[serde(serialize_with = "serialize_bytes")]
    pub out_ciphertext: [u8; 80],
    
    /// Zero-knowledge proof (192 bytes) - proves output construction is valid
    /// Groth16 proof: π_A (48) + π_B (96) + π_C (48)
    #[serde(serialize_with = "serialize_bytes")]
    pub zkproof: [u8; 192],
}

impl std::fmt::Debug for OutputDescription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "OutputDescription {{")?;
        writeln!(f, "    cv: {}", hex::encode(self.cv))?;
        writeln!(f, "    cmu: {}", hex::encode(self.cmu))?;
        writeln!(f, "    ephemeral_key: {}", hex::encode(self.ephemeral_key))?;
        writeln!(f, "    enc_ciphertext: {}... ({} bytes)", &hex::encode(&self.enc_ciphertext[..16]), self.enc_ciphertext.len())?;
        writeln!(f, "    out_ciphertext: {}... ({} bytes)", &hex::encode(&self.out_ciphertext[..16]), self.out_ciphertext.len())?;
        writeln!(f, "    zkproof: {}... ({} bytes)", &hex::encode(&self.zkproof[..16]), self.zkproof.len())?;
        write!(f, "}}")
    }
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<DB>,
    pub cache: Arc<CacheManager>,
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
