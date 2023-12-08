use std::fs;
use std::fs::File;
use std::io::{self, BufRead, Read, Seek, SeekFrom, ErrorKind, Cursor};
use std::sync::{Arc};
use std::path::{Path, PathBuf};
use std::collections::HashSet;
use std::convert::TryInto;
use std::fmt;
use std::error::Error;
use core::borrow::Borrow;
use sha2::{Sha256, Digest};
use ripemd160::{Ripemd160, Digest as Ripemd160Digest};
use serde_json::{Value, json};
use serde::Serialize;
use lazy_static::lazy_static;
use tokio::task;
use tokio::task::JoinError;
use tokio::sync::Mutex;

use byteorder::{LittleEndian, ReadBytesExt};
use hex;
use rocksdb::{DB, Options, ColumnFamilyDescriptor};

use bitcoin::consensus::encode::{Decodable, VarInt};
use config::{Config, File as ConfigFile};
use leveldb::database::Database;
use leveldb::kv::KV;
use leveldb::options::{Options as LevelDBOptions, ReadOptions as LevelDBReadOptions};
//use pivx_rpc_rs;

//use pivx_rpc_rs::FullBlock;
//use pivx_rpc_rs::BitcoinRpcClient;
use rusty_piv::BitcoinRpcClient;
use rustyblox::call_quark_hash;

use once_cell::sync::OnceCell;

static GLOBAL_CONFIG: OnceCell<Config> = OnceCell::new();

mod parser;
mod api;
struct Hash([u8; 32]);

const PREFIX: [u8; 4] = [0x90, 0xc4, 0xfd, 0xe9];
const MAX_PAYLOAD_SIZE: usize = 10000;

#[derive(Clone)]
enum AddressType {
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

fn from_rocksdb_error(err: rocksdb::Error) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Byte33([u8; 33]);

impl Borrow<Byte33> for [u8; 33] {
    fn borrow(&self) -> &Byte33 {
        // SAFETY: This transmutes a &[u8; 33] slice into a &Byte33.
        // This is safe as the memory layouts are identical.
        unsafe { &*(self.as_ptr() as *const Byte33) }
    }
}

impl db_key::Key for Byte33 {
    fn from_u8(key: &[u8]) -> Self {
        let mut arr = [0u8; 33];
        arr.copy_from_slice(key);
        Byte33(arr)
    }

    fn as_slice<T, F: Fn(&[u8]) -> T>(&self, f: F) -> T {
        let Byte33(inner) = self;
        f(&inner[..])
    }
}

impl fmt::LowerHex for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0.iter().rev() {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
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

pub struct CTransaction {
    pub version: i16,
    pub inputs: Vec<CTxIn>,
    pub outputs: Vec<CTxOut>,
    pub lock_time: u32,
}

pub struct CTxIn {
    pub prevout: Option<COutPoint>,
    pub script_sig: CScript,
    pub sequence: u32,
    pub index: u64,
    pub coinbase: Option<Vec<u8>>,
}

#[derive(Clone)]
pub struct CTxOut {
    pub value: i64,
    pub script_length: i32,
    pub script_pubkey: CScript,
    pub index: u64,
    pub address: Vec<String>,
}

#[derive(Debug, Default)]
pub struct COutPoint {
    pub hash: String,
    pub n: u32,
}

#[derive(Clone)]
pub struct CScript {
    pub script: Vec<u8>,
}

#[derive(Serialize)]
pub struct SaplingTxData {
    pub value: i64,
    pub vshield_spend: Vec<VShieldSpend>,
    pub vshield_output: Vec<VShieldOutput>,
    pub binding_sig: Vec<u8>,
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

#[derive(Serialize)]
pub struct VShieldOutput {
    pub cv: Vec<u8>,
    pub cmu: Vec<u8>,
    pub ephemeral_key: Vec<u8>,
    pub enc_ciphertext: Vec<u8>,
    pub out_ciphertext: Vec<u8>,
    pub proof: Vec<u8>,
}

#[derive(Debug)]
pub struct CustomError {
    message: String,
}

impl From<JoinError> for CustomError {
    fn from(error: JoinError) -> Self {
        CustomError::new(&format!("Task join error: {}", error))
    }
}

impl CustomError {
    pub fn new(message: &str) -> CustomError {
        CustomError {
            message: message.to_string(),
        }
    }
}

impl fmt::Display for CustomError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Error: {}", self.message)
    }
}

impl Error for CustomError {}

impl std::fmt::Debug for CScript {
    // Formatting for CScript
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", hex::encode(&self.script))
    }
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
            writeln!(f, "Accumulator Checkpoint: {:?}", hex::encode(accumulator_checkpoint))?;
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

impl std::fmt::Debug for CTransaction {
    // Formatting for CTransaction
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Transaction {{")?;
        writeln!(f, "    version: {}", self.version)?;
        writeln!(f, "    inputs: {:?}", self.inputs)?;
        writeln!(f, "    outputs: {:?}", self.outputs)?;
        writeln!(f, "    lock_time: {}", self.lock_time)?;
        write!(f, "}}")
    }
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

impl fmt::Debug for VShieldSpend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Formatting vShieldSpend
        writeln!(f, "{{")?;
        writeln!(f, "    cv: {:?}", hex::encode(&self.cv))?;
        writeln!(f, "    anchor: {:?}", hex::encode(&self.anchor))?;
        writeln!(f, "    nullifier: {:?}", hex::encode(&self.nullifier))?;
        writeln!(f, "    rk: {:?}", hex::encode(&self.rk))?;
        writeln!(f, "    proof: {:?}", hex::encode(&self.proof))?;
        writeln!(f, "    spend_auth_sig: {:?}", hex::encode(&self.spend_auth_sig))?;
        write!(f, "}}")
    }
}

impl std::fmt::Debug for VShieldOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Formatting vShieldOutput
        writeln!(f, "{{")?;
        writeln!(f, "    cv: {:?}", hex::encode(&self.cv))?;
        writeln!(f, "    cmu: {:?}", hex::encode(&self.cmu))?;
        writeln!(f, "    ephemeral_key: {:?}", hex::encode(&self.ephemeral_key))?;
        writeln!(f, "    enc_ciphertext: {:?}", hex::encode(&self.enc_ciphertext))?;
        writeln!(f, "    out_ciphertext: {:?}", hex::encode(&self.out_ciphertext))?;
        writeln!(f, "    proof: {:?}", hex::encode(&self.proof))?;
        write!(f, "}}")
    }
}

const COLUMN_FAMILIES: [&str; 7] = [
    "blocks", "transactions",
    "addr_index", "utxo",
    "chain_metadata", "pubkey",
    "chain_state",
];

fn init_global_config() -> Result<(), Box<dyn Error>> {
    let mut config = Config::default();
    config.merge(ConfigFile::with_name("config.toml"))?;
    GLOBAL_CONFIG.set(config).map_err(|_| "Config already set")?;
    Ok(())
}

