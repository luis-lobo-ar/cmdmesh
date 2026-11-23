//! Fixed memory-profile boundaries. No procedure execution surface yet.
mod journal;
use crate::{
    config::{Environment, LifecycleOwner, Target},
    digest, file_digest, recipe, version, write_private_json,
};
use anyhow::{bail, Context, Result};
pub use journal::{Intent, Journal, SubjectGuard};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

const PROFILE: &str = "proxmox-lxc-memory";
const PLUGINS: &str = "ansible/ansible_collections/cmdmesh/proxmox/plugins/";
const FILES: [&str; 3] = [
    "modules/lxc_memory.py",
    "module_utils/memory_state.py",
    "module_utils/memory_remote.py",
];
const COMMON_CHECKS: [&str; 7] = [
    "host-guest-identity",
    "ownership-generation",
    "host-boot-continuity",
    "runtime-continuity",
    "unchanged-state",
    "fresh-complete",
    "configuration-ready",
];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Transport {
    pub policy: String,
    pub ssh_executable: PathBuf,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub identity_file: PathBuf,
    pub known_hosts_file: PathBuf,
    pub python_executable: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Driver {
    pub executable: PathBuf,
    pub executable_sha256: String,
    pub version: String,
    pub transport: Transport,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub authenticated_current: bool,
    pub observation: Value,
    pub source: Value,
}

/// Accepted only from the configured pinned inspection transport, never as live
/// caller evidence. Parsing checks the composition contract, not native facts.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Assessment {
    pub document_type: String,
    pub schema_version: u32,
    pub profile_id: String,
    pub profile_version: u32,
    pub phase: String,
    pub subject_key: String,
    pub prepared_sha256: String,
    pub prepared: Value,
    pub checks: BTreeMap<String, bool>,
    pub evidence: Evidence,
    pub driver: Option<Driver>,
    pub profile_files: BTreeMap<String, String>,
    pub references: BTreeMap<String, Reference>,
    pub mutation_authorized: bool,
}

pub fn fingerprint(path: &Path) -> Result<String> {
    file_digest(path)
}
pub fn json_fingerprint(value: &Value) -> Result<String> {
    Ok(digest(&serde_json::to_vec(value)?))
}

fn sha256(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        bail!("invalid procedure SHA-256");
    }
    Ok(())
}

impl Assessment {
    pub fn parse(value: Value, phase: &str) -> Result<Self> {
        let result: Self = serde_json::from_value(value).context("invalid procedure assessment shape")?;
        result.validate(phase)?;
        Ok(result)
    }

    pub fn satisfied(&self) -> bool {
        self.checks.values().all(|v| *v)
    }

