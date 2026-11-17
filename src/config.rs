use crate::{identifier, read_json, version};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub executable: PathBuf,
    pub expected_sha256: Option<String>,
    #[serde(default)]
    pub config: Value,
    #[serde(default)]
    pub env_allowlist: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LifecycleOwner {
    Terraform,
    Ansible,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub inventory: PathBuf,
    pub limit: String,
    pub vars_file: Option<PathBuf>,
    pub lifecycle_owner: LifecycleOwner,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Execution {
    #[serde(default)]
    pub allow_execution: bool,
    #[serde(default)]
    pub allow_mutations: bool,
    pub ansible_executable: Option<PathBuf>,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub env_allowlist: Vec<String>,
}
fn default_timeout() -> u64 {
    300
}
impl Default for Execution {
    fn default() -> Self {
        Self {
            allow_execution: false,
            allow_mutations: false,
            ansible_executable: None,
            timeout_seconds: default_timeout(),
            env_allowlist: vec![],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    pub schema_version: u32,
    #[serde(default)]
    pub providers: BTreeMap<String, Provider>,
    #[serde(default)]
    pub targets: BTreeMap<String, Target>,
    #[serde(default)]
    pub packages: BTreeMap<String, PathBuf>,
    #[serde(default)]
    pub execution: Execution,
    #[serde(skip)]
    pub path: PathBuf,
}

impl Environment {
    pub fn load(path: &Path) -> Result<Self> {
        let mut environment: Self = read_json(path)?;
        version(environment.schema_version)?;
        environment.path = path.canonicalize().context("cannot resolve environment file")?;
        for name in environment
            .providers
            .keys()
            .chain(environment.targets.keys())
            .chain(environment.packages.keys())
        {
            identifier(name)?;
        }
        for target in environment.targets.values() {
            identifier(&target.limit)?;
            if target.limit.eq_ignore_ascii_case("all") || target.limit.eq_ignore_ascii_case("ungrouped") {
                bail!("target limit must select a single explicit host or group, not all");
            }
        }
        if !(1..=3600).contains(&environment.execution.timeout_seconds) {
            bail!("execution timeout must be 1..3600 seconds");
        }
        for names in environment
            .providers
            .values()
            .map(|p| &p.env_allowlist)
            .chain(std::iter::once(&environment.execution.env_allowlist))
        {
            for name in names {
                validate_env_name(name)?;
            }
        }
        Ok(environment)
    }

    pub fn resolve(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_owned()
        } else {
            self.path
                .parent()
                .expect("canonical environment has parent")
                .join(path)
        }
    }
}

pub(crate) fn validate_env_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
        || name.as_bytes()[0].is_ascii_digit()
    {
        bail!("invalid environment allowlist entry");
    }
    // Tool/code loading and command lookup are owned by the runner, not inherited.
    if name == "PATH"
        || name.starts_with("LD_")
        || name.starts_with("DYLD_")
        || name.starts_with("PYTHON")
        || name.starts_with("ANSIBLE_")
        || name == "BASH_ENV"
        || name == "ENV"
    {
        bail!("unsafe environment allowlist entry");
    }
    Ok(())
}
