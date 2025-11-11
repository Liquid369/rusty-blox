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
    GLOBAL_CONFIG.get().expect("Config not initialized")
}
