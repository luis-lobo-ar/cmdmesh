//! Generic transport for explicitly configured, pinned inspection integrations.
use crate::{config::Environment, digest, identifier, process, read_regular_file, version};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    time::Duration,
};

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Observed,
    Rejected,
    Unsupported,
    Incomplete,
    Unavailable,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub code: String,
    pub summary: String,
    pub next_checks: Vec<String>,
    pub recovery: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InspectionResult {
    pub document_type: String,
    pub schema_version: u32,
    pub operation: String,
    pub outcome: Outcome,
    pub data: Map<String, Value>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Serialize)]
pub struct Receipt {
    pub document_type: &'static str,
    pub schema_version: u32,
    pub inspector: String,
    pub executable_sha256: String,
    pub configuration_sha256: String,
    pub request_sha256: String,
    pub result: InspectionResult,
}

fn validate(result: &InspectionResult, operation: &str, exit: Option<i32>) -> Result<()> {
    version(result.schema_version)?;
    if result.document_type != "cmdmesh.inspection.result" || result.operation != operation {
        bail!("inspector response does not match the requested protocol and operation; raw output withheld");
    }
    let expected = if result.outcome == Outcome::Observed { 0 } else { 1 };
    if exit != Some(expected) {
        bail!("inspector outcome and exit status disagree; raw output withheld");
    }
    if result.diagnostics.len() > 32 || (expected == 1 && result.diagnostics.is_empty()) {
        bail!("inspector diagnostics are missing or exceed limits; raw output withheld");
    }
    for diagnostic in &result.diagnostics {
        identifier(&diagnostic.code).context("invalid inspector diagnostic code; raw output withheld")?;
        if diagnostic.summary.is_empty() || diagnostic.summary.len() > 1024 {
            bail!("inspector diagnostic summary exceeds limits; raw output withheld");
        }
        for list in [&diagnostic.next_checks, &diagnostic.recovery] {
            if list.is_empty()
                || list.len() > 16
                || list.iter().any(|text| text.is_empty() || text.len() > 2048)
            {
                bail!("inspector diagnostic guidance is missing or exceeds limits; raw output withheld");
            }
        }
    }
    Ok(())
}

pub fn run(
    environment: &Environment,
    inspector_id: &str,
    operation: &str,
    input: Value,
    timeout_seconds: u64,
) -> Result<Receipt> {
    identifier(inspector_id)?;
    identifier(operation)?;
    if !(1..=120).contains(&timeout_seconds) || !input.is_object() {
        bail!("inspection requires object input and a 1..120 second timeout");
    }
    if operation == "describe" && !input.as_object().unwrap().is_empty() {
        bail!("describe takes empty inspection input");
    }
    let provider = environment
        .providers
        .get(inspector_id)
        .context("unknown inspector provider")?;
    let expected = provider
        .expected_sha256
        .as_ref()
        .context("inspection requires an executable SHA-256 pin")?;
    if expected.len() != 64
        || !expected
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        bail!("inspection requires a lowercase executable SHA-256 pin");
    }
    if !provider.config.is_object() {
        bail!("inspection provider config must be an object");
    }
    let source = environment.resolve(&provider.executable);
    let bytes = read_regular_file(&source).context("inspector executable unavailable")?;
    if digest(&bytes) != *expected {
        bail!("inspector executable digest mismatch");
    }
    if fs::metadata(&source)?.permissions().mode() & 0o111 == 0 {
        bail!("inspector executable has no execute permission");
    }
    let parent = environment.path.parent().unwrap();
    let scratch_root = parent.join(".generated/cmdmesh-inspections");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&scratch_root)
        .context("cannot create private inspector scratch")?;
    let metadata = fs::symlink_metadata(&scratch_root)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        bail!("inspector scratch must be a private owned directory, not a symlink");
    }
    let scratch = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(scratch_root)?;
    let frozen = scratch.path().join("inspector");
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(&frozen)?
        .write_all(&bytes)?;
    let request = serde_json::to_vec(
        &json!({"document_type":"cmdmesh.inspection.request","schema_version":1,
        "operation":operation,"input":input,"config":provider.config}),
    )?;
    if request.len() > 16 * 1024 * 1024 {
        bail!("inspection request exceeds size limit");
    }
    let configuration_sha256 = digest(&serde_json::to_vec(&provider.config)?);
    let request_sha256 = digest(&request);
    let output = process::capture(process::RunSpec {
        program: &frozen,
        args: &[],
        input: Some(request),
        allowlist: &provider.env_allowlist,
        extra_env: &BTreeMap::from([("CMDMESH_INSPECTION_PROCESS_GROUP".to_owned(), "1".to_owned())]),
        cwd: parent,
        timeout: Duration::from_secs(timeout_seconds),
        private_root: parent,
        ansible_config: None,
        logs: None,
    })?;
    let result: InspectionResult = serde_json::from_slice(&output.stdout)
        .map_err(|_| anyhow::anyhow!("inspector returned invalid inspection JSON; raw output withheld"))?;
    validate(&result, operation, output.status.code())?;
    Ok(Receipt {
        document_type: "cmdmesh.inspection.receipt",
        schema_version: 1,
        inspector: inspector_id.to_owned(),
        executable_sha256: expected.clone(),
        configuration_sha256,
        request_sha256,
        result,
    })
}