    fn validate(&self, phase: &str) -> Result<()> {
        version(self.schema_version)?;
        if self.document_type != "cmdmesh.procedure.assessment"
            || self.profile_id != PROFILE
            || self.profile_version != 1
            || self.phase != phase
            || !["prepare", "preflight", "verify"].contains(&phase)
            || self.mutation_authorized
        {
            bail!("unsupported procedure assessment");
        }
        sha256(&self.subject_key)?;
        sha256(&self.prepared_sha256)?;
        if !self.prepared.is_object()
            || self.prepared["document_type"] != "cmdmesh.proxmox.lxc-memory.prepared"
            || self.prepared["schema_version"] != 1
            || self.prepared["subject_key"] != self.subject_key
            || !self.prepared["change_required"].is_boolean()
            || json_fingerprint(&self.prepared)? != self.prepared_sha256
        {
            bail!("procedure prepared intent does not match assessment");
        }
        if !self.evidence.authenticated_current
            || !self.evidence.observation.is_object()
            || !self.evidence.source.is_object()
        {
            bail!("procedure assessment lacks authenticated current evidence");
        }
        let mut expected: BTreeSet<&str> = COMMON_CHECKS.into_iter().collect();
        expected.extend(if phase == "verify" {
            vec![
                "configuration-target",
                "enforced-memory-target",
                "configuration-digest-transition",
            ]
        } else {
            vec!["configuration-preimage", "capacity-headroom"]
        });
        if self.checks.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected
            || (phase != "verify" && !self.satisfied())
        {
            bail!("procedure required checks are missing, unknown or false");
        }
        let files: BTreeSet<String> = FILES.into_iter().map(|name| format!("{PLUGINS}{name}")).collect();
        if self.profile_files.keys().cloned().collect::<BTreeSet<_>>() != files {
            bail!("procedure driver profile files mismatch");
        }
        for hash in self.profile_files.values() {
            sha256(hash)?;
        }
        let mut refs: BTreeSet<&str> = ["tls-ca", "ssh-executable", "ssh-identity", "ssh-known-hosts"]
            .into_iter()
            .collect();
        if phase != "verify" {
            refs.insert("ansible-executable");
        }
        if self
            .references
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            != refs
        {
            bail!("procedure reference roles mismatch");
        }
        for reference in self.references.values() {
            if !reference.path.is_absolute() {
                bail!("procedure references require explicit absolute paths");
            }
            sha256(&reference.sha256)?;
        }
        if phase == "verify" {
            if self.driver.is_some() {
                bail!("verification must not require the mutation driver");
            }
        } else {
            let driver = self.driver.as_ref().context("procedure driver context missing")?;
            let transport = &driver.transport;
            if transport.policy != "strict-direct-ssh-v1"
                || transport.user != "root"
                || transport.python_executable != Path::new("/usr/bin/python3")
                || transport.port == 0
                || transport.host.is_empty()
                || transport.host.len() > 253
                || transport.host.starts_with('-')
                || !transport
                    .host
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._:-".contains(&c))
            {
                bail!("unsupported procedure SSH transport");
            }
            let parts = driver
                .version
                .split('.')
                .map(str::parse::<u32>)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if parts.len() != 3 || parts[0] != 2 || !(parts[1] == 20 || (parts[1] == 19 && parts[2] >= 4)) {
                bail!("unsupported procedure Ansible version");
            }
            for (role, path) in [
                ("ansible-executable", &driver.executable),
                ("ssh-executable", &transport.ssh_executable),
                ("ssh-identity", &transport.identity_file),
                ("ssh-known-hosts", &transport.known_hosts_file),
            ] {
                if self.references[role].path != *path {
                    bail!("procedure transport/reference mismatch");
                }
            }
            if self.references["ansible-executable"].sha256 != driver.executable_sha256 {
                bail!("procedure executable/reference hash mismatch");
            }
            for path in [&transport.identity_file, &transport.known_hosts_file] {
                if path
                    .to_string_lossy()
                    .chars()
                    .any(|c| c.is_whitespace() || "\"'\\%$~".contains(c))
                {
                    bail!("unsupported procedure SSH reference path");
                }
            }
        }
        Ok(())
    }

    pub fn verify_references(&self) -> Result<()> {
        for reference in self.references.values() {
            if file_digest(&reference.path)? != reference.sha256 {
                bail!("procedure referenced file changed");
            }
        }
        Ok(())
    }
}