fn get_global_config() -> &'static Config {
    GLOBAL_CONFIG.get().expect("Config not initialized")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load the configuration file
    init_global_config()?;
    let config = get_global_config();
    let paths = config.get_table("paths")?;

    // Open RocksDB
    let db_path = paths
        .get("db_path")
        .and_then(|value| value.to_owned().into_string().ok())
        .ok_or("Missing or invalid db_path in config.toml")?;
    let mut cf_descriptors = vec![ColumnFamilyDescriptor::new("default", Options::default())];
    for cf in COLUMN_FAMILIES.iter() {
        cf_descriptors.push(ColumnFamilyDescriptor::new(cf.to_string(), Options::default()));
    }

    let mut db_options = Options::default();
    db_options.create_if_missing(true);
    db_options.create_missing_column_families(true);
    let db = DB::open_cf_descriptors(&db_options, db_path, cf_descriptors)?;
    let db = Arc::new(db);

    // Path for blk files "blocks" folder
    let blk_dir = paths
        .get("blk_dir")
        .and_then(|value| value.to_owned().into_string().ok())
        .ok_or("Invalid blk_dir in config.toml")?;

    // Load processed files from the default column family
    let processed_files = load_processed_files_from_db(&db)?; 
    let processed_files = Arc::new(Mutex::new(processed_files));

    // Process each file in the directory
    let entries = tokio::task::spawn_blocking(move || -> Result<Vec<PathBuf>, std::io::Error> {
        fs::read_dir(blk_dir)?
            .map(|res| res.map(|e| e.path())) // Transform DirEntry to Result<PathBuf, std::io::Error>
            .collect() // Collects into Result<Vec<PathBuf>, std::io::Error>
    }).await??;

    let futures: Vec<_> = entries.into_iter().map(|file_path| {
        let db_clone = Arc::clone(&db);
        let processed_files_clone = Arc::clone(&processed_files);
    
        tokio::task::spawn(async move {
            let should_process = {
                let processed_files = processed_files_clone.lock().await;
                if let Some(file_name) = file_path.file_name().and_then(|n| n.to_str()) {
                    file_name.starts_with("blk") && file_name.ends_with(".dat") && !processed_files.contains(&file_path)
                } else {
                    false
                }
            }; 
    
            if should_process {
                match process_blk_file(&file_path, &db_clone).await {
                    Ok(_) => {
                        // Lock the mutex to modify processed_files and immediately drop it
                        let mut processed_files_guard = processed_files_clone.lock().unwrap();
                        processed_files_guard.insert(file_path);
                        drop(processed_files_guard); // Explicitly drop the MutexGuard
                        
                        // Now it's safe to call async operations
                        if let Err(save_err) = save_processed_files_to_db(&db_clone, &*processed_files_guard) {
                            eprintln!("Failed to save processed files to the database: {}", save_err);
                        }
                    },
                    Err(process_err) => {
                        eprintln!("Failed to process blk file: {}", process_err);
                    }
                }
            }
    
            Ok::<(), String>(())
        })
    }).collect();

    // Wait for all tasks to complete
    for future in futures {
        let _ = future.await?;
    }

    Ok(())
}
fn load_processed_files_from_db(db: &DB) -> Result<HashSet<PathBuf>, String> {
    let read_options = rocksdb::ReadOptions::default();
    let cf = db.cf_handle("chain_metadata").expect("Chain metadata column family not found."); // Using chain_metadata for this
    let data = db.get_cf_opt(cf, b"processed_files", &read_options)?;
    if let Some(data) = data {
        let files: HashSet<PathBuf> = bincode::deserialize(&data)
            .map_err(|e| format!("Bincode deserialization error: {}", e))?;
        Ok(files)
    } else {
        Ok(HashSet::new())
    }
}

fn save_processed_files_to_db(db: &DB, processed_files: &HashSet<PathBuf>) -> Result<(), String> {
    let cf = db.cf_handle("chain_metadata").expect("Chain metadata column family not found.");
    let data = bincode::serialize(processed_files)
        .map_err(|e| format!("Bincode serialization error: {}", e))?;
    db.put_cf(cf, b"processed_files", &data)?;
    Ok(())
}

async fn process_blk_file(file_path: impl AsRef<Path>, _db: &DB) -> io::Result<()> {
    // Open file
    let mut file = File::open(file_path)?;
    // Set buffers for prefix, size
    let mut prefix_buffer = [0u8; 4];
    let mut size_buffer = [0u8; 4];
    // Counting positions for loop
    let mut stream_position = 0;

    loop {
        let mut reader = io::BufReader::new(&file);
        reader.seek(SeekFrom::Start(stream_position))?;

        // Read the prefix
        if reader.read_exact(&mut prefix_buffer).is_err() {
            // Reached end of stream
            break;
        }

        // Check if the prefix matches
        if prefix_buffer != PREFIX {
            // Find the next prefix
            let _next_prefix = [0u8; 4];
            let mut prefix_found = false;

            while !prefix_found {
                // Move to the next byte in the stream
                let mut byte = [0u8; 1];
                if reader.read_exact(&mut byte).is_err() {
                    // Reached end of stream
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Invalid prefix found or end of file",
                    ));
                }

                // Shift the bytes in the prefix buffer
                for i in (2..4).rev() {
                    prefix_buffer[i] = prefix_buffer[i - 2];
                }
                
                // Add the new byte to the prefix buffer
                prefix_buffer[0] = prefix_buffer[2];
                prefix_buffer[1] = byte[0];

                // Check if the prefix matches
                if prefix_buffer == PREFIX {
                    prefix_found = true;
                    continue;
                }
            }

            // Alert to no prefix being found
            if !prefix_found {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "No prefix found",
                ));
            }

            continue; // Continue to the next iteration of the outer loop
        }
        //println!("Prefix buffer: {:?}", prefix_buffer);
        println!("Prefix hex: {}", hex::encode(&prefix_buffer));

        // Convert the block size to little-endian u32
        reader.read_exact(&mut size_buffer)?;
        let block_size = u32::from_le_bytes(size_buffer);

        println!("Block Size: {}", block_size);

        // Peek at next 4 bytes, need to know the version before setting header size
        let _version = read_4_bytes(&mut reader)?;
        let ver_as_int = u32::from_le_bytes(_version);

        // Variable header size based on block versions
        let header_size = match ver_as_int {
            4 | 5 | 6 | 8 | 9 | 10 | 11 => 112, // Version 4, 5, 6, 8: 112 bytes header
            7 => 80, // Version 7 is 80 bytes
            //8..=u32::MAX => 144, // Version 8 and above: 144 bytes header
            _ => 80, // Default: Version 1 to 3: 80 bytes header
        };

        // Read the block header
        let mut header_buffer = vec![0u8; header_size];
        reader.read_exact(&mut header_buffer)?;

        // Process and print the block header
        let block_header = parse_block_header(&header_buffer, header_size).await;
        println!("{:?}", block_header);

        // Write to RocksDB
        // 'b' + block_hash -> block_data
        let cf_blocks = _db.cf_handle("blocks").expect("Blocks column family not found.");
        let mut key = vec![b'b'];
        key.extend_from_slice(&block_header.block_hash);
        _db.put_cf(cf_blocks, &key, &header_buffer).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        // 'h' + block_height -> block_hash
        let mut key_height = vec![b'h'];
        let height = block_header.block_height.unwrap_or(0);
        let height_bytes = height.to_le_bytes();
        key_height.extend_from_slice(&height_bytes);
        _db.put_cf(cf_blocks, &key_height, &block_header.block_hash).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        // Process and print tx data
        process_transaction(&mut reader, ver_as_int, &block_header.block_hash, _db)?;

        // Move to the next position in the stream
        let next_position = stream_position + block_size as u64 + 8; // 8 bytes for the prefix and size
        file.seek(SeekFrom::Start(next_position))?;
        stream_position = next_position;
    }

    Ok(())
}

