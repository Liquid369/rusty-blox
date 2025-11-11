use std::path::PathBuf;
use std::sync::Arc;
use rocksdb::DB;
use tokio::sync::Semaphore;
use crate::types::AppState;
use crate::blocks::process_blk_file;
use crate::db_utils::save_file_as_incomplete;

/// Process multiple block files in parallel with controlled concurrency
/// 
/// Architecture:
/// - Uses tokio tasks with semaphore to limit concurrent processing
/// - Each file is processed on the tokio runtime
/// - Database writes are batched within each file processor
pub async fn process_files_parallel(
    entries: Vec<PathBuf>,
    db_arc: Arc<DB>,
    state: AppState,
    max_concurrent: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    
    println!("Starting parallel file processing with {} workers", max_concurrent);
    
    // Filter for .dat files
    let blk_files: Vec<_> = entries
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("blk") && n.ends_with(".dat"))
                .unwrap_or(false)
        })
        .collect();
    
    println!("Found {} block files to process", blk_files.len());
    
    // Semaphore to limit concurrent file processing
    let semaphore = Arc::new(Semaphore::new(max_concurrent));
    
    // Process files with controlled concurrency
    let tasks: Vec<_> = blk_files
        .into_iter()
        .map(|file_path| {
            let sem = semaphore.clone();
            let db = db_arc.clone();
            let st = state.clone();
            
            async move {
                // Acquire permit
                let _permit = sem.acquire().await.unwrap();
                
                // Process file (this is async but not Send, so we run it directly)
                if let Err(e) = process_blk_file(st, file_path.clone(), db.clone()).await {
                    eprintln!("Failed to process {}: {}", file_path.display(), e);
                    let _ = save_file_as_incomplete(&db, &file_path).await;
                }
            }
        })
        .collect();
    
    // Execute all tasks concurrently
    futures::future::join_all(tasks).await;
    
    println!("All files processed!");
    
    Ok(())
}
