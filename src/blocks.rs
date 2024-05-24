use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeek, AsyncSeekExt, BufReader, SeekFrom};
use rocksdb::DB;
use crate::db_utils::RocksDBOperations;
use crate::types::{CBlockHeader};
use std::error::Error;

async fn process_blk_file(
    state: AppState,
    file_path: impl AsRef<Path>,
    db: Arc<DB>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Set as in progress
    let file_path_buf = PathBuf::from(file_path.as_ref());
    save_file_as_in_progress(&db, &file_path_buf).await?;

    let mut file = File::open(file_path).await?;
    let mut prefix_buffer = [0u8; 4];
    let mut size_buffer = [0u8; 4];
    let mut stream_position = 0;
    let mut reader = BufReader::new(file);

    while let Ok(_) = reader.seek(SeekFrom::Start(stream_position)).await {
        if reader.read_exact(&mut prefix_buffer).await.is_err() {
            break; // End of file or error
        }

        if prefix_buffer != PREFIX {
            continue; // If the prefix doesn't match, skip to next
        }

        reader.read_exact(&mut size_buffer).await?;
        let block_size = u32::from_le_bytes(size_buffer);

        let _version = read_4_bytes(&mut reader).await?;
        let ver_as_int = u32::from_le_bytes(_version);
        let header_size = get_header_size(ver_as_int).await;

        let mut header_buffer = vec![0u8; header_size];
        reader.read_exact(&mut header_buffer).await?;

        let block_header = parse_block_header(&header_buffer, header_size).await?;
        
        let cf_name = Arc::new(db.cf_handle("blocks").ok_or("Column family not found")?);
        let block_hash_vec: Vec<u8> = block_header.block_hash.iter().rev().cloned().collect();
        perform_rocksdb_put(db.clone(), "blocks", block_hash_vec.clone(), header_buffer).await;
        let height_bytes = block_header.block_height.unwrap_or(0).to_le_bytes();
        let height_bytes_vec = height_bytes.to_vec();
        perform_rocksdb_put(db.clone(), "blocks", height_bytes_vec, block_hash_vec.clone()).await;

        process_transaction(&mut reader, ver_as_int, &block_header.block_hash, db.clone()).await?;

        stream_position += block_size as u64 + 8; // Adjusting stream_position for the next block
    }

    Ok(())
}

async fn get_header_size(ver_as_int: u32) -> usize {
    match ver_as_int {
        4 | 5 | 6 | 8 | 9 | 10 | 11 => 112,
        7 => 80,
        _ => 80,
    }
}

