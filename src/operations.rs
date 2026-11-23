//! Discover known operations without executing instructions from metadata.
use crate::{config::Environment, digest, file_digest, identifier, procedure, read_regular_file, recipe};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub document_type: String,
    pub schema_version: u32,
    pub operation_version: u32,
    pub id: String,
    pub aliases: Vec<String>,
    pub title: String,
    pub summary: String,
    pub parameters: BTreeMap<String, Parameter>,
    pub requirements: Vec<Requirement>,
    pub entrypoint: EntryPoint,
    pub expected_results: Vec<String>,
    pub known_failures: Vec<KnownFailure>,
    pub provenance: Provenance,
    #[serde(skip)]
    path: PathBuf,
    #[serde(skip)]
    source_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    #[serde(rename = "type")]
    pub kind: ParameterType,
    pub required: bool,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ParameterType {
    Target,
    Inspector,
    StateDirectory,
    MemoryRequest,
    MemoryMb,
    MemoryCapacityPolicy,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    pub capability: String,
    pub protocol_version: u32,
    pub supported_tools: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "driver", rename_all = "kebab-case", deny_unknown_fields)]
pub enum EntryPoint {
    RecipePlan {
        manifest: PathBuf,
        operation: recipe::Operation,
    },
    ProcedurePlan {
        profile: String,
        profile_version: u32,
    },
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum PreparedPlan {
    Recipe(Box<recipe::Plan>),
    Procedure(Box<procedure::Plan>),
}

impl PreparedPlan {
    pub fn digest(&self) -> &str {
        match self {
            Self::Recipe(plan) => &plan.digest,
            Self::Procedure(plan) => &plan.digest,
        }
    }
    pub fn warning(&self) -> &str {
        match self {Self::Recipe(plan)=>&plan.warning,Self::Procedure(_)=>"Bound verified procedure plan; review its exact subject, inputs and digest. A consumed plan cannot be replayed."}
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KnownFailure {
    pub code: String,
    pub summary: String,
    pub next_checks: Vec<String>,
    pub recovery: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub references: Vec<String>,
    pub qualification: String,
    pub limitations: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Summary {
    pub id: String,
    pub operation_version: u32,
    pub aliases: Vec<String>,
    pub title: String,
    pub summary: String,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub schema_version: u32,
    pub items: Vec<Summary>,
    pub total: usize,
    pub next_offset: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct Diagnostic {
    pub code: &'static str,
    pub parameter: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct InputReport {
    pub schema_version: u32,
    pub valid: bool,
    pub execution: &'static str,
    pub requirements_evaluated: bool,
    pub diagnostics: Vec<Diagnostic>,
}

fn text(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        bail!("operation text must be nonempty, bounded and contain no control characters");
    }
    Ok(())
}

fn texts(values: &[String]) -> Result<()> {
    if values.is_empty() || values.len() > 100 {
        bail!("operation guidance requires 1..100 entries");
    }
    for value in values {
        text(value)?;
    }
    Ok(())
}

fn lookup(value: &str) -> Result<String> {
    text(value)?;
    if value.len() > 128 {
        bail!("operation identity or alias exceeds 128 bytes");
    }
    Ok(value.trim().to_lowercase())
}

fn load(path: &Path) -> Result<Operation> {
    let bytes = read_regular_file(path)?;
    let mut operation: Operation = serde_json::from_slice(&bytes).context("invalid operation document")?;
    if operation.document_type != "cmdmesh.operation" {
        bail!("unsupported operation document_type");
    }
    if !(1..=2).contains(&operation.schema_version) {
        bail!("unsupported operation schema_version");
    }
    if operation.operation_version != 1 {
        bail!("unsupported operation_version; expected 1");
    }
    identifier(&operation.id)?;
    text(&operation.title)?;
    text(&operation.summary)?;
    if operation.aliases.len() > 32 {
        bail!("too many operation aliases");
    }
    for alias in &operation.aliases {
        lookup(alias)?;
    }
    let expected = match &operation.entrypoint {
        EntryPoint::RecipePlan { .. } if operation.schema_version == 1 => {
            vec![("target", ParameterType::Target)]
        }
        EntryPoint::ProcedurePlan {
            profile,
            profile_version,
        } if operation.schema_version == 2 && profile == "proxmox-lxc-memory" && *profile_version == 1 => {
            vec![
                ("inspector", ParameterType::Inspector),
                ("state_directory", ParameterType::StateDirectory),
                ("request", ParameterType::MemoryRequest),
                ("desired_memory_mb", ParameterType::MemoryMb),
                ("capacity_policy", ParameterType::MemoryCapacityPolicy),
            ]
        }
        _ => bail!("unsupported operation schema/entrypoint/profile combination"),
    };
    if operation.parameters.len() != expected.len() {
        bail!("entrypoint requires its exact typed private parameters");
    }
    for (name, kind) in expected {
        let parameter = operation
            .parameters
            .get(name)
            .context("entrypoint parameter is missing")?;
        if !parameter.required || parameter.kind != kind {
            bail!("entrypoint requires mandatory correctly typed private parameters");
        }
        text(&parameter.description)?;
    }
    if operation.requirements.is_empty() || operation.requirements.len() > 100 {
        bail!("operation requires explicit capabilities");
    }
    let mut capabilities = BTreeSet::new();
    for requirement in &operation.requirements {
        identifier(&requirement.capability)?;
        if requirement.protocol_version != 1 {
            bail!("unsupported capability protocol_version; expected 1");
        }
        if !capabilities.insert(&requirement.capability) {
            bail!("duplicate operation capability");
        }
        if requirement.supported_tools.is_empty() {
            bail!("supported tool versions are required");
        }
        for (tool, versions) in &requirement.supported_tools {
            identifier(tool)?;
            texts(versions)?;
        }
    }
    let required = match operation.entrypoint {
        EntryPoint::RecipePlan { .. } => "ansible-recipe",
        EntryPoint::ProcedurePlan { .. } => "verified-procedure",
    };
    if !capabilities.contains(&required.to_owned()) {
        bail!("entrypoint requires its declared execution capability");
    }
    texts(&operation.expected_results)?;
    if operation.known_failures.is_empty() || operation.known_failures.len() > 100 {
        bail!("operation requires known failure and recovery guidance");
    }
    let mut codes = BTreeSet::new();
    for failure in &operation.known_failures {
        identifier(&failure.code)?;
        if !codes.insert(&failure.code) {
            bail!("duplicate operation failure code");
        }
        text(&failure.summary)?;
        texts(&failure.next_checks)?;
        texts(&failure.recovery)?;
    }
    texts(&operation.provenance.references)?;
    text(&operation.provenance.qualification)?;
    texts(&operation.provenance.limitations)?;
    operation.path = path.canonicalize()?;
    operation.source_sha256 = digest(&bytes);
    // Colocation makes the existing recipe tree digest bind these instructions.
    if let EntryPoint::RecipePlan { manifest, .. } = &operation.entrypoint {
        if manifest != Path::new("recipe.json") {
            bail!("recipe-plan manifest must be the colocated recipe.json");
        }
        let root = operation.path.parent().unwrap();
        let recipe = recipe::inspect(&recipe::confined_file(root, manifest)?)?;
        if recipe.status != recipe::Status::Ready {
            bail!("operation entrypoint recipe is not ready");
        }
    }
    Ok(operation)
}

fn catalog(root: &Path) -> Result<Vec<Operation>> {
    if fs::symlink_metadata(root)?.file_type().is_symlink() {
        bail!("catalog root cannot be a symlink");
    }
    let root = root.canonicalize()?;
    let mut output = Vec::new();
    let mut found_directory = false;
    for name in ["recipes", "operations"] {
        let directory = root.join(name);
        let metadata = match fs::symlink_metadata(&directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() {
            bail!("operation directory cannot be a symlink");
        }
        found_directory = true;
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if entry.file_type()?.is_symlink() {
                bail!("operation entries cannot be symlinks");
            }
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let path = entry.path().join("operation.json");
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if output.len() >= 1000 {
                bail!("catalog exceeds 1000 operations");
            }
            if metadata.file_type().is_symlink() {
                bail!("operation manifest cannot be a symlink");
            }
            output.push(load(&path)?);
        }
    }
    if !found_directory {
        bail!("catalog must contain recipes or operations directory");
    }
    output.sort_by(|a, b| a.id.cmp(&b.id));
    let mut aliases = BTreeSet::new();
    for operation in &output {
        for alias in std::iter::once(&operation.id).chain(&operation.aliases) {
            if !aliases.insert(lookup(alias)?) {
                bail!("duplicate operation identity or alias");
            }
        }
    }
    Ok(output)
}

pub fn search(root: &Path, query: &str, offset: usize, limit: usize) -> Result<SearchResult> {
    if !(1..=100).contains(&limit) || query.len() > 256 || query.chars().any(char::is_control) {
        bail!("operation search needs limit 1..100 and a bounded query without control characters");
    }
    let words: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
    let entries: Vec<_> = catalog(root)?
        .into_iter()
        .filter(|entry| {
            let content = format!(
                "{} {} {} {}",
                entry.id,
                entry.title,
                entry.summary,
                entry.aliases.join(" ")
            )
            .to_lowercase();
            words.iter().all(|word| content.contains(word))
        })
        .collect();
    let total = entries.len();
    let items = entries
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|entry| Summary {
            id: entry.id,
            operation_version: entry.operation_version,
            aliases: entry.aliases,
            title: entry.title,
            summary: entry.summary,
        })
        .collect();
    Ok(SearchResult {
        schema_version: 1,
        items,
        total,
        next_offset: offset.checked_add(limit).filter(|next| *next < total),
    })
}

pub fn inspect(root: &Path, selector: &str) -> Result<Operation> {
    let selector = lookup(selector)?;
    catalog(root)?
        .into_iter()
        .find(|entry| {
            std::iter::once(&entry.id)
                .chain(&entry.aliases)
                .any(|alias| lookup(alias).is_ok_and(|value| value == selector))
        })
        .context("unknown operation; list or search the catalog")
}

pub fn validate_inputs(
    entry: &Operation,
    values: &Value,
    environment: Option<&Environment>,
) -> Result<InputReport> {
    let mut diagnostics = Vec::new();
    let mut issue = |code, parameter| diagnostics.push(Diagnostic { code, parameter });
    if let Some(values) = values.as_object() {
        if values.keys().any(|key| !entry.parameters.contains_key(key)) {
            issue("unknown-input", None);
        }
        if matches!(entry.entrypoint, EntryPoint::RecipePlan { .. }) {
            match values.get("target") {
                None => issue("missing-input", Some("target")),
                Some(value) => match value.as_str() {
                    None => issue("invalid-input-type", Some("target")),
                    Some(target) => match environment {
                        None => issue("configuration-required", Some("target")),
                        Some(environment) if !environment.targets.contains_key(target) => {
                            issue("unknown-target", Some("target"))
                        }
                        Some(_) => (),
                    },
                },
            }
        } else {
            for name in [
                "inspector",
                "state_directory",
                "request",
                "desired_memory_mb",
                "capacity_policy",
            ] {
                let Some(value) = values.get(name) else {
                    issue("missing-input", Some(name));
                    continue;
                };
                let valid = match name {
                    "inspector" | "state_directory" => value
                        .as_str()
                        .is_some_and(|text| !text.is_empty() && !text.chars().any(char::is_control)),
                    "desired_memory_mb" => value.as_u64().is_some_and(|n| (16..=2147483647).contains(&n)),
                    _ => value.is_object(),
                };
                if !valid {
                    issue("invalid-input-type", Some(name));
                    continue;
                }
                if name == "inspector" {
                    match environment {
                        None => issue("configuration-required", Some(name)),
                        Some(env) if !env.providers.contains_key(value.as_str().unwrap()) => {
                            issue("unknown-inspector", Some(name))
                        }
                        _ => {}
                    }
                }
            }
        }
    } else {
        issue("invalid-input-document", None);
    }
    Ok(InputReport {
        schema_version: 1,
        valid: diagnostics.is_empty(),
        execution: "not-run",
        requirements_evaluated: false,
        diagnostics,
    })
}

pub fn plan(environment: &Environment, root: &Path, selector: &str, values: &Value) -> Result<recipe::Plan> {
    let entry = inspect(root, selector)?;
    if !validate_inputs(&entry, values, Some(environment))?.valid {
        bail!("invalid operation inputs; use operation validate-inputs for diagnostic codes");
    }
    plan_recipe(environment, &entry, values)
}

fn plan_recipe(environment: &Environment, entry: &Operation, values: &Value) -> Result<recipe::Plan> {
    if file_digest(&entry.path)? != entry.source_sha256 {
        bail!("operation metadata changed before planning");
    }
    let EntryPoint::RecipePlan { manifest, operation } = &entry.entrypoint else {
        bail!("use operation prepare for verified procedure planning");
    };
    let planned = recipe::plan(
        environment,
        &entry.path.parent().unwrap().join(manifest),
        values["target"].as_str().unwrap(),
        operation.clone(),
    )?;
    recipe::verify_recipe_member(&planned, &entry.path, &entry.source_sha256)?;
    if file_digest(&entry.path)? != entry.source_sha256 {
        bail!("operation metadata changed during recipe capture; prepare a new plan");
    }
    Ok(planned)
}

pub fn prepare(
    environment: &Environment,
    root: &Path,
    selector: &str,
    values: &Value,
) -> Result<PreparedPlan> {
    let entry = inspect(root, selector)?;
    if !validate_inputs(&entry, values, Some(environment))?.valid {
        bail!("invalid operation inputs; use operation validate-inputs for diagnostic codes");
    }
    match &entry.entrypoint {
        EntryPoint::RecipePlan { .. } => Ok(PreparedPlan::Recipe(Box::new(plan_recipe(
            environment,
            &entry,
            values,
        )?))),
        EntryPoint::ProcedurePlan { .. } => Ok(PreparedPlan::Procedure(Box::new(procedure::plan_bound(
            environment,
            values["inspector"].as_str().unwrap(),
            serde_json::json!({"request":values["request"],"desired_memory_mb":values["desired_memory_mb"],"capacity_policy":values["capacity_policy"]}),
            Path::new(values["state_directory"].as_str().unwrap()),
            Some(procedure::OperationSource {
                id: entry.id,
                document: procedure::Reference {
                    path: entry.path,
                    sha256: entry.source_sha256,
                },
            }),
        )?))),
    }
}
