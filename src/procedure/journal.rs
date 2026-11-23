//! Local durable intent only; no mutation dispatcher or distributed coordinator.
use super::sha256;
use crate::version;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{ErrorKind, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Journal {
    root: PathBuf,
}

/// Holds the stable file inode locked until drop. Never unlink a lock file.
pub struct SubjectGuard {
    root: PathBuf,
    subject_key: String,
    _lock: fs::File,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub document_type: String,
    pub schema_version: u32,
    pub plan_digest: String,
    pub subject_key: String,
    pub consumed_at_unix_ms: u64,
}

fn private_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        bail!("procedure journal requires an owned private directory");
    }
    Ok(())
}

impl Journal {
    /// Root must already be private and owned. All environments that should
    /// coordinate locally must explicitly select this same state directory.
    pub fn open(root: &Path) -> Result<Self> {
        private_directory(root)?;
        let root = root.canonicalize()?;
        for name in ["locks", "intents"] {
            let path = root.join(name);
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            };
            private_directory(&path)?;
        }
        fs::File::open(&root)?.sync_all()?;
        if let Some(parent) = root.parent() {
            fs::File::open(parent)?.sync_all()?;
        }
        Ok(Self { root })
    }

    pub fn lock(&self, subject_key: &str) -> Result<SubjectGuard> {
        sha256(subject_key)?;
        private_directory(&self.root.join("locks"))?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.root.join("locks").join(format!("{subject_key}.lock")))
            .context("cannot open procedure subject lock")?;
        let metadata = lock.metadata()?;
        if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0
        {
            bail!("procedure subject lock must be an owned private regular file");
        }
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            bail!("another local procedure owns this physical subject");
        }
        Ok(SubjectGuard {
            root: self.root.clone(),
            subject_key: subject_key.into(),
            _lock: lock,
        })
    }

    /// Any existing entry, even partial JSON or a dangling symlink, forbids
    /// redispatch. Missing final evidence is uncertainty, never permission.
    pub fn consumed(&self, plan_digest: &str) -> Result<bool> {
        sha256(plan_digest)?;
        private_directory(&self.root.join("intents"))?;
        match fs::symlink_metadata(self.root.join("intents").join(format!("{plan_digest}.json"))) {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
}

impl SubjectGuard {
    /// Call only after approved bindings and fresh preflight, immediately before
    /// dispatch. Return errors without dispatching; partial intent stays consumed.
    pub fn consume(&self, plan_digest: &str) -> Result<Intent> {
        sha256(plan_digest)?;
        let directory = self.root.join("intents");
        private_directory(&directory)?;
        let intent = Intent {
            document_type: "cmdmesh.procedure.intent".into(),
            schema_version: 1,
            plan_digest: plan_digest.into(),
            subject_key: self.subject_key.clone(),
            consumed_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)?
                .as_millis()
                .try_into()?,
        };
        version(intent.schema_version)?;
        let bytes = serde_json::to_vec_pretty(&intent)?;
        // Leave an incomplete entry after any write/sync failure. Dispatch has
        // not happened; retained uncertainty is safer than permission to replay.
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(directory.join(format!("{plan_digest}.json")))
            .context("procedure plan is already consumed or intent cannot be created")?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::File::open(&directory)?.sync_all()?;
        Ok(intent)
    }
}
