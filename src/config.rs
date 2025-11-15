pub use config::{Config, File as ConfigFile};
pub use once_cell::sync::OnceCell;
use std::error::Error;

static GLOBAL_CONFIG: OnceCell<Config> = OnceCell::new();

pub fn init_global_config() -> Result<(), Box<dyn Error>> {
    let mut config = Config::default();
    config.merge(ConfigFile::with_name("config.toml"))?;
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
    let mut config = Config::default();
    config.merge(ConfigFile::with_name("config.toml"))?;
    Ok(config)
}

/// Get db_path from config
pub fn get_db_path(config: &Config) -> Result<String, Box<dyn Error>> {
    config
        .get_string("paths.db_path")
        .map_err(|e| format!("Missing paths.db_path in config: {}", e).into())
}
