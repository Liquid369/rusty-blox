use sha2::{Sha256, Digest};
use tokio::io::{AsyncReadExt, BufReader};
use tokio::fs::File;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = File::open("BLK_FILE_PATH_PUT_HERE").await?;
    let mut reader = BufReader::new(file);
    
    // Read magic
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic).await?;
    println!("Magic: {}", hex::encode(&magic));
    
    // Read size
    let mut size_buf = [0u8; 4];
    reader.read_exact(&mut size_buf).await?;
    let size = u32::from_le_bytes(size_buf);
    println!("Size: {}", size);
    
    // Read version
    let mut version_bytes = [0u8; 4];
    reader.read_exact(&mut version_bytes).await?;
    let version = u32::from_le_bytes(version_bytes);
    println!("Version: {}", version);
    
    // Read rest of header (80 total - 4 for version = 76)
    let mut header = vec![0u8; 80];
    header[..4].copy_from_slice(&version_bytes);
    reader.read_exact(&mut header[4..]).await?;
    
    println!("Full header (80 bytes): {}", hex::encode(&header));
    
    // Hash it
    let first = Sha256::digest(&header);
    let second = Sha256::digest(&first);
    
    println!("Hash (internal): {}", hex::encode(&second));
    
    Ok(())
}
