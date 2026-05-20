pub mod costs;
pub mod models;
pub mod schema;

use anyhow::Result;
use schema::Config;
use std::path::{Path, PathBuf};

pub struct LoadedConfig {
    pub config: Config,
    pub costs: costs::CostsTable,
    pub models: models::ModelRegistry,
}

pub fn load(project_root: Option<&Path>) -> Result<LoadedConfig> {
    let config_file = crate::platform::dirs::global_config_file()?;
    let global = load_toml_or_default::<Config>(&config_file)?;

    // Project config (.zedplus.toml) wins over global
    let merged = if let Some(root) = project_root {
        let project_file = root.join(".zedplus.toml");
        if project_file.exists() {
            merge(global, load_toml_or_default::<Config>(&project_file)?)
        } else {
            global
        }
    } else {
        global
    };

    let costs = costs::load_or_default(&crate::platform::dirs::costs_file()?)?;
    let models = models::load_or_default(&crate::platform::dirs::models_file()?)?;

    Ok(LoadedConfig { config: merged, costs, models })
}

fn load_toml_or_default<T: serde::de::DeserializeOwned + Default>(path: &PathBuf) -> Result<T> {
    if path.exists() {
        let raw = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&raw)?)
    } else {
        Ok(T::default())
    }
}

// Project config fields override global where set.
// Simple field-level merge: project non-default values win.
fn merge(global: Config, project: Config) -> Config {
    Config {
        privacy: if project.privacy.cloud_allowed.is_some() {
            project.privacy
        } else {
            global.privacy
        },
        routing: project.routing,
        hooks: project.hooks,
        // locale, behavior, training, sessions, testing, services, update stay global
        ..global
    }
}

pub fn write_global(config: &Config) -> Result<()> {
    let path = crate::platform::dirs::global_config_file()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = toml::to_string_pretty(config)?;
    std::fs::write(&path, raw)?;
    Ok(())
}
