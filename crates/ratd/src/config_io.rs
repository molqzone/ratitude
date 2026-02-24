use anyhow::{Context, Result};
use rat_config::{ConfigStore, RatitudeConfig};

pub(crate) async fn load_config(config_path: &str) -> Result<RatitudeConfig> {
    let path = config_path.to_string();
    tokio::task::spawn_blocking(move || -> Result<RatitudeConfig> {
        Ok(ConfigStore::new(path).load()?)
    })
    .await
    .context("failed to join config load task")?
}

pub(crate) async fn save_config(config_path: &str, config: &RatitudeConfig) -> Result<()> {
    let path = config_path.to_string();
    let config = config.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        ConfigStore::new(path).save(&config)?;
        Ok(())
    })
    .await
    .context("failed to join config save task")?
}
