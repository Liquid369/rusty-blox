use std::fs;
use std::fs::File;
use std::io::{self, BufRead, Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::convert::TryInto;
use std::fmt;
use std::error::Error;
use core::borrow::Borrow;
use sha2::{Sha256, Digest};

use byteorder::{LittleEndian, ReadBytesExt, ByteOrder};
use hex;
use rocksdb::{DB, Options};

use bitcoin::consensus::encode::{Decodable, VarInt};
use config::{Config, File as ConfigFile};
use fs2::FileExt;
use leveldb::database::Database;
use leveldb::iterator::Iterable;
use leveldb::iterator::LevelDBIterator;
use leveldb::kv::KV;
use db_key::Key;
use leveldb::options::{Options as LevelDBOptions, ReadOptions as LevelDBReadOptions};
struct Hash([u8; 32]);

const PREFIX: [u8; 4] = [0x90, 0xc4, 0xfd, 0xe9];
const MAX_PAYLOAD_SIZE: usize = 10000;

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
    pub version: u32,
    pub inputs: Vec<CTxIn>,
    pub outputs: Vec<CTxOut>,
    pub lock_time: u32,
    pub sapling_tx_data: Option<SaplingTxData>,
    pub extra_payload: Option<String>,
}

pub struct CTxIn {
    pub prevout: COutPoint,
    pub script_sig: CScript,
    pub sequence: u32,
}

pub struct CTxOut {
    pub value: i64,
    pub script_length: i32,
    pub script_pubkey: CScript,
}

#[derive(Debug)]
pub struct COutPoint {
    pub hash: String,
    pub n: u32,
}

pub struct CScript {
    pub script: Vec<u8>,
}

pub struct SaplingTxData {
    pub value: i64,
    pub vshield_spend: Vec<VShieldSpend>,
    pub vshield_output: Vec<VShieldOutput>,
    pub binding_sig: String,
}

pub struct VShieldSpend {
    pub cv: [u8; 32],
    pub anchor: [u8; 32],
    pub nullifier: [u8; 32],
    pub rk: [u8; 32],
    pub proof: [u8; 192],
    pub spend_auth_sig: [u8; 64],
}

pub struct VShieldOutput {
    pub cv: [u8; 32],
    pub cmu: [u8; 32],
    pub ephemeral_key: [u8; 32],
    pub enc_ciphertext: [u8; 580],
    pub out_ciphertext: [u8; 80],
    pub proof: [u8; 192],
}

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
        writeln!(f, "    sapling_tx_data: {:?}", self.sapling_tx_data)?;
        writeln!(f, "    extra_payload: {:?}", self.extra_payload)?;
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
        writeln!(f, "VShieldSpend {{")?;
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
        writeln!(f, "VShieldOutput {{")?;
        writeln!(f, "    cv: {:?}", hex::encode(&self.cv))?;
        writeln!(f, "    cmu: {:?}", hex::encode(&self.cmu))?;
        writeln!(f, "    ephemeral_key: {:?}", hex::encode(&self.ephemeral_key))?;
        writeln!(f, "    enc_ciphertext: {:?}", hex::encode(&self.enc_ciphertext))?;
        writeln!(f, "    out_ciphertext: {:?}", hex::encode(&self.out_ciphertext))?;
        writeln!(f, "    proof: {:?}", hex::encode(&self.proof))?;
        write!(f, "}}")
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load the configuration file
    let mut config = Config::default();
    config.merge(ConfigFile::with_name("config.toml"))?;
    let paths = config.get_table("paths")?;

    // Open RocksDB
    let db_path: &str = &paths
        .get("db_path")
        .and_then(|value| value.to_owned().into_string().ok())
        .ok_or("Missing or invalid db_path in config.toml")?;

    let _db = DB::open_default(db_path)?;

    // Path for blk files "blocks" folder
    let blk_dir: &str = &paths
        .get("blk_dir")
        .and_then(|value| value.to_owned().into_string().ok())
        .ok_or("Invalid blk_dir in config.toml")?;

    let dir = fs::read_dir(blk_dir)
    .map_err(|err| format!("Failed to read directory entries: {}", err))?;

    // Keep track of processed files
    let mut processed_files = Vec::new();

    // Process each file in the directory
    for entry in dir {
        if let Ok(entry) = entry {
            if let Some(file_name) = entry.file_name().to_str() {
                if file_name.starts_with("blk") && file_name.ends_with(".dat") {
                    let file_path = entry.path();
                    if processed_files.contains(&file_path) {
                        continue; // Skip already processed files
                    }
                    process_blk_file(&file_path, &_db)?;
                    processed_files.push(file_path.clone());
                }
            }
        }
    }

    Ok(())
}