async fn parse_block_header(slice: &[u8], header_size: usize) -> CBlockHeader {
    // Grab header bytes
    let mut reader = io::Cursor::new(slice);

    // Set buffer
    let max_size = 112;
    let mut header_buffer = vec![0u8; header_size.min(max_size)];
    // Set position
    let current_position = match reader.seek(SeekFrom::Current(0)) {
        Ok(pos) => pos,
        Err(e) => {
            eprintln!("Error while setting current position: {:?}", e);
            0 // or some other default value or action
        }
    };
    // Read buffer
    if let Err(e) = reader.read_exact(&mut header_buffer) {
        eprintln!("Error while reading header buffer: {:?}", e);
    }
    println!("Header Buffer: {:?}", hex::encode(&header_buffer));
    // Return to original position to start breaking down header
    if let Err(e) = reader.seek(SeekFrom::Start(current_position)) {
        eprintln!("Error while seeking: {:?}", e);
    }
    // Read block version
    let n_version = reader.read_u32::<LittleEndian>().unwrap();
    // Read previous block hash
    let mut hash_prev_block = {
        let mut buf = [0u8; 32];
        reader.read_exact(&mut buf).unwrap();
        if n_version < 4 {
            buf.reverse(); // Reverse the hash for n_version less than 4
        }
        buf
    };
    // Calculate the hash based on the version
    let reversed_hash = match n_version {
        0..=3 => {
            // Use quark_hash for n_version less than 4
            let output_hash = call_quark_hash(&header_buffer);
            output_hash.iter().rev().cloned().collect::<Vec<_>>()
        },
        _ => {
            // Use SHA-256 based hashing for n_version 4 or greater
            Sha256::digest(&Sha256::digest(&header_buffer)).iter().rev().cloned().collect::<Vec<_>>()
        },
    };

    // Test print hash
    println!("Block hash: {:?}", hex::encode(&reversed_hash));

    let reversed_hash_array: [u8; 32] = match reversed_hash.try_into() {
        Ok(arr) => arr,
        Err(_) => panic!("Expected a Vec<u8> of length 32"),
    };
    // Determine the block height
    /*let block_height = match n_version {
        0..=3 if hash_prev_block.iter().all(|&b| b == 0) => {
            // If hash_prev_block is all zeros for version less than 4, assign 0
            0
        },
        0..=3 => {
            get_block_height(&reversed_hash_array).await.unwrap_or(Some(0)).unwrap_or(0)
        },
        _ => {
            get_block_height(&reversed_hash_array).await.unwrap_or(Some(0)).unwrap_or(0)
        },
    };*/

    let block_height = read_ldb_block_async(&hash_prev_block, header_size).await.unwrap_or(None);

    // Reverse hash_prev_block back to its original order if n_version is less than 4
    if n_version < 4 {
        hash_prev_block.reverse();
    }
    // Read merkle root
    let hash_merkle_root = {
        let mut buf = [0u8; 32];
        reader.read_exact(&mut buf).unwrap();
        buf
    };
    // Read nTime, nBits, and nNonce
    let n_time = reader.read_u32::<LittleEndian>().unwrap();
    let n_bits = reader.read_u32::<LittleEndian>().unwrap();
    let n_nonce = reader.read_u32::<LittleEndian>().unwrap();

    // Handle the expanded header size based on the params given
    let (hash_final_sapling_root, n_accumulator_checkpoint) = match n_version {
        7 => (None, None),
        8..=11 => {
            let mut final_sapling_root = [0u8; 32];
            reader.read_exact(&mut final_sapling_root).expect("Failed to read final sapling root");
            (Some(final_sapling_root), None)
        },
        4..=6 => {
            let mut accumulator_checkpoint = [0u8; 32];
            reader.read_exact(&mut accumulator_checkpoint).expect("Failed to read accumulator checkpoint");
            (None, Some(accumulator_checkpoint))
        },
        _ => (None, None),
    };

    // Create CBlockHeader
    CBlockHeader {
        n_version,
        block_hash: reversed_hash_array,
        block_height: block_height,
        hash_prev_block,
        hash_merkle_root,
        n_time,
        n_bits,
        n_nonce,
        n_accumulator_checkpoint,
        hash_final_sapling_root,
    }
}

fn read_script<R: io::Read>(reader: &mut R) -> Result<Vec<u8>, io::Error> {
    let script_length = read_varint(reader)?;
    let mut script = vec![0u8; script_length as usize];
    reader.read_exact(&mut script)?;
    Ok(script)
}

fn handle_address(_db: &DB, address_type: &AddressType, reversed_txid: &Vec<u8>, tx_out_index: u32) -> Result<(), io::Error> {
    let address_keys = match address_type {
        AddressType::P2PKH(address) | AddressType::P2SH(address) => vec![address.clone()],
        AddressType::P2PK(pubkey) => vec![pubkey.clone()],
        AddressType::Staking(staker, owner) => vec![staker.clone(), owner.clone()],
        _ => return Ok(()),
    };
    
    for address_key in &address_keys {
        let cf_addr = _db.cf_handle("addr_index").expect("Address_index column family not found");
        let mut key_address = vec![b'a']; 
        key_address.extend_from_slice(address_key.as_bytes());
        let existing_data = _db.get_cf(cf_addr, &key_address).map_err(from_rocksdb_error)?;
        let mut existing_utxos = existing_data.as_deref().map_or(Vec::new(), deserialize_utxos);
        existing_utxos.push((reversed_txid.clone(), tx_out_index.into()));
        _db.put_cf(cf_addr, &key_address, &serialize_utxos(&existing_utxos)).map_err(from_rocksdb_error)?;
    }

    Ok(())
}

fn process_transaction(mut reader: &mut io::BufReader<&File>, block_version: u32, block_hash: &[u8], _db: &DB) -> Result<(), io::Error> {
    let tx_amt = read_varint(reader)?;
    for _ in 0..tx_amt {
        let start_pos = reader.stream_position()?;

        let tx_ver_out = reader.read_u16::<LittleEndian>()?;
        let tx_type = reader.read_u16::<LittleEndian>()?;

        if block_version == 11 {
            if tx_ver_out < 3 {
                process_transaction_v1(reader, tx_ver_out.try_into().unwrap(), block_version, block_hash, _db, start_pos)?;
            } else {
                parse_sapling_tx_data(reader, start_pos, _db)?;
            }
        } else if (tx_ver_out <= 2 && block_version < 11) || (tx_ver_out > 1 && block_version > 7) {
            if tx_ver_out <= 2 {
                process_transaction_v1(reader, tx_ver_out.try_into().unwrap(), block_version, block_hash, _db, start_pos)?;
            } else {
                parse_sapling_tx_data(reader, start_pos, _db)?;
            }
        }
    }
    Ok(())
}

