pub use config::Config;
pub use once_cell::sync::OnceCell;
use std::error::Error;

static GLOBAL_CONFIG: OnceCell<Config> = OnceCell::new();

pub fn init_global_config() -> Result<(), Box<dyn Error>> {
    let config = Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .build()?;
    GLOBAL_CONFIG
        .set(config)
        .map_err(|_| "Config already set")?;
    Ok(())
}

pub fn get_global_config() -> &'static Config {
    GLOBAL_CONFIG.get().unwrap_or_else(|| {
        eprintln!("FATAL: Config not initialized - call init_global_config() first");
        std::process::exit(1);
    })
}

/// Load config for standalone binaries/utilities
pub fn load_config() -> Result<Config, Box<dyn Error>> {
    Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .build()
        .map_err(|e| Box::new(e) as Box<dyn Error>)
}

/// Conventional PIVX data directory for the current platform, used as the
/// fallback when `paths.pivx_data_dir` is not set in config. A real deployment
/// should set it explicitly; this just mirrors PIVX Core's own default:
///   - Linux / other: `$HOME/.pivx`   (e.g. `/home/rustyblox/.pivx`, `/root/.pivx`)
///   - macOS:         `$HOME/Library/Application Support/PIVX`
///
/// Keyed off `$HOME` so it follows whichever user the process runs as. This
/// replaced hardcoded macOS-only paths that broke height resolution on Linux /
/// in containers (the copy failed -> silent repair fallback -> inflated balances).
pub fn default_pivx_data_dir() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    if cfg!(target_os = "macos") {
        format!("{home}/Library/Application Support/PIVX")
    } else {
        format!("{home}/.pivx")
    }
}

/// Get db_path from config
pub fn get_db_path(config: &Config) -> Result<String, Box<dyn Error>> {
    config
        .get_string("paths.db_path")
        .map_err(|e| format!("Missing paths.db_path in config: {e}").into())
}
