use sha2::{Sha256, Digest};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut file = File::open("BLK_FILE_PATH_PUT_HERE")?;
    
    // Skip magic (4) + size (4) = 8 bytes
    file.seek(SeekFrom::Start(8))?;
    
    // Read 80 bytes for version 1 header
    let mut header = vec![0u8; 80];
    file.read_exact(&mut header)?;
    
    println!("Header bytes: {}", hex::encode(&header));
    
    // Compute SHA256d
    let first_hash = Sha256::digest(&header);
    let second_hash = Sha256::digest(&first_hash);
    
    println!("Hash (internal byte order): {}", hex::encode(&second_hash));
    
    // Reverse for display (big-endian)
    let reversed: Vec<u8> = second_hash.iter().rev().cloned().collect();
    println!("Hash (display format): {}", hex::encode(&reversed));
    
    Ok(())
}