fn process_transaction_v1(reader: &mut io::BufReader<&File>, tx_ver_out: i16, block_version: u32, block_hash: &[u8], _db: &DB, start_pos: u64) -> Result<(), io::Error> {
    let cf_transactions = _db.cf_handle("transactions").expect("Transaction column family not found");
    let cf_pubkey = _db.cf_handle("pubkey").expect("Pubkey column family not found");
    let cf_utxo = _db.cf_handle("utxo").expect("UTXO column family not found");
    let input_count = read_varint(reader)?;

    let inputs = (0..input_count)
    .map(|i| {
        let mut coinbase = None;
        let mut prev_output = None;
        let mut script = None;

        match (block_version, tx_ver_out) {
            (ver, 2) if ver < 3 => {
                let mut buffer = [0; 26];
                reader.read_exact(&mut buffer)?;
                coinbase = Some(buffer.to_vec());
            }
            _ => {
                prev_output = Some(read_outpoint(reader)?);
                script = Some(read_script(reader)?);
            }
        }

        let sequence = reader.read_u32::<LittleEndian>()?;
        Ok(CTxIn {
            prevout: prev_output,
            script_sig: CScript { script: script.unwrap_or_default() }, 
            sequence,
            index: i,
            coinbase,
        })
    })
    .collect::<Result<Vec<_>, std::io::Error>>()?;

    let output_count = read_varint(reader)?;
    let mut general_address_type = if input_count == 1 && output_count == 1 {
        AddressType::CoinBaseTx
    } else if output_count > 1 {
        AddressType::CoinStakeTx
    } else {
        AddressType::Nonstandard
    };
    let outputs = (0..output_count)
        .map(|i| {
            let value = reader.read_i64::<LittleEndian>()?;
            let script = read_script(reader)?;
            let address_type = get_address_type(&CTxOut {
                value,
                script_length: script.len().try_into().unwrap(),
                script_pubkey: CScript { script: script.clone() }, // Cloned because we need it again
                index: i,
                address: Vec::new(), // Temporary dummy value
            }, &general_address_type);
            let addresses = address_type_to_string(Some(address_type.clone()));

            Ok(CTxOut {
                value,
                script_length: script.len().try_into().unwrap(),
                script_pubkey: CScript { script },
                index: i,
                address: addresses, // directly assign the Vec<String>
            })
        })
        .collect::<Result<Vec<_>, std::io::Error>>()?;

    let lock_time_buff = reader.read_u32::<LittleEndian>()?;

    let transaction = CTransaction {
        version: tx_ver_out, 
        inputs,
        outputs: outputs.clone(),
        lock_time: lock_time_buff, 
    };

    let end_pos: u64 = set_end_pos(reader, start_pos)?;
    let tx_bytes: Vec<u8> = get_txid_bytes(reader, start_pos, end_pos)?;
    //println!("Tx Bytes: {:?}", hex::encode(&tx_bytes));
    let reversed_txid: Vec<u8> = hash_txid(&tx_bytes)?;

    println!("Transaction ID: {:?}", hex::encode(&reversed_txid));

    let mut key_pubkey = vec![b'p'];
    for tx_out in &transaction.outputs {
        let address_type = get_address_type(tx_out, &general_address_type);

        // Associate by these with UTXO set
        handle_address(_db, &address_type, &reversed_txid, tx_out.index.try_into().unwrap())?;

        // 'p' + scriptpubkey -> list of (txid, output_index)
        key_pubkey.extend_from_slice(&tx_out.script_pubkey.script); 

        // Fetch existing UTXOs
        let existing_data_option = _db.get(&key_pubkey);
        if let Ok(Some(existing_data)) = existing_data_option {
            let mut existing_utxos = deserialize_utxos(&existing_data);
            // Add new UTXO
            existing_utxos.push((reversed_txid.clone(), tx_out.index));

            // Store the updated UTXOs
            let serialized_utxos = serialize_utxos(&existing_utxos);
            _db.put_cf(cf_pubkey, &key_pubkey, &serialized_utxos).unwrap();
        }
        // Create a UTXO identifier (txid + output index)
        let mut key_utxo = vec![b'u'];
        key_utxo.extend_from_slice(&hex::encode(&reversed_txid).into_bytes());
        let utxos_to_serialize = vec![(reversed_txid.clone(), tx_out.index)];
        _db.put_cf(cf_utxo, &key_utxo, &serialize_utxos(&utxos_to_serialize)).unwrap();
    }

    for tx_in in &transaction.inputs {
        let mut key = vec![b't'];
        if let Some(actual_prevout) = &tx_in.prevout {
            key.extend_from_slice(&actual_prevout.hash.as_bytes());
        }
        let spent_output: Option<&CTxOut> = None;
        let tx_data_option = _db.get_cf(cf_transactions, &key).unwrap();
        if let Some(tx_data) = tx_data_option {
            let referenced_transaction = deserialize_transaction(&tx_data, block_version).unwrap();
        
            if let Some(prevout) = &tx_in.prevout {
                let output = &referenced_transaction.outputs[prevout.n as usize];
                let address_type = get_address_type(output, &general_address_type);
        
                let _ = remove_utxo_addr(_db, &address_type, &prevout.hash, prevout.n);
            }
        }
        let mut key_utxo = vec![b'u'];
        if let Some(actual_prevout) = &tx_in.prevout {
            key.extend_from_slice(&actual_prevout.hash.as_bytes());
        }
        if let Ok(Some(data)) = _db.get_cf(cf_pubkey, &key_utxo) {
            let mut utxos = deserialize_utxos(&data);
    
            // Remove the UTXO that matches the current transaction's input
            if let Some(prevout) = &tx_in.prevout {
                let _hash = &prevout.hash;
                let _n = prevout.n;
                if let Some(pos) = utxos.iter().position(|(txid, index)| *txid == _hash.as_bytes() && *index == _n as u64) {
                    utxos.remove(pos);
                }
            }
    
            // Serialize the updated list of UTXOs and store it back in the database
            if !utxos.is_empty() {
                _db.put_cf(cf_pubkey, &key_pubkey, &serialize_utxos(&utxos)).unwrap();
            } else {
                _db.delete_cf(cf_pubkey, &key_pubkey);
            }
        }
    
        // Remove the referenced UTXO from the UTXO set
        _db.delete_cf(cf_utxo, &key_utxo).unwrap();
    }

    // 't' + txid -> tx_bytes
    let mut key = vec![b't'];
    key.extend_from_slice(&reversed_txid);
    _db.put_cf(cf_transactions, &key, &tx_bytes).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    reader.seek(SeekFrom::Start(end_pos))?;

    Ok(())
}

fn get_address_type(tx_out: &CTxOut, general_address_type: &AddressType) -> AddressType {
    let address_type = if !tx_out.script_pubkey.script.is_empty() {
        scriptpubkey_to_address(&tx_out.script_pubkey).unwrap_or_else(|| general_address_type.clone())
    } else {
        general_address_type.clone()
    };
    address_type
}

fn get_txid_bytes<R: Read>(reader: &mut R, start_pos: u64, end_pos: u64) -> Result<Vec<u8>, io::Error> where R: Seek {
    // Calculate tx_size
    let tx_size = (end_pos - start_pos) as usize;
    let mut tx_bytes = vec![0u8; tx_size];
    // Read the transaction bytes
    reader.seek(SeekFrom::Start(start_pos))?;
    reader.read_exact(&mut tx_bytes)?;

    Ok(tx_bytes)
}

fn set_end_pos<R: Read + Seek>(reader: &mut R, start_pos: u64) -> Result<u64, io::Error> {
    let end_pos = reader.stream_position()?;
    reader.seek(SeekFrom::Start(start_pos))?;
    Ok(end_pos)
}

fn hash_txid(tx_bytes: &[u8]) -> Result<Vec<u8>, io::Error> {
    //Create TXID by hashing twice and reversing result
    let first_hash = Sha256::digest(&tx_bytes);
    let txid = Sha256::digest(&first_hash);
    let reversed_txid: Vec<_> = txid.iter().rev().cloned().collect();

    Ok(reversed_txid)
}

fn read_outpoint(reader: &mut dyn Read) -> io::Result<COutPoint> {
    // Set size for hash
    let mut hash = [0u8; 32];
    // Read hash
    reader.read_exact(&mut hash)?;
    // Read output index
    let n = reader.read_u32::<LittleEndian>()?;
    let reversed_bytes = reverse_bytes(&hash);
    let hex_hash = hex::encode(&reversed_bytes);

    Ok(COutPoint { hash: hex_hash, n })
}

fn read_varint<R: Read>(reader: &mut R) -> io::Result<u64> {
    let varint = VarInt::consensus_decode(reader)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(varint.0)
}

fn read_4_bytes(reader: &mut dyn BufRead) -> io::Result<[u8; 4]> {
    let mut buffer = [0u8; 4];
    let peek_buffer = reader.fill_buf()?;
    if peek_buffer.len() < 4 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Insufficient bytes available in the stream",
        ));
    }

    // Copy the 4 bytes into the buffer
    buffer.copy_from_slice(&peek_buffer[..4]);
    Ok(buffer)
}