/// Build only trusted fixed inputs. Legacy target inventory/vars are ignored.
/// The caller owns the empty private context directory and its retained lifetime.
pub fn prepare_context(
    environment: &Environment,
    assessment: &Assessment,
    context: &Path,
) -> Result<recipe::Plan> {
    assessment.validate("prepare")?;
    assessment.verify_references()?;
    let driver = assessment.driver.as_ref().context("procedure driver missing")?;
    if recipe::resolve_ansible(environment)? != driver.executable.canonicalize()? {
        bail!("procedure driver differs from environment-selected Ansible");
    }
    let package = environment.resolve(
        environment
            .packages
            .get("cmdmesh-proxmox")
            .context("procedure requires cmdmesh-proxmox package")?,
    );
    if fs::symlink_metadata(&package)?.file_type().is_symlink() {
        bail!("procedure package cannot be a symlink");
    }
    let package = package.canonicalize()?;
    let mut captured = BTreeMap::new();
    for (relative, expected) in &assessment.profile_files {
        let file = recipe::confined_file(&package, Path::new(relative))?;
        let bytes = crate::read_regular_file(&file)?;
        if digest(&bytes) != *expected {
            bail!("procedure collection differs from pinned inspector driver profile");
        }
        captured.insert(relative, bytes);
    }
    let metadata = fs::symlink_metadata(context)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        bail!("procedure context must be an owned private directory");
    }
    let context = context.canonicalize()?;
    if context.starts_with(&package) || fs::read_dir(&context)?.next().is_some() {
        bail!("procedure context must be empty and outside the package tree");
    }
    // A matching module can still be intercepted by an action plugin or routing
    // metadata in the original collection. Load only these captured driver bytes.
    let frozen_package = context.join("driver");
    for (relative, bytes) in captured {
        let destination = frozen_package.join(relative);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(destination.parent().unwrap())?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(destination)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    let transport = &driver.transport;
    let options = [
        "BatchMode=yes".to_owned(),
        "StrictHostKeyChecking=yes".into(),
        "IdentitiesOnly=yes".into(),
        "IdentityAgent=none".into(),
        "CertificateFile=none".into(),
        "PreferredAuthentications=publickey".into(),
        "ForwardAgent=no".into(),
        "ControlMaster=no".into(),
        "ControlPath=none".into(),
        "ControlPersist=no".into(),
        "GlobalKnownHostsFile=/dev/null".into(),
        format!("UserKnownHostsFile={}", transport.known_hosts_file.display()),
        "UpdateHostKeys=no".into(),
        "ProxyCommand=none".into(),
        "ProxyJump=none".into(),
        "ClearAllForwardings=yes".into(),
        "RequestTTY=no".into(),
        "NumberOfPasswordPrompts=0".into(),
        "ConnectionAttempts=1".into(),
        "ConnectTimeout=2".into(),
        "LogLevel=ERROR".into(),
    ];
    let common_args = format!(
        "-F none {}",
        options
            .iter()
            .map(|s| format!("-o {s}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    write_private_json(
        &context.join("inventory.json"),
        &json!({"all":{"hosts":{"cmdmesh_subject":{
            "ansible_host":transport.host,"ansible_port":transport.port,"ansible_user":"root",
            "ansible_connection":"ssh","ansible_become":false,"ansible_python_interpreter":"/usr/bin/python3",
            "ansible_ssh_executable":transport.ssh_executable,"ansible_private_key_file":transport.identity_file,
            "ansible_ssh_common_args":common_args,"ansible_ssh_transfer_method":"piped","ansible_pipelining":true
        }}}}),
    )?;
    write_private_json(
        &context.join("vars.json"),
        &json!({"cmdmesh_memory_prepared":assessment.prepared}),
    )?;
    let recipe_dir = context.join("recipe");
    fs::DirBuilder::new().mode(0o700).create(&recipe_dir)?;
    write_private_json(
        &recipe_dir.join("playbook.json"),
        &json!([{"name":"Apply the frozen owned LXC memory intent","hosts":"cmdmesh_subject","gather_facts":false,"become":false,"tasks":[{"name":"Check identity and grow only memory","cmdmesh.proxmox.lxc_memory":{"prepared":"{{ cmdmesh_memory_prepared }}","allow_changes":"{{ cmdmesh_allow_changes | bool }}"}}]}]),
    )?;
    write_private_json(
        &recipe_dir.join("recovery.json"),
        &json!({"recovery":"Inspect actual state after failure. Never automatically replay, shrink, restart or roll back."}),
    )?;
    write_private_json(
        &recipe_dir.join("recipe.json"),
        &recipe::Recipe {
            schema_version: 1,
            id: "owned-lxc-memory".into(),
            title: "Owned LXC memory".into(),
            summary: "One independently guarded memory-only change".into(),
            status: recipe::Status::Ready,
            requirements: vec!["lifecycle-owner:ansible".into()],
            dependencies: vec!["cmdmesh-proxmox".into()],
            steps: vec![recipe::Step {
                id: "grow-memory".into(),
                driver: recipe::Driver::Ansible,
                playbook: "playbook.json".into(),
            }],
            verification: vec!["Independent live memory profile postconditions are required".into()],
            recovery: "recovery.json".into(),
        },
    )?;
    let derived = Environment {
        schema_version: 1,
        providers: BTreeMap::new(),
        targets: BTreeMap::from([(
            "subject".into(),
            Target {
                inventory: context.join("inventory.json"),
                limit: "cmdmesh_subject".into(),
                vars_file: Some(context.join("vars.json")),
                lifecycle_owner: LifecycleOwner::Ansible,
            },
        )]),
        packages: BTreeMap::from([("cmdmesh-proxmox".into(), frozen_package)]),
        execution: crate::config::Execution {
            ansible_executable: Some(driver.executable.clone()),
            ..environment.execution.clone()
        },
        path: PathBuf::new(),
    };
    write_private_json(&context.join("environment.json"), &derived)?;
    let derived = Environment::load(&context.join("environment.json"))?;
    recipe::plan(
        &derived,
        &recipe_dir.join("recipe.json"),
        "subject",
        recipe::Operation::Apply,
    )
}
