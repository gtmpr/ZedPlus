use anyhow::{Context, Result};
use keyring::Entry;

const SERVICE: &str = "zedplus";

pub fn store_secret(key: &str, value: &str) -> Result<()> {
    Entry::new(SERVICE, key)
        .context("Failed to create keyring entry")?
        .set_password(value)
        .with_context(|| format!("Failed to store secret '{key}'"))
}

pub fn get_secret(key: &str) -> Result<Option<String>> {
    match Entry::new(SERVICE, key).context("Failed to create keyring entry")?.get_password() {
        Ok(val) => Ok(Some(val)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e).with_context(|| format!("Failed to retrieve secret '{key}'")),
    }
}

pub fn delete_secret(key: &str) -> Result<()> {
    match Entry::new(SERVICE, key).context("Failed to create keyring entry")?.delete_password() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e).with_context(|| format!("Failed to delete secret '{key}'")),
    }
}

// Canonical key names for each provider
pub fn api_key_name(provider: &str) -> String {
    format!("api_key_{provider}")
}

pub fn oauth_token_name(provider: &str) -> String {
    format!("oauth_access_{provider}")
}

pub fn oauth_refresh_name(provider: &str) -> String {
    format!("oauth_refresh_{provider}")
}