fn parse_sapling_tx_data(reader: &mut io::BufReader<&File>, start_pos: u64, _db: &DB) -> Result<SaplingTxData, io::Error> {
    let cf_transactions = _db.cf_handle("transactions").expect("Transaction column family not found");
    let cf_pubkey = _db.cf_handle("pubkey").expect("Pubkey column family not found");
    let cf_utxo = _db.cf_handle("utxo").expect("UTXO column family not found");

    // Set empty vectors for later access
    let mut inputs: Vec<CTxIn> = Vec::new();
    let mut outputs: Vec<CTxOut> = Vec::new();
    // Potential Vin Vector
    let input_count = read_varint2(reader)? as u64;
    println!("Input Count: {}", input_count);

    if input_count > 0 {
        inputs = (0..input_count)
            .map(|i| {
                let coinbase = None;
                let prev_output = read_outpoint(reader)?;
                let script = read_script(reader)?;
                let sequence = reader.read_u32::<LittleEndian>()?;
                Ok(CTxIn {
                    prevout: Some(prev_output),
                    script_sig: CScript { script },
                    sequence,
                    index: i,
                    coinbase,
                })
            })
            .collect::<Result<Vec<_>, std::io::Error>>()?;
    }

    let output_count = read_varint(reader)?;
    println!("Output Count: {}", output_count);
    let mut general_address_type = if input_count == 1 && output_count == 1 {
        AddressType::CoinBaseTx
    } else if output_count > 1 {
        AddressType::CoinStakeTx
    } else {
        AddressType::Nonstandard
    };

    if output_count > 0 {
        outputs = (0..output_count)
            .map(|i| {
                let value = reader.read_i64::<LittleEndian>()?;
                let script = read_script(reader)?;
                let address_type = get_address_type(&CTxOut {
                    value,
                    script_length: script.len().try_into().unwrap(),
                    script_pubkey: CScript { script: script.clone() },
                    index: i,
                    address: Vec::new(),
                }, &general_address_type);
                let addresses = address_type_to_string(Some(address_type.clone()));

                Ok(CTxOut {
                    value,
                    script_length: script.len().try_into().unwrap(),
                    script_pubkey: CScript { script },
                    index: i,
                    address: addresses, // directly assign the Vec<String>
                })
            })
            .collect::<Result<Vec<_>, std::io::Error>>()?;
    }

    let lock_time_buff = reader.read_u32::<LittleEndian>()?;
    println!("Lock Time: {}", lock_time_buff);
    // Hacky fix for getting proper values/spends/outputs for Sapling
    let value_count = read_varint(reader)?;
    let value = reader.read_i64::<LittleEndian>()?;
    println!("Value: {}", value);
    // Read the SaplingTxData
    let vshield_spend = parse_vshield_spends(reader)?;
    let vshield_output = parse_vshield_outputs(reader)?;
    // Read the binding_sig as an array of unsigned chars max size 64
    let mut binding_sig = [0u8; 64];
    reader.read_exact(&mut binding_sig)?;

    // Create and return the SaplingTxData struct
    let sapling_tx_data = SaplingTxData {
        value,
        vshield_spend,
        vshield_output,
        binding_sig: binding_sig.to_vec(),
    };

    let serialized_data = bincode::serialize(&sapling_tx_data)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    let end_pos: u64 = set_end_pos(reader, start_pos)?;
    let tx_bytes: Vec<u8> = get_txid_bytes(reader, start_pos, end_pos)?;
    //println!("Tx Bytes: {:?}", hex::encode(&tx_bytes));
    let reversed_txid: Vec<u8> = hash_txid(&tx_bytes)?;
    println!("Sapling TXID: {:?}", hex::encode(&reversed_txid));
    println!("{:?}", sapling_tx_data);

    let mut key_utxo = vec![b'u'];
    let mut referenced_utxo_id: Option<String> = None;
    for tx_in in &inputs {
        let mut key_pubkey = vec![b'p'];
        if let Some(prevout) = &tx_in.prevout {
            key_pubkey.extend_from_slice(&prevout.hash.as_bytes());
            referenced_utxo_id = Some(hex::encode(&prevout.hash));
        }

        if let Ok(Some(data)) = _db.get_cf(cf_pubkey, &key_pubkey) {
            let mut utxos = deserialize_utxos(&data);


            if let Some(prevout) = &tx_in.prevout {
                if let Some(pos) = utxos.iter().position(|(txid, index)| *txid == prevout.hash.as_bytes() && *index == prevout.n as u64) {
                    utxos.remove(pos);
                }
            }

            if !utxos.is_empty() {
                _db.put_cf(cf_pubkey, &key_pubkey, &serialize_utxos(&utxos)).unwrap();
            } else {
                _db.delete_cf(cf_pubkey, &key_pubkey).unwrap();
            }
        }

        key_utxo.extend_from_slice(referenced_utxo_id.as_ref().unwrap().as_bytes());
        _db.delete_cf(cf_utxo, &key_utxo).unwrap();
    }

    for tx_out in &outputs {
        let address_type = get_address_type(tx_out, &general_address_type);
        handle_address(_db, &address_type, &reversed_txid, tx_out.index.try_into().unwrap())?;

        let mut key_pubkey = vec![b'p'];
        key_pubkey.extend_from_slice(&tx_out.script_pubkey.script);

        let existing_data_option = _db.get(&key_pubkey);
        if let Ok(Some(existing_data)) = existing_data_option {
            let mut existing_utxos = deserialize_utxos(&existing_data);
            existing_utxos.push((reversed_txid.clone(), tx_out.index));

            let serialized_utxos = serialize_utxos(&existing_utxos);
            _db.put_cf(cf_pubkey, &key_pubkey, &serialized_utxos).unwrap();
        }

        key_utxo.extend_from_slice(&reversed_txid);
        let utxos_to_serialize = vec![(reversed_txid.clone(), tx_out.index)];
        _db.put_cf(cf_utxo, &key_utxo, &serialize_utxos(&utxos_to_serialize)).unwrap();
    }

    // 't' + txid -> serialized_data
    let mut key = vec![b't'];
    key.extend_from_slice(&reversed_txid);
    _db.put_cf(cf_transactions, &key, &serialized_data).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    Ok(sapling_tx_data)
}

fn parse_vshield_spends(reader: &mut io::BufReader<&File>) -> Result<Vec<VShieldSpend>, io::Error> {
    // Read the number of vShieldSpend entries
    let count = read_varint(reader)? as usize;
    //println!("vShieldSpend Count: {}", count);
    if count == 0 {
        return Ok(Vec::new());
    }

    // Define buffer sizes for respective fields
    let buff_32 = [0u8; 32];
    let buff_64 = [0u8; 64];
    let buff_192 = [0u8; 192];

    // Read each vShieldSpend entry
    let mut vshield_spends = Vec::with_capacity(count.try_into().unwrap());
    for _ in 0..count {
        // Read each field
        let mut cv = buff_32;
        reader.read_exact(&mut cv)?;
        let mut anchor = buff_32;
        reader.read_exact(&mut anchor)?;
        let mut nullifier = buff_32;
        reader.read_exact(&mut nullifier)?;
        let mut rk = buff_32;
        reader.read_exact(&mut rk)?;
        let mut proof = buff_192;
        reader.read_exact(&mut proof)?;
        let mut spend_auth_sig = buff_64;
        reader.read_exact(&mut spend_auth_sig)?;

        // Create and return the VShieldSpend struct
        let vshield_spend = VShieldSpend {
            cv: reverse_bytes(&cv),
            anchor: reverse_bytes(&anchor),
            nullifier: reverse_bytes(&nullifier),
            rk: reverse_bytes(&rk),
            proof: proof.to_vec(),
            spend_auth_sig: spend_auth_sig.to_vec(),
        };
        vshield_spends.push(vshield_spend);
        //println!("{:?}", vshield_spends);
    }
    Ok(vshield_spends)
}