fn process_blk_file(file_path: impl AsRef<Path>, _db: &DB) -> io::Result<()> {
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
            4 | 5 | 6 => 112, // Version 4, 5, 6: 112 bytes header
            8..=u32::MAX => 144, // Version 8 and above: 144 bytes header
            _ => 80, // Default: Version 1 to 3: 80 bytes header
        };

        // Read the block header
        let mut header_buffer = vec![0u8; header_size];
        reader.read_exact(&mut header_buffer)?;

        // Process and print the block header
        let block_header = parse_block_header(&header_buffer, header_size);
        println!("{:?}", block_header);

        // Write to RocksDB
        let mut key = vec![b'b'];
        key.extend_from_slice(&block_header.block_hash);
        _db.put(&key, &header_buffer).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        // Process and print tx data
        process_transaction(&mut reader, ver_as_int)?;

        // Move to the next position in the stream
        let next_position = stream_position + block_size as u64 + 8; // 8 bytes for the prefix and size
        file.seek(SeekFrom::Start(next_position))?;
        stream_position = next_position;
    }

    Ok(())
}

fn parse_block_header(slice: &[u8], header_size: usize) -> CBlockHeader {
    // Grab header bytes
    let mut reader = io::Cursor::new(slice);

    // Set buffer
    let mut header_buffer = vec![0u8; header_size];
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
    // Start hashing header for block_hash
    let first_hash = Sha256::digest(&header_buffer);
    let block_hash = Sha256::digest(&first_hash);
    // Reverse final hash
    let reversed_hash: Vec<_> = block_hash.iter().rev().cloned().collect();
    // Test print hash
    println!("Block hash: {:?}", hex::encode(&reversed_hash));
    // Return to original position to start breaking down header
    if let Err(e) = reader.seek(SeekFrom::Start(current_position)) {
        eprintln!("Error while seeking: {:?}", e);
    }

    // Read block version
    let n_version = reader.read_u32::<LittleEndian>().unwrap();
    // Read previous block hash
    let hash_prev_block = {
        let mut buf = [0u8; 32];
        reader.read_exact(&mut buf).unwrap();
        buf
    };
    let block_height = read_ldb_block(&hash_prev_block, header_size).unwrap_or(None);
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
    let (n_accumulator_checkpoint, hash_final_sapling_root) = match header_size {
        112 => {
            let mut acc_checkpoint = [0u8; 32];
            reader.read_exact(&mut acc_checkpoint).unwrap();
            (Some(acc_checkpoint), None)
        }
        144 => {
            let mut final_sapling_root = [0u8; 32];
            reader.read_exact(&mut final_sapling_root).unwrap();
            (None, Some(final_sapling_root))
        }
        _ => (None, None), // Default case
    };

    let block_height = block_height;

    // Create CBlockHeader
    CBlockHeader {
        n_version,
        block_hash: block_hash.into(),
        block_height,
        hash_prev_block,
        hash_merkle_root,
        n_time,
        n_bits,
        n_nonce,
        n_accumulator_checkpoint,
        hash_final_sapling_root,
    }
}

fn process_transaction(mut reader: &mut io::BufReader<&File>, block_version: u32) -> Result<(), io::Error> {
    // Read Tx Amount
    let tx_amt = read_varint(reader)?;
    //println!("TxAmt: {:?}", tx_amt);
    for _ in 0..tx_amt {
        // Tx Version
        let tx_ver_out = reader.read_u32::<LittleEndian>().unwrap();
        // Read the input count
        let input_count = read_varint(reader)?;
        // Read the inputs
        let mut inputs = Vec::new();
        for _ in 0..input_count {
            // Read previous outputs
            let prev_output = read_outpoint(reader)?;
            // Determine script sig length for reading script sig below
            let script_length = read_varint(reader)?;
            // Read script sig
            let mut script = vec![0u8; script_length as usize];
            reader.read_exact(&mut script)?;
            // Read sequence
            let sequence = reader.read_u32::<LittleEndian>()?;

            // Create CTxIn struct and add it to the inputs vector
            let tx_in = CTxIn {
                prevout: prev_output,
                script_sig: CScript { script },
                sequence,
            };
            inputs.push(tx_in);
        }

        // Read the output count
        let output_count = read_varint(reader)?;
        //println!("Outputs: {:?}", output_count);

        // Read the outputs
        let mut outputs = Vec::new();
        for _ in 0..output_count {
            // Read tx value
            let value = reader.read_i64::<LittleEndian>()?;
            // Get script pubkey length
            let script_length = read_varint(reader)?;
            // Set size to read the scriptpubkey
            let mut script = vec![0u8; script_length.try_into().unwrap()];
            reader.read_exact(&mut script)?;

            // Create CTxOut struct and add it to the outputs vector
            let tx_out = CTxOut {
                value: value.try_into().unwrap(),
                script_length: script_length.try_into().unwrap(),
                script_pubkey: CScript { script },
            };
            outputs.push(tx_out);
        }
        // Read tx lock time
        let mut lock_time_buff = 0;
        lock_time_buff = reader.read_u32::<LittleEndian>()?;

        // Only blocks above version 8 MAY contain saplingtxdata
        let sapling_tx_data = if block_version >= 8 {
            match parse_sapling_tx_data(&mut reader) {
                Ok(data) => Some(data),
                Err(err) => return Err(err),
            }
        } else {
            None
        };

        // Only transaction version 2's have extra_payloads
        let payload_data: Option<Vec<u8>> = if tx_ver_out == 2 && block_version >=8 {
            parse_payload_data(reader)?
        } else {
            None
        };
        let extra_payload = payload_data.map(|data| String::from_utf8_lossy(&data).into_owned());

        // Create the CTransaction struct
        let transaction = CTransaction {
            version: tx_ver_out, 
            inputs,
            outputs,
            lock_time: lock_time_buff, 
            sapling_tx_data: sapling_tx_data,
            extra_payload: extra_payload,
        };

        //println!("{:?}", transaction);
    }
    Ok(())
}

