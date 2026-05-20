use anyhow::{Context, Result};
use std::path::PathBuf;

pub fn config_dir() -> Result<PathBuf> {
    dirs::config_dir()
        .map(|p| p.join("zedplus"))
        .context("Could not determine config directory")
}

pub fn data_dir() -> Result<PathBuf> {
    dirs::data_local_dir()
        .map(|p| p.join("zedplus"))
        .context("Could not determine data directory")
}

pub fn global_config_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

pub fn costs_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("costs.toml"))
}

pub fn models_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("models.toml"))
}

pub fn skills_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("skills"))
}

pub fn db_file() -> Result<PathBuf> {
    Ok(data_dir()?.join("zedplus.db"))
}

pub fn distill_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("distill"))
}

pub fn bench_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("bench"))
}

pub fn train_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("train"))
}

pub fn quota_cache_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("quota_cache.json"))
}

pub fn ensure_dirs() -> Result<()> {
    let dirs = [config_dir()?, data_dir()?, distill_dir()?, bench_dir()?, train_dir()?, skills_dir()?];
    for dir in &dirs {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("Failed to create directory: {}", dir.display()))?;
    }
    Ok(())
}