fn parse_vshield_outputs(reader: &mut io::BufReader<&File>) -> Result<Vec<VShieldOutput>, io::Error> {
    // Read the number of vShieldOutput entries
    let count = read_varint(reader)? as usize;
    //println!("vShieldOutput Count: {}", count);
    if count == 0 {
        return Ok(Vec::new());
    }

    // Define buffer sizes for respective fields
    let buff_32 = [0u8; 32];
    let buff_80 = [0u8; 80];
    let buff_192 = [0u8; 192];
    let buff_580 = [0u8; 580];

    // Read each vShieldOutput entry
    let mut vshield_outputs = Vec::with_capacity(count);
    for _ in 0..count {
        // Read each field
        let mut cv = buff_32;
        reader.read_exact(&mut cv)?;
        let mut cmu = buff_32;
        reader.read_exact(&mut cmu)?;
        let mut ephemeral_key = buff_32;
        reader.read_exact(&mut ephemeral_key)?;
        let mut enc_ciphertext = buff_580;
        reader.read_exact(&mut enc_ciphertext)?;
        let mut out_ciphertext = buff_80;
        reader.read_exact(&mut out_ciphertext)?;
        let mut proof = buff_192;
        reader.read_exact(&mut proof)?;

        // Create and return the VShieldOutput struct
        let vshield_output = VShieldOutput {
            cv: reverse_bytes(&cv),
            cmu: reverse_bytes(&cmu),
            ephemeral_key: reverse_bytes(&ephemeral_key),
            enc_ciphertext: enc_ciphertext.to_vec(),
            out_ciphertext: out_ciphertext.to_vec(),
            proof: proof.to_vec(),
        };
        vshield_outputs.push(vshield_output);
        //println!("{:?}", vshield_outputs);
    }

    Ok(vshield_outputs)
}

fn parse_payload_data(reader: &mut io::BufReader<&File>) -> Result<Option<Vec<u8>>, io::Error> {
    let mut prefix_found = false;
    let mut byte_count = 0;
    let mut buffer = [0u8; 4];

    // Read byte by byte until the PREFIX is found or the end of the stream is reached
    while !prefix_found && reader.read_exact(&mut buffer).is_ok() {
        byte_count += 1;
        if buffer == PREFIX {
            prefix_found = true;
        }
    }

    // Adjust the byte count to exclude the PREFIX sequence
    if prefix_found && byte_count >= PREFIX.len() {
        byte_count -= PREFIX.len();
    }

    // Check if the byte count exceeds the maximum payload size
    if byte_count > MAX_PAYLOAD_SIZE {
        // Handle the case where the payload exceeds the maximum size
        Err(io::Error::new(io::ErrorKind::Other, "Payload size exceeds the maximum."))
    } else {
        // Return the payload data as Some if prefix was found, None otherwise
        if prefix_found {
            let mut payload_data = vec![0u8; byte_count];
            reader.read_exact(&mut payload_data)?;
            Ok(Some(payload_data))
        } else {
            Ok(None)
        }
    }
}

async fn get_block_height(hash_block: &[u8; 32]) -> Result<Option<i32>, Box<dyn Error>> {
    let config = get_global_config();
    let rpc_host = config.get::<String>("rpc.host")?;
    let rpc_user = config.get::<String>("rpc.user")?;
    let rpc_pass = config.get::<String>("rpc.pass")?;
    
    let client = BitcoinRpcClient::new(
        rpc_host,
        Some(rpc_user),
        Some(rpc_pass),
        3,   // Max retries
        10,  // Connection timeout
        1000 // Read/write timeout
    );

    let hash_block_hex = hex::encode(hash_block);

    let block_height = match client.getblock(hash_block_hex) {
        Ok(block_info) => Some(
            block_info.height.try_into()
                             .expect("Block height is too large for i32")
        ),
        Err(err) => {
            println!("Failed to get block height {:?}: {:?}", hash_block, err);
            None
        }
    };

    Ok(block_height)
}

fn reverse_bytes(array: &[u8]) -> Vec<u8> {
    let mut vec = Vec::from(array);
    vec.reverse();
    vec
}

// Bitcoin normal varint
pub fn read_varint2<R: Read + ?Sized>(reader: &mut R) -> io::Result<u64> {
    let first = reader.read_u8()?; // read first length byte
    let value = match first {
        0x00..=0xfc => u64::from(first),
        0xfd => u64::from(reader.read_u16::<LittleEndian>()?),
        0xfe => u64::from(reader.read_u32::<LittleEndian>()?),
        0xff => reader.read_u64::<LittleEndian>()?,
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid varint")),
    };
    Ok(value)
}

// Bitcoin varint128
fn read_varint128(data: &[u8]) -> (usize, u64) {
    let mut index = 0;
    let mut value: u64 = 0;
    let mut byte: u8;

    loop {
        byte = data[index];
        index += 1;
        value = (value << 7) | (byte & 0x7F) as u64;

        if (byte & 0x80) != 0 {
            value += 1;
        }

        if (byte & 0x80) == 0 {
            break;
        }
    }

    (index, value)
}

lazy_static! {
    static ref DB_MUTEX: Mutex<()> = Mutex::new(());
}

async fn read_ldb_block_async(hash_prev_block: &[u8; 32], header_size: usize) -> Result<Option<i32>, CustomError> {
    let hash_clone = hash_prev_block.to_owned();
    
    // Offload the blocking operation to a separate thread
    let result = {
        // Lock the mutex only for the duration of cloning the data
        let _lock = DB_MUTEX.lock().unwrap();

        task::spawn_blocking(move || {
            read_ldb_block(&hash_clone, header_size)
        }).await?
    };

    result
}

fn read_ldb_block(hash_prev_block: &[u8; 32], header_size: usize) -> Result<Option<i32>, CustomError> {
    // Load the configuration
    let mut config = Config::default();
    config.merge(ConfigFile::with_name("config.toml"))
          .map_err(|e| CustomError::new(&format!("Error loading config: {}", e)))?;
    let ldb_files_dir = config.get::<String>("paths.ldb_dir")
                             .map_err(|e| CustomError::new(&format!("Error getting ldb_files_dir: {}", e)))?;
    let ldb_files_path = std::path::Path::new(&ldb_files_dir);

    // Open the LevelDB database
    let options = LevelDBOptions::new();
    let database: Database<Byte33> = Database::open(ldb_files_path, options)
                                         .map_err(|e| CustomError::new(&format!("Error opening database: {}", e)))?;

    // Create the key
    let mut key = [0u8; 33];  // 'b' + 32 bytes
    key[0] = b'b';
    key[1..].copy_from_slice(&hash_prev_block[..]);

    // Get the value from the database.
    let read_options: leveldb::options::ReadOptions<'_, Byte33> = LevelDBReadOptions::new();
    let height = match database.get(read_options, key) {
        Ok(Some(value)) => {
            // Process the value to get the height, and handle potential errors with new error handling
            parse_ldb_block(&value).map_err(|e| CustomError::new(&format!("Error parsing block: {}", e)))
        },
        Ok(None) => {
            println!("Key not found in database.");
            Ok(None)
        },
        Err(e) => {
            println!("Error reading from database: {:?}", e);
            Err(CustomError::new(&format!("Error reading from database: {}", e)))
        }
    };

    height
}

fn parse_ldb_block(block: &[u8]) -> Result<Option<i32>, CustomError> {
    // Get the slice starting from the 0 position
    let remaining_data = &block[0..];

    // Read version
    let (bytes_consumed_for_version, version) = read_varint128(remaining_data);

    // After reading the version, move to the next unread part of remaining_data
    let next_data = &remaining_data[bytes_consumed_for_version..];

    // Read block height using read_varint128 function
    let (_, block_height) = read_varint128(next_data);

    // Increment the block height
    let incremented_block_height = match block_height.checked_add(1) {
        Some(val) => val,
        None => return Err(CustomError::new("Block height overflow when incremented.")),
    };

    Ok(Some(incremented_block_height.try_into()
        .map_err(|_| CustomError::new("Failed to convert block height to i32"))?))
}

