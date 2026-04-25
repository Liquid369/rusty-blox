/// Telemetry Module - Structured Logging with Tracing
/// 
/// Implements LOGGING_GUIDE.md requirements:
/// - Structured logging with tracing
/// - JSON vs pretty format support
/// - File logging with daily rotation
/// - RUST_LOG env var support
/// - Sampling and truncation helpers

use std::sync::atomic::{AtomicU64, Ordering};
use tracing_subscriber::{fmt, EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
use tracing_appender::rolling;

/// Telemetry configuration
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    /// Log level: "trace", "debug", "info", "warn", "error"
    pub log_level: String,
    /// Log format: "json" or "pretty"
    pub log_format: String,
    /// Optional log file path (None = console only)
    pub log_file: Option<String>,
    /// Rotation interval: "daily", "hourly", "never"
    pub rotation: String,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            log_level: std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
            log_format: std::env::var("RUSTYBLOX_LOG_FORMAT").unwrap_or_else(|_| "pretty".to_string()),
            log_file: std::env::var("RUSTYBLOX_LOG_FILE").ok(),
            rotation: "daily".to_string(),
        }
    }
}

/// Initialize tracing subscriber
/// 
/// Sets up structured logging based on configuration:
/// - Reads RUST_LOG env var (default: info)
/// - Supports JSON vs pretty format
/// - Optional file logging with rotation
/// 
/// LOGGING_GUIDE.md Section 5: Configuration
pub fn init_tracing(config: TelemetryConfig) -> Result<(), Box<dyn std::error::Error>> {
    // Build env filter from RUST_LOG or config
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.log_level));
    
    // Determine if we're writing to file
    if let Some(log_file_path) = config.log_file {
        // Parse directory and filename
        let path = std::path::Path::new(&log_file_path);
        let directory = path.parent()
            .ok_or("Invalid log file path: no parent directory")?;
        let filename_prefix = path.file_stem()
            .and_then(|s| s.to_str())
            .ok_or("Invalid log file path: no filename")?;
        
        // Create rotating file appender
        let file_appender = match config.rotation.as_str() {
            "daily" => rolling::daily(directory, filename_prefix),
            "hourly" => rolling::hourly(directory, filename_prefix),
            "never" => rolling::never(directory, path.file_name().unwrap()),
            _ => rolling::daily(directory, filename_prefix),
        };
        
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
        
        // JSON or pretty format
        if config.log_format == "json" {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(
                    fmt::layer()
                        .json()
                        .with_current_span(true)
                        .with_span_list(true)
                        .with_writer(non_blocking)
                )
                .init();
        } else {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(
                    fmt::layer()
                        .with_target(false)
                        .with_thread_ids(false)
                        .with_file(true)
                        .with_line_number(true)
                        .with_writer(non_blocking)
                )
                .init();
        }
        
        // Keep guard alive (otherwise logs won't flush)
        std::mem::forget(_guard);
    } else {
        // Console-only logging
        if config.log_format == "json" {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(
                    fmt::layer()
                        .json()
                        .with_current_span(true)
                        .with_span_list(true)
                )
                .init();
        } else {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(
                    fmt::layer()
                        .with_target(false)
                        .with_thread_ids(false)
                        .with_file(true)
                        .with_line_number(true)
                )
                .init();
        }
    }
    
    Ok(())
}

/// Truncate hex string for logging
/// 
/// LOGGING_GUIDE.md Section 4.2: Truncation Rules
/// - Transaction IDs: 16 chars
/// - Block hashes: 16 chars
/// - Script hex: 64 chars
/// 
/// Example: "0a1b2c3d4e5f67890a1b2c3d4e5f6789" → "0a1b2c3d4e5f6789..."
pub fn truncate_hex(hex: &str, len: usize) -> String {
    if hex.len() <= len {
        hex.to_string()
    } else {
        format!("{}...", &hex[..len])
    }
}

/// Truncate list for logging
/// 
/// Shows first N items, indicates total count if longer
/// 
/// Example: ["a", "b", "c", "d", "e", "f"] (max 3) → "[3 of 6]: [a, b, c]"
pub fn truncate_list<T: std::fmt::Display + std::fmt::Debug>(items: &[T], max: usize) -> String {
    if items.len() <= max {
        format!("{:?}", items)
    } else {
        let preview: Vec<String> = items.iter().take(max).map(|i| i.to_string()).collect();
        format!("[{} of {}]: {:?}", max, items.len(), preview)
    }
}

/// Sampling helper for progress logs
/// 
/// LOGGING_GUIDE.md Section 3.2: Log Sampling
/// 
/// Usage:
/// ```
/// static TX_COUNTER: AtomicU64 = AtomicU64::new(0);
/// 
/// for tx in transactions {
///     // ... process tx ...
///     
///     if should_log_progress(&TX_COUNTER, 100_000) {
///         info!(tx_scanned = processed, "Progress");
///     }
/// }
/// ```
/// 
/// Returns true every `interval` calls (e.g., every 100,000)
pub fn should_log_progress(counter: &AtomicU64, interval: u64) -> bool {
    let count = counter.fetch_add(1, Ordering::Relaxed);
    count % interval == 0
}

/// Create a sampled progress counter
/// 
/// Convenience wrapper that creates and manages counter internally
pub struct ProgressCounter {
    counter: AtomicU64,
    interval: u64,
}

impl ProgressCounter {
    pub fn new(interval: u64) -> Self {
        Self {
            counter: AtomicU64::new(0),
            interval,
        }
    }
    
    pub fn should_log(&self) -> bool {
        should_log_progress(&self.counter, self.interval)
    }
    
    pub fn get(&self) -> u64 {
        self.counter.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_truncate_hex() {
        assert_eq!(truncate_hex("abcd", 16), "abcd");
        assert_eq!(truncate_hex("0123456789abcdef0123456789abcdef", 16), "0123456789abcdef...");
        assert_eq!(truncate_hex("", 16), "");
    }
    
    #[test]
    fn test_should_log_progress() {
        let counter = AtomicU64::new(0);
        
        // First call (count=0): true (0 % 10 == 0)
        assert!(should_log_progress(&counter, 10));
        
        // Calls 1-9: false
        for _ in 1..10 {
            assert!(!should_log_progress(&counter, 10));
        }
        
        // Call 10 (count=10): true (10 % 10 == 0)
        assert!(should_log_progress(&counter, 10));
    }
    
    #[test]
    fn test_progress_counter() {
        let counter = ProgressCounter::new(5);
        
        assert!(counter.should_log());  // 0 % 5 == 0
        assert!(!counter.should_log()); // 1 % 5 != 0
        assert!(!counter.should_log()); // 2 % 5 != 0
        assert!(!counter.should_log()); // 3 % 5 != 0
        assert!(!counter.should_log()); // 4 % 5 != 0
        assert!(counter.should_log());  // 5 % 5 == 0
        
        assert_eq!(counter.get(), 6);
    }
}
