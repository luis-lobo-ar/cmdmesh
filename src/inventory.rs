use crate::{config::Environment, file_digest, identifier, process, version};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub kind: String,
    pub id: String,
    pub properties: Value,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema_version: u32,
    pub complete: bool,
    pub resources: Vec<Resource>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Desired {
    schema_version: u32,
    resources: Vec<DesiredResource>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DesiredResource {
    kind: String,
    id: String,
    spec: Value,
}
#[derive(Debug, Serialize)]
pub struct DriftItem {
    pub change: String,
    pub kind: String,
    pub id: String,
    pub fields: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct DriftReport {
    pub schema_version: u32,
    pub clean: bool,
    pub complete: bool,
    pub items: Vec<DriftItem>,
}

impl Snapshot {
    pub fn validate(&self) -> Result<()> {
        version(self.schema_version)?;
        if self.resources.len() > 100_000 || self.warnings.len() > 10_000 {
            bail!("inventory exceeds item limit");
        }
        let mut seen = BTreeSet::new();
        for resource in &self.resources {
            validate_identity(&resource.kind, &resource.id)?;
            if !resource.properties.is_object() {
                bail!("resource properties must be an object");
            }
            if !seen.insert((&resource.kind, &resource.id)) {
                bail!("duplicate inventory identity");
            }
        }
        Ok(())
    }
}

fn validate_identity(kind: &str, id: &str) -> Result<()> {
    if kind.is_empty()
        || id.is_empty()
        || kind.len() > 256
        || id.len() > 1024
        || kind.chars().chain(id.chars()).any(char::is_control)
    {
        bail!("invalid resource identity");
    }
    Ok(())
}

pub fn pull(environment: &Environment, provider_id: &str) -> Result<Snapshot> {
    identifier(provider_id)?;
    let provider = environment
        .providers
        .get(provider_id)
        .context("unknown provider")?;
    let executable = environment
        .resolve(&provider.executable)
        .canonicalize()
        .context("provider executable unavailable")?;
    if let Some(expected) = &provider.expected_sha256 {
        if file_digest(&executable)? != *expected {
            bail!("provider executable digest mismatch");
        }
    }
    let mut config = provider.config.clone();
    if let Some(ca) = config
        .get("ca_file")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
    {
        config["ca_file"] = json!(environment.resolve(std::path::Path::new(ca)));
    }
    let request = serde_json::to_vec(&json!({"schema_version":1,"operation":"inventory","config":config}))?;
    let output = process::run(process::RunSpec {
        program: &executable,
        args: &[],
        input: Some(request),
        allowlist: &provider.env_allowlist,
        extra_env: &BTreeMap::new(),
        cwd: environment.path.parent().unwrap(),
        timeout: Duration::from_secs(120),
        private_root: environment.path.parent().unwrap(),
    })?;
    let snapshot: Snapshot = serde_json::from_slice(&output)
        .map_err(|_| anyhow::anyhow!("provider returned invalid inventory JSON"))?;
    snapshot.validate()?;
    Ok(snapshot)
}

pub fn drift_values(desired: Value, actual: Value) -> Result<DriftReport> {
    let desired: Desired = serde_json::from_value(desired).context("invalid desired-state document")?;
    let actual: Snapshot = serde_json::from_value(actual).context("invalid inventory document")?;
    version(desired.schema_version)?;
    actual.validate()?;
    let by_id: BTreeMap<_, _> = actual
        .resources
        .iter()
        .map(|r| ((r.kind.as_str(), r.id.as_str()), r))
        .collect();
    let mut seen = BTreeSet::new();
    let mut items = vec![];
    for resource in &desired.resources {
        validate_identity(&resource.kind, &resource.id)?;
        let spec = resource
            .spec
            .as_object()
            .context("desired spec must be an object")?;
        let key = (resource.kind.as_str(), resource.id.as_str());
        if !seen.insert(key) {
            bail!("duplicate desired identity");
        }
        let Some(observed) = by_id.get(&key) else {
            items.push(DriftItem {
                change: "missing".into(),
                kind: resource.kind.clone(),
                id: resource.id.clone(),
                fields: vec![],
            });
            continue;
        };
        let mut fields = vec![];
        for (field, expected) in spec {
            if !subset(expected, observed.properties.get(field)) {
                fields.push(field.clone());
            }
        }
        if !fields.is_empty() {
            items.push(DriftItem {
                change: "changed".into(),
                kind: resource.kind.clone(),
                id: resource.id.clone(),
                fields,
            });
        }
    }
    for observed in &actual.resources {
        if !seen.contains(&(observed.kind.as_str(), observed.id.as_str())) {
            items.push(DriftItem {
                change: "unmanaged".into(),
                kind: observed.kind.clone(),
                id: observed.id.clone(),
                fields: vec![],
            });
        }
    }
    Ok(DriftReport {
        schema_version: 1,
        clean: actual.complete && items.is_empty(),
        complete: actual.complete,
        items,
    })
}

fn subset(expected: &Value, actual: Option<&Value>) -> bool {
    match (expected, actual) {
        (Value::Object(fields), Some(Value::Object(observed))) => fields
            .iter()
            .all(|(name, value)| subset(value, observed.get(name))),
        (_, Some(actual)) => expected == actual,
        _ => false,
    }
}