fn read_outpoint(reader: &mut dyn Read) -> io::Result<COutPoint> {
    // Set size for hash
    let mut hash = [0u8; 32];
    // Read hash
    reader.read_exact(&mut hash)?;
    // Read output index
    let n = reader.read_u32::<LittleEndian>()?;
    let hex_hash = hex::encode(&hash);

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

fn parse_sapling_tx_data(reader: &mut io::BufReader<&File>) -> Result<SaplingTxData, io::Error> {
    // Read the SaplingTxData
    let value = reader.read_i64::<LittleEndian>()?;
    let vshield_spend = parse_vshield_spends(reader)?;
    let vshield_output = parse_vshield_outputs(reader)?;
    // Read the binding_sig as an array of unsigned chars max size 64
    let mut binding_sig = [0u8; 64];
    reader.read_exact(&mut binding_sig)?;

    // Convert the binding_sig to a String (or any other representation as needed)
    let binding_sig_str = hex::encode(binding_sig);

    // Create and return the SaplingTxData struct
    let sapling_tx_data = Some(SaplingTxData {
        value,
        vshield_spend,
        vshield_output,
        binding_sig: binding_sig_str,
    });

    Ok(sapling_tx_data.unwrap())
}

fn parse_vshield_spends(reader: &mut io::BufReader<&File>) -> Result<Vec<VShieldSpend>, io::Error> {
    // Read the number of vShieldSpend entries
    let count = read_varint(reader)?;

    // Read each vShieldSpend entry
    let mut vshield_spends = Vec::with_capacity(count as usize);
    for _ in 0..count {
        // Define buffer sizes for respective fields
        let buff_32 = [0u8; 32];
        let buff_64 = [0u8; 64];
        let buff_192 = [0u8; 192];
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
            cv,
            anchor,
            nullifier,
            rk,
            proof,
            spend_auth_sig,
        };
        vshield_spends.push(vshield_spend);
    }

    Ok(vshield_spends)
}

fn parse_vshield_outputs(reader: &mut io::BufReader<&File>) -> Result<Vec<VShieldOutput>, io::Error> {
    // Read the number of vShieldOutput entries
    let count = read_varint(reader)?;

    // Read each vShieldOutput entry
    let mut vshield_outputs = Vec::with_capacity(count as usize);
    for _ in 0..count {
        // Define buffer sizes for respective fields
        let buff_32 = [0u8; 32];
        let buff_80 = [0u8; 80];
        let buff_192 = [0u8; 192];
        let buff_580 = [0u8; 580];
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
            cv,
            cmu,
            ephemeral_key,
            enc_ciphertext,
            out_ciphertext,
            proof,
        };
        vshield_outputs.push(vshield_output);
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

fn read_ldb_block(hash_prev_block: &[u8; 32], header_size: usize) -> Result<Option<i32>, Box<dyn Error>> {
    // Load the configuration
    let mut config = Config::default();
    config.merge(ConfigFile::with_name("config.toml"))?;
    let ldb_files_dir = config.get::<String>("paths.ldb_dir")?;
    let ldb_files_path = std::path::Path::new(&ldb_files_dir);

    // Open the LevelDB database
    let options = LevelDBOptions::new();
    let database: Database<Byte33> = match Database::open(ldb_files_path, options) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Error opening database: {:?}", e);
            return Err(Box::new(e));
        }
    };

    // Create the key
    let mut key = [0u8; 33];  // 'b' + 32 bytes
    key[0] = b'b';
    key[1..].copy_from_slice(&hash_prev_block[..]);

    // Get the value from the database.
    let read_options: leveldb::options::ReadOptions<'_, Byte33> = LevelDBReadOptions::new();
    let height = match database.get(read_options, key) {
        Ok(Some(value)) => {
            let value_str = hex::encode(&value);
            match parse_ldb_block(&value) {
                Ok(Some(height)) => {
                    Some(height)
                },
                Ok(None) => {
                    None
                },
                Err(e) => {
                    println!("Error while parsing block: {:?}", e);
                    return Err(e);
                }
            }
        }
        Ok(None) => {
            println!("Key not found in database.");
            None
        }
        Err(e) => {
            println!("Error reading from database: {:?}", e);
            return Err(Box::new(e));
        }
    };

    Ok(height)
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

fn parse_ldb_block(block: &[u8]) -> Result<Option<i32>, Box<dyn Error>> {
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
        None => return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Block height overflow when incremented.",
        ))),
    };

    Ok(Some(incremented_block_height.try_into()?))
}
