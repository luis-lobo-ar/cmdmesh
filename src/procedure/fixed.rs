//! One fixed prepare/preflight/change/verify sequence, not a workflow language.
use super::{journal::private_directory, Assessment, Journal, Reference};
use crate::{
    config::{Environment, Provider},
    digest, file_digest, identifier, inspection, read_json, read_regular_file, recipe, version,
    write_private_json,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub document_type: String,
    pub schema_version: u32,
    pub digest: String,
    pub engine_sha256: String,
    pub environment: PathBuf,
    pub inspector: String,
    pub provider: Provider,
    pub state_directory: PathBuf,
    pub context_directory: PathBuf,
    pub assessment: Assessment,
    pub bindings: Vec<Reference>,
    pub inner_plan: recipe::Plan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_source: Option<OperationSource>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationSource {
    pub id: String,
    pub document: Reference,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Parameters {
    request: Value,
    desired_memory_mb: u32,
    capacity_policy: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Planned,
    Pending,
    Checked,
    Rejected,
    Verified,
    Unverified,
    Uncertain,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CommandCompletion {
    NotDispatched,
    Succeeded,
    Failed,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandFacts {
    pub completion_scope: String,
    pub dispatch_attempted: bool,
    pub completion: CommandCompletion,
    pub exit_code: Option<i32>,
    pub completed_steps: usize,
    pub reporting_failed: bool,
    pub native_completion: CommandCompletion,
    pub native_mutation_attempted: Option<bool>,
    pub native_module_failed: Option<bool>,
}

impl Default for CommandFacts {
    fn default() -> Self {
        Self {
            completion_scope: "controlled-ansible-invocation".into(),
            dispatch_attempted: false,
            completion: CommandCompletion::NotDispatched,
            exit_code: None,
            completed_steps: 0,
            reporting_failed: false,
            native_completion: CommandCompletion::NotDispatched,
            native_mutation_attempted: None,
            native_module_failed: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DriverFacts {
    document_type: String,
    schema_version: u32,
    prepared_sha256: String,
    subject_key: String,
    command_outcome: CommandCompletion,
    mutation_attempted: bool,
    module_failed: bool,
}

fn driver_facts(plan: &Plan) -> Result<DriverFacts> {
    let path = plan.context_directory.join("driver-context/driver-facts.json");
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
    {
        bail!("native driver facts must be a private regular file");
    }
    let facts: DriverFacts = read_json(&path)?;
    version(facts.schema_version)?;
    if facts.document_type != "cmdmesh.proxmox.lxc-memory.driver-facts"
        || facts.prepared_sha256 != plan.assessment.prepared_sha256
        || facts.subject_key != plan.assessment.subject_key
        || (facts.command_outcome == CommandCompletion::NotDispatched && facts.mutation_attempted)
        || (matches!(
            facts.command_outcome,
            CommandCompletion::Succeeded | CommandCompletion::Failed
        ) && !facts.mutation_attempted)
    {
        bail!("native driver facts do not match the frozen intent");
    }
    fs::File::open(&path)?.sync_all()?;
    fs::File::open(path.parent().unwrap())?.sync_all()?;
    Ok(facts)
}

pub fn describe() -> Value {
    json!({"document_type":"cmdmesh.procedure.description","schema_version":1,
        "profile_id":"proxmox-lxc-memory","profile_version":1,
        "summary":"Grow one owned running LXC memory ceiling and independently verify actual configuration and enforced memory",
        "parameters":{"type":"object","required":["request","desired_memory_mb","capacity_policy"],"additionalProperties":false,"properties":{
            "request":{"type":"object","description":"The configured native lxc-memory-procedure prepare request: exact selector, host identity pins, ownership and generation tags; discover its full schema with inspection describe"},
            "desired_memory_mb":{"type":"integer","minimum":16,"maximum":2147483647},
            "capacity_policy":{"type":"object","required":["mode","minimum_remaining_bytes"],"additionalProperties":false,"properties":{"mode":{"const":"observed-available-headroom"},"minimum_remaining_bytes":{"type":"integer","minimum":0}}}}},
        "required_configuration":["SHA-pinned inspector supporting lxc-memory-procedure profile1","matching cmdmesh-proxmox package","explicit TLS CA and strict root SSH","existing owner-only durable state directory shared by coordinating environments"],
        "planning":"Reads current remote state and creates private frozen context/evidence; does not mutate infrastructure",
        "execution":"CLI only, reviewed digest plus original execution/mutation permissions; unchanged target needs no mutation permission",
        "outcomes":["checked","rejected","verified","unverified","uncertain"],
        "status":"Historical evidence only; planned/pending are nonterminal and do not prove current health or controller liveness",
        "recovery":"Inspect retained phase receipts and actual state before a new plan; consumed plans cannot be replayed. No automatic shrink, restart, retry or rollback."})
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub document_type: String,
    pub schema_version: u32,
    pub plan_digest: String,
    pub subject_key: String,
    pub outcome: Outcome,
    pub current_observation: bool,
    pub command: CommandFacts,
    pub postconditions_satisfied: Option<bool>,
    pub checks: Option<BTreeMap<String, bool>>,
    pub evidence_directory: PathBuf,
    pub diagnostics: Vec<String>,
}

fn engine_fingerprint() -> Result<String> {
    let mut file = fs::File::open(std::env::current_exe()?)?;
    if !file.metadata()?.is_file() {
        bail!("procedure executor is not a regular executable");
    }
    let mut hash = Sha256::new();
    let mut total = 0usize;
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count;
        if total > 256 * 1024 * 1024 {
            bail!("procedure executor exceeds fingerprint limit");
        }
        hash.update(&buffer[..count]);
    }
    Ok(hex::encode(hash.finalize()))
}

fn plan_digest(plan: &Plan) -> Result<String> {
    let mut unsigned = plan.clone();
    unsigned.digest.clear();
    Ok(digest(&serde_json::to_vec(&unsigned)?))
}
fn binding(path: &Path) -> Result<Reference> {
    Ok(Reference {
        path: path.to_owned(),
        sha256: file_digest(path)?,
    })
}
fn persist<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    write_private_json(path, value)?;
    fs::File::open(path.parent().context("evidence requires a parent")?)?.sync_all()?;
    Ok(())
}
fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn frozen_environment(plan: &Plan) -> Environment {
    Environment {
        schema_version: 1,
        providers: BTreeMap::from([(plan.inspector.clone(), plan.provider.clone())]),
        targets: BTreeMap::new(),
        packages: BTreeMap::new(),
        execution: Default::default(),
        path: plan.environment.clone(),
    }
}

fn assessment(receipt: &inspection::Receipt, phase: &str) -> Result<Assessment> {
    let result = Assessment::parse(Value::Object(receipt.result.data.clone()), phase)?;
    if (receipt.result.outcome == inspection::Outcome::Observed) != result.satisfied()
        || (!result.satisfied() && receipt.result.outcome != inspection::Outcome::Incomplete)
    {
        bail!("procedure phase outcome disagrees with required checks");
    }
    Ok(result)
}

pub fn plan(
    environment: &Environment,
    inspector: &str,
    input: Value,
    state_directory: &Path,
) -> Result<Plan> {
    plan_bound(environment, inspector, input, state_directory, None)
}

pub(crate) fn plan_bound(
    environment: &Environment,
    inspector: &str,
    input: Value,
    state_directory: &Path,
    operation_source: Option<OperationSource>,
) -> Result<Plan> {
    identifier(inspector)?;
    if let Some(source) = &operation_source {
        identifier(&source.id)?;
        if file_digest(&source.document.path)? != source.document.sha256 {
            bail!("operation metadata changed before preparation");
        }
    }
    let parameters: Parameters =
        serde_json::from_value(input).context("invalid memory procedure parameters")?;
    if !parameters.request.is_object()
        || !parameters.capacity_policy.is_object()
        || !(16..=2u32.pow(31) - 1).contains(&parameters.desired_memory_mb)
    {
        bail!("invalid memory procedure parameters");
    }
    let environment = Environment::load(&environment.path)?;
    let source_environment = binding(&environment.path)?;
    let original = environment
        .providers
        .get(inspector)
        .context("unknown procedure inspector")?;
    let source = environment.resolve(&original.executable);
    let bytes = read_regular_file(&source)?;
    if original.expected_sha256.as_deref() != Some(digest(&bytes).as_str())
        || fs::metadata(&source)?.permissions().mode() & 0o111 == 0
    {
        bail!("procedure inspector pin or execute permission mismatch");
    }
    Journal::open(state_directory)?;
    let state_directory = state_directory.canonicalize()?;
    let plans = state_directory.join("plans");
    match fs::DirBuilder::new().mode(0o700).create(&plans) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    };
    private_directory(&plans)?;
    fs::File::open(&state_directory)?.sync_all()?;
    let temporary = tempfile::Builder::new()
        .prefix("memory-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(&plans)?;
    let context = temporary.keep();
    let executable = context.join("inspector");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(&executable)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    let mut provider = original.clone();
    provider.executable = executable.clone();
    let mut frozen = environment.clone();
    frozen.providers = BTreeMap::from([(inspector.into(), provider.clone())]);
    let receipt = inspection::run(
        &frozen,
        inspector,
        "lxc-memory-procedure",
        json!({"phase":"prepare","request":parameters.request,"desired_memory_mb":parameters.desired_memory_mb,"capacity_policy":parameters.capacity_policy}),
        120,
    ).with_context(||format!("procedure preparation unavailable; diagnostic context retained at {}",context.display()))?;
    let receipt_path = context.join("prepare.json");
    persist(&receipt_path, &receipt)?;
    let assessment = assessment(&receipt, "prepare").with_context(|| {
        format!(
            "procedure preparation did not qualify; native receipt retained at {}",
            receipt_path.display()
        )
    })?;
    let mut returned_request = assessment.prepared["request"].clone();
    if parameters.request.get("request_id").is_none() {
        returned_request
            .as_object_mut()
            .context("prepared request is not an object")?
            .remove("request_id");
    }
    if returned_request != parameters.request
        || assessment.prepared["desired_memory_mb"] != parameters.desired_memory_mb
        || assessment.prepared["capacity_policy"] != parameters.capacity_policy
    {
        bail!("procedure preparation ignored requested parameters");
    }
    let inner_context = context.join("driver-context");
    fs::DirBuilder::new().mode(0o700).create(&inner_context)?;
    let inner_plan = super::prepare_context(&environment, &assessment, &inner_context)?;
    let mut bindings = vec![
        source_environment,
        Reference {
            path: source,
            sha256: digest(&bytes),
        },
        binding(&executable)?,
        binding(&context.join("prepare.json"))?,
    ];
    bindings.extend(assessment.references.values().cloned());
    if let Some(source) = &operation_source {
        bindings.push(source.document.clone());
    }
    let package = environment
        .resolve(&environment.packages["cmdmesh-proxmox"])
        .canonicalize()?;
    for (relative, expected) in &assessment.profile_files {
        let path = recipe::confined_file(&package, Path::new(relative))?;
        if file_digest(&path)? != *expected {
            bail!("procedure source driver changed during preparation");
        }
        bindings.push(Reference {
            path,
            sha256: expected.clone(),
        });
    }
    for item in &bindings {
        if file_digest(&item.path)? != item.sha256 {
            bail!("procedure input changed during preparation");
        }
    }
    let mut plan = Plan {
        document_type: "cmdmesh.procedure.plan".into(),
        schema_version: 1,
        digest: String::new(),
        engine_sha256: engine_fingerprint()?,
        environment: environment.path.clone(),
        inspector: inspector.into(),
        provider,
        state_directory,
        context_directory: context.to_owned(),
        assessment,
        bindings,
        inner_plan,
        operation_source,
    };
    plan.digest = plan_digest(&plan)?;
    persist(&context.join("plan.json"), &plan)?;
    fs::File::open(&plans)?.sync_all()?;
    Ok(plan)
}

fn validate_document(plan: &Plan) -> Result<()> {
    version(plan.schema_version)?;
    if plan.document_type != "cmdmesh.procedure.plan" || plan_digest(plan)? != plan.digest {
        bail!("procedure plan digest or document mismatch");
    }
    identifier(&plan.inspector)?;
    plan.assessment.validate("prepare")?;
    private_directory(&plan.state_directory)?;
    private_directory(&plan.context_directory)?;
    if !plan.state_directory.is_absolute()
        || plan.context_directory.parent() != Some(plan.state_directory.join("plans").as_path())
    {
        bail!("procedure context does not belong to selected state directory");
    }
    let recorded: Plan = read_json(&plan.context_directory.join("plan.json"))?;
    if serde_json::to_value(recorded)? != serde_json::to_value(plan)? {
        bail!("procedure plan differs from retained preparation");
    }
    Ok(())
}

fn verify_bindings(plan: &Plan) -> Result<Environment> {
    validate_document(plan)?;
    if engine_fingerprint()? != plan.engine_sha256 {
        bail!("procedure executor changed; prepare a new plan");
    }
    for item in &plan.bindings {
        if file_digest(&item.path)? != item.sha256 {
            bail!("procedure input changed; prepare a new plan");
        }
    }
    let environment = Environment::load(&plan.environment)?;
    plan.assessment.verify_references()?;
    recipe::verify_plan(&plan.inner_plan)?;
    if plan.inner_plan.environment != plan.context_directory.join("driver-context/environment.json")
        || plan.inner_plan.manifest != plan.context_directory.join("driver-context/recipe/recipe.json")
        || plan.inner_plan.operation != recipe::Operation::Apply
        || plan.inner_plan.target != "subject"
        || plan.inner_plan.commands.len() != 1
    {
        bail!("procedure inner execution context mismatch");
    }
    let vars: Value = read_json(&plan.context_directory.join("driver-context/vars.json"))?;
    if vars != json!({"cmdmesh_memory_prepared":plan.assessment.prepared}) {
        bail!("procedure prepared vars mismatch");
    }
    Ok(environment)
}

fn phase(plan: &Plan, phase: &str) -> Result<Assessment> {
    let receipt = inspection::run(
        &frozen_environment(plan),
        &plan.inspector,
        "lxc-memory-procedure",
        json!({"phase":phase,"prepared":plan.assessment.prepared}),
        120,
    )?;
    persist(&plan.context_directory.join(format!("{phase}.json")), &receipt)?;
    let actual = assessment(&receipt, phase)?;
    let mut expected_refs = plan.assessment.references.clone();
    if phase == "verify" {
        expected_refs.remove("ansible-executable");
    }
    if actual.subject_key != plan.assessment.subject_key
        || actual.prepared != plan.assessment.prepared
        || actual.prepared_sha256 != plan.assessment.prepared_sha256
        || actual.profile_files != plan.assessment.profile_files
        || actual.references != expected_refs
        || (phase == "preflight" && actual.driver != plan.assessment.driver)
    {
        bail!("procedure phase changed frozen subject, intent, profile or references");
    }
    actual.verify_references()?;
    Ok(actual)
}

fn report(plan: &Plan, outcome: Outcome) -> Report {
    Report {
        document_type: "cmdmesh.procedure.result".into(),
        schema_version: 1,
        plan_digest: plan.digest.clone(),
        subject_key: plan.assessment.subject_key.clone(),
        outcome,
        current_observation: false,
        command: CommandFacts::default(),
        postconditions_satisfied: None,
        checks: None,
        evidence_directory: plan.context_directory.clone(),
        diagnostics: vec![],
    }
}

pub fn execute(plan: &Plan, approval: &str) -> Result<Report> {
    if approval != plan.digest {
        bail!("approval digest does not match procedure plan");
    }
    let environment = verify_bindings(plan)?;
    let change_required = plan.assessment.prepared["change_required"].as_bool().unwrap();
    if !environment.execution.allow_execution || (change_required && !environment.execution.allow_mutations) {
        bail!("procedure execution or mutations disabled in original environment");
    }
    let journal = Journal::open(&plan.state_directory)?;
    let guard = journal.lock(&plan.assessment.subject_key)?;
    verify_bindings(plan)?;
    if journal.consumed(&plan.digest)?
        || exists(&plan.context_directory.join("result.json"))?
        || exists(&plan.context_directory.join("preflight.json"))?
        || exists(&plan.context_directory.join("driver-context/driver-facts.json"))?
    {
        bail!("procedure plan already assessed or consumed; inspect status instead of replaying");
    }
    if phase(plan, "preflight").is_err() {
        let mut result = report(plan, Outcome::Rejected);
        result.diagnostics.push("procedure-preflight-unqualified".into());
        persist(&plan.context_directory.join("result.json"), &result)?;
        return Ok(result);
    }
    verify_bindings(plan)?;
    if !change_required {
        let result = report(plan, Outcome::Checked);
        persist(&plan.context_directory.join("result.json"), &result)?;
        return Ok(result);
    }
    guard.consume(&plan.digest)?;
    let mut result = report(plan, Outcome::Uncertain);
    result.command.dispatch_attempted = true;
    result.command.native_completion = CommandCompletion::Unknown;
    match recipe::execute_attempt(
        &plan.inner_plan,
        &plan.inner_plan.digest,
        Some(&plan.context_directory.join("driver-logs")),
    ) {
        Ok(attempt) => {
            result.command.completed_steps = attempt.report.completed_steps;
            result.command.reporting_failed = attempt.reporting_error.is_some();
            result.command.completion = match attempt.completion {
                recipe::Completion::Succeeded => CommandCompletion::Succeeded,
                recipe::Completion::Failed { exit_code } => {
                    result.command.exit_code = Some(exit_code);
                    CommandCompletion::Failed
                }
                recipe::Completion::Unknown => CommandCompletion::Unknown,
            };
        }
        Err(_) => {
            result.command.completion = CommandCompletion::Unknown;
            result.diagnostics.push("procedure-dispatch-incomplete".into());
        }
    }
    match driver_facts(plan) {
        Ok(facts) => {
            result.command.native_completion = facts.command_outcome;
            result.command.native_mutation_attempted = Some(facts.mutation_attempted);
            result.command.native_module_failed = Some(facts.module_failed);
        }
        Err(_) => result
            .diagnostics
            .push("procedure-native-outcome-unavailable".into()),
    }
    let command_retention_error =
        persist(&plan.context_directory.join("command.json"), &result.command).err();
    match phase(plan, "verify") {
        Ok(actual) => {
            result.postconditions_satisfied = Some(actual.satisfied());
            result.checks = Some(actual.checks);
            result.outcome = if result.command.completion == CommandCompletion::Unknown
                || result.command.native_completion == CommandCompletion::Unknown
            {
                Outcome::Uncertain
            } else if result.command.completion == CommandCompletion::Succeeded
                && result.command.native_completion == CommandCompletion::Succeeded
                && result.command.native_module_failed == Some(false)
                && result.postconditions_satisfied == Some(true)
            {
                Outcome::Verified
            } else {
                Outcome::Unverified
            };
        }
        Err(_) => result.diagnostics.push("procedure-postcheck-unavailable".into()),
    }
    if let Some(error) = command_retention_error {
        return Err(
            error.context("command evidence could not be retained; independent postcheck was attempted")
        );
    }
    persist(&plan.context_directory.join("result.json"), &result)?;
    Ok(result)
}

/// Reads retained evidence only. It never reobserves current infrastructure.
pub fn status(plan: &Plan) -> Result<Report> {
    validate_document(plan)?;
    if let Ok(result) = read_json::<Report>(&plan.context_directory.join("result.json")) {
        version(result.schema_version)?;
        if result.document_type != "cmdmesh.procedure.result"
            || result.plan_digest != plan.digest
            || result.subject_key != plan.assessment.subject_key
            || result.current_observation
        {
            bail!("historical procedure result mismatch");
        }
        return Ok(result);
    }
    let journal = Journal::open(&plan.state_directory)?;
    let consumed = journal.consumed(&plan.digest)?;
    let invalid_result = exists(&plan.context_directory.join("result.json"))?;
    let interrupted_preflight = exists(&plan.context_directory.join("preflight.json"))?;
    let mut result = report(
        plan,
        if consumed || invalid_result {
            Outcome::Uncertain
        } else if interrupted_preflight {
            Outcome::Pending
        } else {
            Outcome::Planned
        },
    );
    if consumed {
        result.command.dispatch_attempted = true;
        result.command.completion = CommandCompletion::Unknown;
        result
            .diagnostics
            .push("procedure-consumed-without-final-result".into());
    } else if invalid_result {
        result
            .diagnostics
            .push("procedure-stored-result-unavailable".into());
    } else if interrupted_preflight {
        result
            .diagnostics
            .push("procedure-preflight-recorded-without-final-result".into());
    }
    Ok(result)
}
