//! Shared operations for the CLI and read-only MCP adapter.
pub mod config;
pub mod inspection;
pub mod inventory;
pub mod mcp;
pub mod operations;
mod process;
pub mod recipe;

use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&read_regular_file(path)?).context("invalid JSON input")
}

pub(crate) fn read_regular_file(path: &Path) -> Result<Vec<u8>> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .context("cannot open JSON input")?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        bail!("JSON input must be a regular file");
    }
    if metadata.len() > MAX_FILE_BYTES {
        bail!("JSON input exceeds size limit");
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        bail!("JSON input exceeds size limit");
    }
    Ok(bytes)
}

pub fn write_private_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .context("cannot create private output; existing files are never overwritten")?;
    let result = (|| -> Result<()> {
        serde_json::to_writer_pretty(&mut file, value)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
pub(crate) fn file_digest(path: &Path) -> Result<String> {
    Ok(digest(&read_regular_file(path)?))
}

pub(crate) fn version(version: u32) -> Result<()> {
    if version != 1 {
        bail!("unsupported schema_version; expected 1");
    }
    Ok(())
}

pub(crate) fn identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
    {
        bail!("invalid identifier");
    }
    Ok(())
}