fn compute_address_hash(data: &[u8]) -> Vec<u8> {
    let sha = Sha256::digest(data);
    Ripemd160::digest(&sha).to_vec()
}

// Function to convert hash to P2PKH Bitcoin address (with prefix 0x00 for mainnet)
fn hash_address(hash: &[u8], prefix: u8) -> String {
    let mut extended_hash = vec![prefix]; // This is 30 in hex, the P2PKH prefix you provided
    extended_hash.extend_from_slice(hash);

    let checksum = sha256d(&extended_hash);
    extended_hash.extend_from_slice(&checksum[0..4]);

    bs58::encode(extended_hash).into_string()
}

fn sha256(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

fn ripemd160_hash(data: &[u8]) -> Vec<u8> {
    let mut hasher = Ripemd160::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

fn sha256d(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let first = hasher.finalize();

    let mut hasher = Sha256::new();
    hasher.update(&first);
    hasher.finalize().to_vec()
}

// Function to parse script_pubkey to a P2PKH address
fn scriptpubkey_to_p2pkh_address(script: &CScript) -> Option<String> {
    if script.script.len() == 25 && 
       script.script[0] == 0x76 && 
       script.script[1] == 0xa9 && 
       script.script[2] == 0x14 && 
       script.script[23] == 0x88 && 
       script.script[24] == 0xac {
           let address_hash = &script.script[3..23];
           Some(hash_address(address_hash, 30))
    } else {
        None
    }
}

fn scriptpubkey_to_p2sh_address(script: &CScript) -> Option<String> {
    // OP_HASH160 = 0xa9, followed by a length byte of 20 (0x14 in hexadecimal) and then OP_EQUAL = 0x87
    if script.script.len() == 23 && script.script[0] == 0xa9 && script.script[1] == 0x14 && script.script[22] == 0x87 {
        let address_hash = &script.script[2..22];
        Some(hash_address(address_hash, 13))
    } else {
        None
    }
}

fn compress_pubkey(pub_key_bytes: &[u8]) -> Option<Vec<u8>> {
    match pub_key_bytes.len() {
        65 if pub_key_bytes[0] == 0x04 => {
            let x = &pub_key_bytes[1..33];
            let y = &pub_key_bytes[33..65];
            let parity = if y[31] % 2 == 0 { 2 } else { 3 };
            let mut compressed_key: Vec<u8> = vec![parity];
            compressed_key.extend_from_slice(x);
            Some(compressed_key)
        },
        33 if pub_key_bytes[0] == 0x02 || pub_key_bytes[0] == 0x03 => {
            // Already compressed, just return as is
            Some(pub_key_bytes.to_vec())
        },
        _ => None
    }
}

fn extract_pubkey_from_script(script: &[u8]) -> Option<&[u8]> {
    const OP_CHECKSIG: u8 = 0xAC;

    if script.last()? != &OP_CHECKSIG {
        return None;
    }

    match script.len() {
        67 => Some(&script[1..66]), // skip the OP_PUSHDATA, then take uncompressed pubkey
        35 => Some(&script[1..34]), // skip the OP_PUSHDATA, then take compressed pubkey
        _ => None,
    }
}

fn scriptpubkey_to_p2pk(script: &CScript) -> Option<String> {
    const OP_DUP: u8 = 0x76;

    if script.script.contains(&OP_DUP) {
        return None; // Not a P2PK script.
    }

    let pubkey = extract_pubkey_from_script(&script.script)?;

    let pubkey_compressed = compress_pubkey(pubkey)?;
    let pubkey_hex: Vec<u8> = hex::encode(&pubkey_compressed).into();
    let pubkey_hash = compute_address_hash(&pubkey_compressed);
    let pubkey_addr = hash_address(&pubkey_hash, 30);

    Some(pubkey_addr)
}

fn scriptpubkey_to_staking_address(script: &CScript) -> Option<(String, String)> {
    const HASH_LEN: usize = 20; // Length of public key hash
    const OP_CHECKCOLDSTAKEVERIFY: u8 = 0xd2;
    const OP_CHECKCOLDSTAKEVERIFY_LOF: u8 = 0xd1;
    const OP_ELSE: u8 = 0x67;

    let pos_checkcoldstakeverify = script.script.iter().position(|&x| x == OP_CHECKCOLDSTAKEVERIFY || x == OP_CHECKCOLDSTAKEVERIFY_LOF)?;

    // Boundary check to avoid panic during slicing
    if script.script.len() < pos_checkcoldstakeverify + 1 + HASH_LEN {
        return None;
    }

    let staker_key_hash = &script.script[(pos_checkcoldstakeverify + 1)..(pos_checkcoldstakeverify + 1 + HASH_LEN)];

    // Find the position of OP_ELSE
    let pos_else = script.script.iter().position(|&x| x == OP_ELSE)?;
    if script.script.len() < pos_else + 1 + HASH_LEN {
        return None;
    }

    let owner_key_hash = &script.script[(pos_else + 1)..(pos_else + 1 + HASH_LEN)];

    let staker_address = hash_address(staker_key_hash, 63); // 63 is the prefix for staker P2PKH
    let owner_address = hash_address(owner_key_hash, 30); // 30 is the prefix for owner P2PKH

    Some((staker_address, owner_address))
}

fn scriptpubkey_to_address(script: &CScript) -> Option<AddressType> {
    // Verify non-empty script
    if script.script.is_empty() {
        return Some(AddressType::Nonstandard);
    }

    // Define op_codes
    const OP_DUP: u8 = 0x76;
    const OP_HASH160: u8 = 0xa9;
    const OP_EQUAL: u8 = 0x87;
    const OP_EQUALVERIFY: u8 = 0x88;
    const OP_CHECKSIG: u8 = 0xac;
    const OP_CHECKCOLDSTAKEVERIFY_LOF: u8 = 0xd1;
    const OP_CHECKCOLDSTAKEVERIFY: u8 = 0xd2;

    // Check the first byte and script length
    match script.script.as_slice() {
        [OP_DUP, OP_HASH160, 0x14, .., OP_EQUALVERIFY, OP_CHECKSIG] if script.script.len() == 25 => {
            if let Some(address) = scriptpubkey_to_p2pkh_address(script) {
                Some(AddressType::P2PKH(address))
            } else {
                Some(AddressType::Nonstandard)
            }
        }
        [OP_HASH160, 0x14, .., OP_EQUAL] if script.script.len() == 23 => {
            if let Some(address) = scriptpubkey_to_p2sh_address(script) {
                Some(AddressType::P2SH(address))
            } else {
                Some(AddressType::Nonstandard)
            }
        }
        [0xc1, ..] => Some(AddressType::ZerocoinMint),
        [0xc2, ..] => Some(AddressType::ZerocoinSpend),
        [0xc3, ..] => Some(AddressType::ZerocoinPublicSpend),
        [.., OP_CHECKSIG] if !script.script.contains(&OP_DUP) && script.script.len() > 1 && !script.script.contains(&OP_CHECKCOLDSTAKEVERIFY) && !script.script.contains(&OP_CHECKCOLDSTAKEVERIFY_LOF) => {
            if let Some(pubkey) = scriptpubkey_to_p2pk(script) {
                Some(AddressType::P2PK(pubkey))
            } else {
                Some(AddressType::Nonstandard)
            }
        }
        _ if script.script.contains(&OP_CHECKCOLDSTAKEVERIFY) || script.script.contains(&OP_CHECKCOLDSTAKEVERIFY_LOF) => {
            if let Some((staker_address, owner_address)) = scriptpubkey_to_staking_address(script) {
                Some(AddressType::Staking(staker_address, owner_address))
            } else {
                Some(AddressType::Nonstandard)
            }
        }
        _ => Some(AddressType::Nonstandard), // Doesn't match non-standard
    }
}

fn address_type_to_string(address: Option<AddressType>) -> Vec<String> {
    match address {
        Some(AddressType::CoinStakeTx) => vec!["CoinStakeTx".to_string()],
        Some(AddressType::CoinBaseTx) => vec!["CoinBaseTx".to_string()],
        Some(AddressType::Nonstandard) => vec!["Nonstandard".to_string()],
        Some(AddressType::P2PKH(addr)) => vec![addr],
        Some(AddressType::P2PK(pubkey)) => vec![pubkey],
        Some(AddressType::P2SH(addr)) => vec![addr],
        Some(AddressType::ZerocoinMint) => vec!["ZerocoinMint".to_string()],
        Some(AddressType::ZerocoinSpend) => vec!["ZerocoinSpend".to_string()],
        Some(AddressType::ZerocoinPublicSpend) => vec!["ZerocoinPublicSpend".to_string()],
        Some(AddressType::Staking(staker, owner)) => vec![format!("Staking({}, {})", staker, owner)],
        Some(AddressType::Sapling) => vec!["Sapling".to_string()],
        None => Vec::new(),
    }
}

fn add_utxo_to_pubkey(db: &DB, pubkey: &[u8], txid: &str, index: u32) {
    let key = format!("pubkey_{}", hex::encode(pubkey));

    // Fetch existing data, if any.
    let existing_data = db.get(&key).unwrap_or(None);

    let mut utxos = match existing_data {
        Some(data) => serde_json::from_slice::<Vec<Value>>(&data).expect("Failed to parse JSON"),
        None => vec![],
    };

    // Append the new UTXO.
    utxos.push(json!({
        "txid": txid,
        "index": index,
    }));

    // Serialize and store back in RocksDB.
    let serialized_data = serde_json::to_vec(&utxos).expect("Failed to serialize JSON");
    db.put(key, serialized_data).expect("Failed to write to RocksDB");
}

fn serialize_utxos(utxos: &Vec<(Vec<u8>, u64)>) -> Vec<u8> {
    let mut serialized = Vec::new();
    for (txid, index) in utxos {
        serialized.extend(txid);
        serialized.extend(&index.to_le_bytes());
    }
    serialized
}

fn deserialize_utxos(data: &[u8]) -> Vec<(Vec<u8>, u64)> {
    let mut utxos = Vec::new();
    let mut iter = data.chunks_exact(40); // 32 bytes for txid and 8 bytes for index
    while let Some(chunk) = iter.next() {
        let txid = chunk[0..32].to_vec();
        let index = u64::from_le_bytes(chunk[32..40].try_into().unwrap());
        utxos.push((txid, index));
    }
    utxos
}

fn deserialize_transaction(data: &[u8], block_version: u32) -> Result<CTransaction, std::io::Error> {
    let mut cursor = Cursor::new(data);

    let version = cursor.read_i16::<LittleEndian>().unwrap();
    let input_count = read_varint(&mut cursor)?;
    let mut inputs = Vec::new();
    for _ in 0..input_count {
        inputs.push(deserialize_tx_in(&mut cursor, version.try_into().unwrap(), block_version));
    }

    let output_count = read_varint(&mut cursor)?;
    let mut outputs = Vec::new();
    for _ in 0..output_count {
        outputs.push(deserialize_tx_out(&mut cursor));
    }

    let lock_time = cursor.read_u32::<LittleEndian>().unwrap();

    Ok(CTransaction {
        version: version,
        inputs: inputs,
        outputs: outputs,
        lock_time: lock_time,
    })
}

fn deserialize_tx_in(cursor: &mut Cursor<&[u8]>, tx_ver_out: u32, block_version: u32) -> CTxIn {
    if block_version < 3 && tx_ver_out == 2 {
        // It's a coinbase transaction
        let mut buffer = [0; 26];
        cursor.read_exact(&mut buffer).unwrap();
        let coinbase = buffer.to_vec();
        let sequence = cursor.read_u32::<LittleEndian>().unwrap();

        CTxIn {
            prevout: None,
            script_sig: CScript { script: Vec::new() },
            sequence: sequence,
            index: 0,
            coinbase: Some(coinbase),
        }
    } else {
        // It's a regular transaction
        let prevout = deserialize_out_point(cursor);
        let script_sig = read_script(cursor).unwrap();
        let sequence = cursor.read_u32::<LittleEndian>().unwrap();
        let index = cursor.read_u64::<LittleEndian>().unwrap();

        CTxIn {
            prevout: Some(prevout),
            script_sig: CScript { script: script_sig },
            sequence: sequence,
            index: index,
            coinbase: None,
        }
    }
}


fn deserialize_tx_out(cursor: &mut Cursor<&[u8]>) -> CTxOut {
    let value = cursor.read_i64::<LittleEndian>().unwrap();
    let script_length = read_varint(cursor);
    let mut script_pubkey = vec![0; script_length.unwrap() as usize];
    cursor.read_exact(&mut script_pubkey).unwrap();
    let index = cursor.read_u64::<LittleEndian>().unwrap();

    let mut address_data = Vec::new();
    cursor.read_to_end(&mut address_data).unwrap();
    let address = String::from_utf8(address_data).unwrap();

    CTxOut {
        value: value,
        script_length: script_pubkey.len() as i32,
        script_pubkey: CScript { script: script_pubkey },
        index: index,
        address: vec![address],
    }
}

fn deserialize_out_point(cursor: &mut Cursor<&[u8]>) -> COutPoint {
    let mut hash_bytes = [0u8; 32];
    cursor.read_exact(&mut hash_bytes).unwrap();
    let hash = hex::encode(hash_bytes);
    let n = cursor.read_u32::<LittleEndian>().unwrap();

    COutPoint {
        hash: hash,
        n: n,
    }
}

fn remove_utxo_addr(_db: &DB, address_type: &AddressType, txid: &str, index: u32) -> Result<(), io::Error> {
    let address_keys = match address_type {
        AddressType::P2PKH(address) | AddressType::P2SH(address) => vec![address.clone()],
        AddressType::P2PK(pubkey) => vec![pubkey.clone()],
        AddressType::Staking(staker, owner) => vec![staker.clone(), owner.clone()],
        _ => return Ok(()),
    };

    for address_key in &address_keys {
        let cf_addr = _db.cf_handle("addr_index").expect("Address_index column family not found");
        let mut key_address = vec![b'a']; 
        key_address.extend_from_slice(address_key.as_bytes());

        // Fetch existing UTXOs associated with this address
        let existing_data = _db.get_cf(cf_addr, &key_address).map_err(from_rocksdb_error)?;
        let mut existing_utxos = existing_data.as_deref().map_or(Vec::new(), deserialize_utxos);

        // Find the UTXO to remove
        if let Some(pos) = existing_utxos.iter().position(|(stored_txid, stored_index)| stored_txid.as_slice() == txid.as_bytes() && *stored_index == index as u64) {
            existing_utxos.remove(pos);
        }

        // Update or delete the UTXO entry for this address
        if !existing_utxos.is_empty() {
            _db.put_cf(cf_addr, &key_address, &serialize_utxos(&existing_utxos)).map_err(from_rocksdb_error)?;
        } else {
            _db.delete_cf(cf_addr, &key_address).map_err(from_rocksdb_error)?;
        }
    }

    Ok(())
}