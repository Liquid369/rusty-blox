/// PIVX Data Copy Utilities
/// 
/// Safely copies PIVX Core data files to avoid read locks when the daemon is running.
/// This allows us to read block index, chainstate, and blk files without conflicts.

use std::path::{Path, PathBuf};
use std::fs;
use std::io;

/// Copy a directory recursively
pub fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> io::Result<()> {
    let src = src.as_ref();
    let dst = dst.as_ref();
    
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }
    
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let file_name = entry.file_name();
        let dst_path = dst.join(&file_name);
        
        if ty.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            // Skip lock files
            if let Some(name) = file_name.to_str() {
                if name == "LOCK" || name.ends_with(".lock") {
                    println!("  Skipping lock file: {}", name);
                    continue;
                }
            }
            
            println!("  Copying: {} -> {}", src_path.display(), dst_path.display());
            fs::copy(&src_path, &dst_path)?;
        }
    }
    
    Ok(())
}

/// Copy PIVX block index to temporary directory
pub fn copy_block_index(
    pivx_blocks_dir: &str,
    dest_dir: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let src_index = PathBuf::from(pivx_blocks_dir).join("index");
    let dest = PathBuf::from(dest_dir);
    
    println!("📋 Copying PIVX block index...");
    println!("  From: {}", src_index.display());
    println!("  To: {}", dest.display());
    
    // Remove old copy if exists
    if dest.exists() {
        println!("  Removing old copy...");
        fs::remove_dir_all(&dest)?;
    }
    
    // Create destination
    fs::create_dir_all(&dest)?;
    
    // Copy the index directory
    copy_dir_all(&src_index, &dest)?;
    
    println!("✅ Block index copied successfully");
    
    Ok(dest)
}

/// Copy PIVX chainstate to temporary directory
pub fn copy_chainstate(
    pivx_data_dir: &str,
    dest_dir: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let src_chainstate = PathBuf::from(pivx_data_dir).join("chainstate");
    let dest = PathBuf::from(dest_dir);
    
    println!("📋 Copying PIVX chainstate...");
    println!("  From: {}", src_chainstate.display());
    println!("  To: {}", dest.display());
    
    // Remove old copy if exists
    if dest.exists() {
        println!("  Removing old copy...");
        fs::remove_dir_all(&dest)?;
    }
    
    // Create destination
    fs::create_dir_all(&dest)?;
    
    // Copy the chainstate directory
    copy_dir_all(&src_chainstate, &dest)?;
    
    println!("✅ Chainstate copied successfully");
    
    Ok(dest)
}

/// Copy blk*.dat files to temporary directory (optional - can be slow)
pub fn copy_blk_files(
    pivx_blocks_dir: &str,
    dest_dir: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let src = PathBuf::from(pivx_blocks_dir);
    let dest = PathBuf::from(dest_dir);
    
    println!("📋 Copying blk*.dat files...");
    println!("  From: {}", src.display());
    println!("  To: {}", dest.display());
    
    // Remove old copy if exists
    if dest.exists() {
        println!("  Removing old copy...");
        fs::remove_dir_all(&dest)?;
    }
    
    // Create destination
    fs::create_dir_all(&dest)?;
    
    // Copy only blk*.dat files
    for entry in fs::read_dir(&src)? {
        let entry = entry?;
        let path = entry.path();
        
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with("blk") && name.ends_with(".dat") {
                let dest_path = dest.join(name);
                println!("  Copying: {}", name);
                fs::copy(&path, &dest_path)?;
            }
        }
    }
    
    println!("✅ Blk files copied successfully");
    
    Ok(dest)
}

/// Get the block index path, copying if needed
pub fn get_block_index_path(
    pivx_blocks_dir: &str,
    copy_dir: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(dest) = copy_dir {
        if !dest.is_empty() {
            let copied = copy_block_index(pivx_blocks_dir, dest)?;
            return Ok(copied.to_string_lossy().to_string());
        }
    }
    
    // Use original path
    let path = PathBuf::from(pivx_blocks_dir).join("index");
    Ok(path.to_string_lossy().to_string())
}

/// Get the chainstate path, copying if needed
pub fn get_chainstate_path(
    pivx_data_dir: &str,
    copy_dir: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(dest) = copy_dir {
        if !dest.is_empty() {
            let copied = copy_chainstate(pivx_data_dir, dest)?;
            return Ok(copied.to_string_lossy().to_string());
        }
    }
    
    // Use original path
    let path = PathBuf::from(pivx_data_dir).join("chainstate");
    Ok(path.to_string_lossy().to_string())
}

/// Get the blk directory path, copying if needed
pub fn get_blk_dir_path(
    pivx_blocks_dir: &str,
    copy_dir: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(dest) = copy_dir {
        if !dest.is_empty() {
            let copied = copy_blk_files(pivx_blocks_dir, dest)?;
            return Ok(copied.to_string_lossy().to_string());
        }
    }
    
    // Use original path
    Ok(pivx_blocks_dir.to_string())
}