async fn parse_block_header(slice: &[u8], header_size: usize) -> Result<CBlockHeader, MyError> {
    // Grab header bytes
    let mut reader = io::Cursor::new(slice);

    // Set buffer
    let max_size = 112;
    let mut header_buffer = vec![0u8; header_size.min(max_size)];
    // Set position
    let current_position = match reader.seek(SeekFrom::Current(0)).await {
        Ok(pos) => pos,
        Err(e) => {
            eprintln!("Error while setting current position: {:?}", e);
            0 // or some other default value or action
        }
    };
    // Read buffer
    if let Err(e) = reader.read_exact(&mut header_buffer).await {
        eprintln!("Error while reading header buffer: {:?}", e);
    }
    //println!("Header Buffer: {:?}", hex::encode(&header_buffer));
    // Return to original position to start breaking down header
    if let Err(e) = reader.seek(SeekFrom::Start(current_position)).await {
        eprintln!("Error while seeking: {:?}", e);
    }
    // Read block version
    let n_version = reader.read_u32_le().await.unwrap();
    // Read previous block hash
    let mut hash_prev_block = {
        let mut buf = [0u8; 32];
        reader.read_exact(&mut buf).await.unwrap();
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
        }
        _ => {
            // Use SHA-256 based hashing for n_version 4 or greater
            Sha256::digest(&Sha256::digest(&header_buffer))
                .iter()
                .rev()
                .cloned()
                .collect::<Vec<_>>()
        }
    };

    // Test print hash
    println!("Block hash: {:?}", hex::encode(&reversed_hash));

    let reversed_hash_array: [u8; 32] = match reversed_hash.try_into() {
        Ok(arr) => arr,
        Err(_) => panic!("Expected a Vec<u8> of length 32"),
    };
    // Determine the block height
    let block_height = match n_version {
        0..=3 if hash_prev_block.iter().all(|&b| b == 0) => {
            // If hash_prev_block is all zeros for version less than 4, assign 0
            0
        }
        _ => match get_block_height_fallback(&reversed_hash_array, header_size).await {
            Ok(Some(height)) => height,
            Ok(None) | Err(_) => {
                0
            },
        },
    };

    // Reverse hash_prev_block back to its original order if n_version is less than 4
    if n_version < 4 {
        hash_prev_block.reverse();
    }
    // Read merkle root
    let hash_merkle_root = {
        let mut buf = [0u8; 32];
        reader.read_exact(&mut buf).await.unwrap();
        buf
    };
    // Read nTime, nBits, and nNonce
    let n_time = reader.read_u32_le().await.unwrap();
    let n_bits = reader.read_u32_le().await.unwrap();
    let n_nonce = reader.read_u32_le().await.unwrap();

    // Handle the expanded header size based on the params given
    let (hash_final_sapling_root, n_accumulator_checkpoint) = match n_version {
        7 => (None, None),
        8..=11 => {
            let mut final_sapling_root = [0u8; 32];
            reader
                .read_exact(&mut final_sapling_root).await
                .expect("Failed to read final sapling root");
            (Some(final_sapling_root), None)
        }
        4..=6 => {
            let mut accumulator_checkpoint = [0u8; 32];
            reader
                .read_exact(&mut accumulator_checkpoint).await
                .expect("Failed to read accumulator checkpoint");
            (None, Some(accumulator_checkpoint))
        }
        _ => (None, None),
    };

    // Create CBlockHeader
    Ok(CBlockHeader {
        n_version,
        block_hash: reversed_hash_array,
        block_height: Some(block_height),
        hash_prev_block,
        hash_merkle_root,
        n_time,
        n_bits,
        n_nonce,
        n_accumulator_checkpoint,
        hash_final_sapling_root,
    })
}

async fn get_block_height_fallback(hash_block: &[u8; 32], header_size: usize) -> Result<Option<i32>, Box<dyn Error>> {
    // First, attempt to read from LevelDB
    let ldb_height = read_ldb_block_async(hash_block, header_size).await;
    match ldb_height {
        Ok(Some(height)) => {
            println!("Retrieved from LevelDB: {:?}", height);
            return Ok(Some(height));
        },
        Ok(None) => {
            // Key not found in LevelDB, continue to attempt RPC.
            println!("Key not found in LevelDB, falling back to RPC.");
        },
        Err(e) => {
            eprintln!("LevelDB read error: {}, falling back to RPC", e);
        },
    }

    // If LevelDB read fails or key not found, fall back to Bitcoin RPC
    get_block_height_from_rpc(hash_block).await
}

async fn get_block_height_from_rpc(hash_block: &[u8; 32]) -> Result<Option<i32>, Box<dyn Error>> {
    let config = get_global_config();
    let rpc_host = config.get::<String>("rpc.host")?;
    let rpc_user = config.get::<String>("rpc.user")?;
    let rpc_pass = config.get::<String>("rpc.pass")?;

    let client = BitcoinRpcClient::new(
        rpc_host,
        Some(rpc_user),
        Some(rpc_pass),
        3,    // Max retries
        10,   // Connection timeout
        1000, // Read/write timeout
    );

    let hash_block_hex = hex::encode(hash_block);

    let block_height = match client.getblock(hash_block_hex) {
        Ok(block_info) => Some(
            block_info
                .height
                .try_into()
                .expect("Block height is too large for i32"),
        ),
        Err(err) => {
            None
        }
    };
    println!("Block height: {:?}", block_height.unwrap_or(0));
    Ok(block_height)
}