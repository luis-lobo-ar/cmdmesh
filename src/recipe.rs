use crate::{
    config::{Environment, LifecycleOwner},
    digest, file_digest, identifier, process, read_json, version, write_private_json,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    os::fd::AsRawFd,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub summary: String,
    pub status: Status,
    pub requirements: Vec<String>,
    pub dependencies: Vec<String>,
    pub steps: Vec<Step>,
    pub verification: Vec<String>,
    pub recovery: PathBuf,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ready,
    Draft,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: String,
    pub driver: Driver,
    pub playbook: PathBuf,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Driver {
    Ansible,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    Check,
    Apply,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BoundInput {
    pub path: PathBuf,
    pub sha256: String,
    pub directory: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedCommand {
    pub executable: PathBuf,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub schema_version: u32,
    pub digest: String,
    pub environment: PathBuf,
    pub manifest: PathBuf,
    pub target: String,
    pub operation: Operation,
    pub inputs: Vec<BoundInput>,
    pub commands: Vec<PlannedCommand>,
    pub collection_paths: Vec<PathBuf>,
    pub ansible_config: String,
    pub warning: String,
}
#[derive(Debug, Serialize)]
pub struct ExecutionReport {
    pub schema_version: u32,
    pub plan_digest: String,
    pub completed_steps: usize,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_dir: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Completion {
    Succeeded,
    Failed { exit_code: i32 },
    Unknown,
}

pub(crate) struct ExecutionAttempt {
    pub report: ExecutionReport,
    pub completion: Completion,
    pub reporting_error: Option<anyhow::Error>,
    error: Option<anyhow::Error>,
}

pub fn inspect(manifest: &Path) -> Result<Recipe> {
    if fs::symlink_metadata(manifest)?.file_type().is_symlink() {
        bail!("recipe manifest cannot be a symlink");
    }
    let recipe: Recipe = read_json(manifest)?;
    version(recipe.schema_version)?;
    identifier(&recipe.id)?;
    if recipe.title.trim().is_empty() || recipe.summary.trim().is_empty() {
        bail!("recipe title and summary are required");
    }
    if recipe.status == Status::Ready && recipe.steps.is_empty() {
        bail!("ready recipe requires executable steps");
    }
    let root = manifest.canonicalize()?.parent().unwrap().to_path_buf();
    confined_file(&root, &recipe.recovery)?;
    let mut step_ids = BTreeSet::new();
    for step in &recipe.steps {
        identifier(&step.id)?;
        if !step_ids.insert(&step.id) {
            bail!("duplicate recipe step id");
        }
        confined_file(&root, &step.playbook)?;
    }
    let mut dependencies = BTreeSet::new();
    for dependency in &recipe.dependencies {
        identifier(dependency)?;
        if !dependencies.insert(dependency) {
            bail!("duplicate dependency");
        }
    }
    Ok(recipe)
}

pub fn list(catalog: &Path) -> Result<Vec<Recipe>> {
    let recipes = catalog.join("recipes");
    let mut paths = vec![];
    for entry in fs::read_dir(recipes).context("catalog must contain recipes directory")? {
        let entry = entry?;
        if entry.file_type()?.is_symlink() {
            bail!("catalog cannot contain symlinked recipes");
        }
        let manifest = entry.path().join("recipe.json");
        if entry.file_type()?.is_dir() && manifest.is_file() {
            paths.push(manifest);
        }
    }
    paths.sort();
    let output: Vec<Recipe> = paths.iter().map(|p| inspect(p)).collect::<Result<_>>()?;
    let mut ids = BTreeSet::new();
    for recipe in &output {
        if !ids.insert(&recipe.id) {
            bail!("duplicate catalog recipe id");
        }
    }
    Ok(output)
}

pub fn plan(
    environment: &Environment,
    manifest: &Path,
    target_name: &str,
    operation: Operation,
) -> Result<Plan> {
    let recipe = inspect(manifest)?;
    if recipe.status == Status::Draft {
        bail!("draft recipes cannot be planned or executed");
    }
    identifier(target_name)?;
    let target = environment.targets.get(target_name).context("unknown target")?;
    if recipe.requirements.iter().any(|r| r == "lifecycle-owner:ansible")
        && target.lifecycle_owner != LifecycleOwner::Ansible
    {
        bail!(
            "recipe requires Ansible lifecycle ownership; Terraform-owned resource fields cannot be changed"
        );
    }
    let manifest = manifest.canonicalize()?;
    let root = manifest.parent().unwrap().to_path_buf();
    let inventory = environment
        .resolve(&target.inventory)
        .canonicalize()
        .context("target inventory unavailable")?;
    let mut inputs = vec![
        bind_file(&environment.path)?,
        bind_tree(&root)?,
        bind_path(&inventory)?,
    ];
    if inventory.is_file() {
        for name in ["group_vars", "host_vars"] {
            let automatic_vars = inventory.parent().unwrap().join(name);
            if automatic_vars.exists() {
                inputs.push(bind_tree(&automatic_vars)?);
            }
        }
    }
    let mut collection_paths = vec![];
    for dependency in &recipe.dependencies {
        let package = environment
            .packages
            .get(dependency)
            .context("required package is not configured")?;
        let path = environment.resolve(package);
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            bail!("dependency root cannot be a symlink");
        }
        let path = path.canonicalize()?;
        inputs.push(bind_tree(&path)?);
        let ansible = path.join("ansible");
        if ansible.join("ansible_collections").is_dir() {
            collection_paths.push(ansible);
        }
    }
    let vars_file = target
        .vars_file
        .as_ref()
        .map(|p| environment.resolve(p).canonicalize())
        .transpose()
        .context("target vars unavailable")?;
    if let Some(path) = &vars_file {
        inputs.push(bind_file(path)?);
    }
    let executable = resolve_ansible(environment)?;
    if executable.exists() {
        inputs.push(bind_file(&executable)?);
    }
    let mut commands = vec![];
    for step in &recipe.steps {
        let playbook = confined_file(&root, &step.playbook)?;
        let mut argv = vec![
            "--inventory".into(),
            inventory.to_string_lossy().into_owned(),
            "--limit".into(),
            target.limit.clone(),
        ];
        if operation == Operation::Check {
            argv.push("--check".into());
        }
        if let Some(vars) = &vars_file {
            argv.extend(["--extra-vars".into(), format!("@{}", vars.display())]);
        }
        argv.extend([
            "--extra-vars".into(),
            serde_json::json!({"cmdmesh_allow_changes":operation == Operation::Apply, "lifecycle_owner":target.lifecycle_owner}).to_string(),
        ]);
        argv.push(playbook.to_string_lossy().into_owned());
        commands.push(PlannedCommand {
            executable: executable.clone(),
            argv,
            cwd: root.clone(),
        });
    }
    let mut plan = Plan { schema_version:1, digest:String::new(), environment:environment.path.clone(), manifest, target:target_name.into(), operation, inputs, commands, collection_paths, ansible_config:"[defaults]\n[ssh_connection]\nssh_args = -C -o ControlMaster=no -o ControlPersist=no -o ControlPath=none\n".into(),
        warning:"Static execution plan, not an Ansible change preview. Trusted Ansible code may have effects even in check mode. Approval is acknowledgement, not a sandbox. Remote outcome may be unknown on interruption.".into() };
    plan.digest = plan_digest(&plan)?;
    Ok(plan)
}

pub fn verify_plan(plan: &Plan) -> Result<()> {
    version(plan.schema_version)?;
    if plan_digest(plan)? != plan.digest {
        bail!("plan digest mismatch");
    }
    let environment = Environment::load(&plan.environment)?;
    let current = self::plan(&environment, &plan.manifest, &plan.target, plan.operation.clone())?;
    if current.digest != plan.digest {
        bail!("plan is stale: recipe, package, target, executable or private inputs changed; prepare a new plan");
    }
    Ok(())
}

pub fn execute(plan: &Plan, approval: &str) -> Result<ExecutionReport> {
    execute_with_logs(plan, approval, None)
}

pub fn execute_with_logs(plan: &Plan, approval: &str, log_dir: Option<&Path>) -> Result<ExecutionReport> {
    let attempt = execute_attempt(plan, approval, log_dir)?;
    if let Some(error) = attempt.reporting_error.or(attempt.error) {
        return Err(error);
    }
    match attempt.completion {
        Completion::Succeeded => Ok(attempt.report),
        Completion::Failed { .. } | Completion::Unknown => {
            bail!("child completion unavailable; remote outcome may be unknown")
        }
    }
}

pub(crate) fn execute_attempt(
    plan: &Plan,
    approval: &str,
    log_dir: Option<&Path>,
) -> Result<ExecutionAttempt> {
    if approval != plan.digest {
        bail!("approval digest does not match plan");
    }
    let environment = Environment::load(&plan.environment)?;
    if !environment.execution.allow_execution {
        bail!("execution disabled in private configuration");
    }
    if plan.operation == Operation::Apply && !environment.execution.allow_mutations {
        bail!("mutations disabled in private configuration");
    }
    verify_plan(plan)?;
    // Keep the lock inode stable: unlinking it after release permits split locks.
    let lock_path = environment
        .path
        .parent()
        .unwrap()
        .join(format!(".cmdmesh-{}.lock", plan.target));
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(lock_path)
        .context("cannot open target execution lock")?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        bail!("another local execution owns this target");
    }
    verify_plan(plan)?;
    let mut extra_env = BTreeMap::from([
        ("ANSIBLE_NOCOLOR".into(), "1".into()),
        ("ANSIBLE_DIFF_ALWAYS".into(), "False".into()),
        ("ANSIBLE_HOST_KEY_CHECKING".into(), "True".into()),
    ]);
    if !plan.collection_paths.is_empty() {
        let paths = std::env::join_paths(&plan.collection_paths).context("invalid collection paths")?;
        extra_env.insert(
            "ANSIBLE_COLLECTIONS_PATH".into(),
            paths.to_string_lossy().into_owned(),
        );
    }
    let log_dir = log_dir.map(|path| create_log_dir(path, plan)).transpose()?;
    let mut report = ExecutionReport {
        schema_version: 1,
        plan_digest: plan.digest.clone(),
        completed_steps: 0,
        status: "failed".into(),
        log_dir,
    };
    let mut completion = Completion::Unknown;
    let result = (|| -> Result<()> {
        for (index, command) in plan.commands.iter().enumerate() {
            let logs = report
                .log_dir
                .as_ref()
                .map(|dir| -> Result<_> {
                    Ok(process::StreamLogs {
                        stdout: create_log_file(&dir.join(format!("step-{:04}.stdout", index + 1)))?,
                        stderr: create_log_file(&dir.join(format!("step-{:04}.stderr", index + 1)))?,
                    })
                })
                .transpose()?;
            let args: Vec<OsString> = command.argv.iter().map(OsString::from).collect();
            let output = process::capture(process::RunSpec {
                program: &command.executable,
                args: &args,
                input: None,
                allowlist: &environment.execution.env_allowlist,
                extra_env: &extra_env,
                cwd: &command.cwd,
                timeout: Duration::from_secs(environment.execution.timeout_seconds),
                private_root: environment.path.parent().unwrap(),
                ansible_config: Some(&plan.ansible_config),
                logs,
            })?;
            if !output.status.success() {
                completion = match output.status.code() {
                    Some(code) if ![124, 137, 143].contains(&code) => Completion::Failed { exit_code: code },
                    _ => Completion::Unknown,
                };
                bail!(
                    "child operation failed (exit {:?}); raw output withheld",
                    output.status.code()
                );
            }
            report.completed_steps += 1;
        }
        Ok(())
    })();
    if result.is_ok() {
        report.status = "succeeded".into();
        completion = Completion::Succeeded;
    }
    let reporting_error = report
        .log_dir
        .as_ref()
        .and_then(|directory| write_private_json(&directory.join("execution.json"), &report).err());
    Ok(ExecutionAttempt {
        report,
        completion,
        reporting_error,
        error: result.err(),
    })
}

fn create_log_dir(path: &Path, plan: &Plan) -> Result<PathBuf> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let directory = parent
        .canonicalize()?
        .join(path.file_name().context("log directory needs a new name")?);
    if plan
        .inputs
        .iter()
        .any(|input| input.directory && directory.starts_with(&input.path))
    {
        bail!("log directory must be outside bound input trees");
    }
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .context("cannot create private log directory; existing paths are never overwritten")?;
    Ok(directory)
}

fn create_log_file(path: &Path) -> Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .context("cannot create private step log")
}

pub(crate) fn resolve_ansible(environment: &Environment) -> Result<PathBuf> {
    if let Some(path) = &environment.execution.ansible_executable {
        return environment
            .resolve(path)
            .canonicalize()
            .context("configured Ansible executable unavailable");
    }
    for directory in ["/usr/local/bin", "/usr/bin", "/bin"] {
        let path = Path::new(directory).join("ansible-playbook");
        if path.is_file() {
            return Ok(path.canonicalize()?);
        }
    }
    // Static planning remains useful without Ansible installed. Installation
    // changes this identity and therefore requires a new plan before execution.
    Ok(PathBuf::from("/usr/bin/ansible-playbook"))
}

fn plan_digest(plan: &Plan) -> Result<String> {
    let mut unsigned = plan.clone();
    unsigned.digest.clear();
    Ok(digest(&serde_json::to_vec(&unsigned)?))
}
fn bind_file(path: &Path) -> Result<BoundInput> {
    Ok(BoundInput {
        path: path.to_owned(),
        sha256: file_digest(path)?,
        directory: false,
    })
}
fn bind_path(path: &Path) -> Result<BoundInput> {
    if path.is_dir() {
        bind_tree(path)
    } else {
        bind_file(path)
    }
}
fn bind_tree(root: &Path) -> Result<BoundInput> {
    if fs::symlink_metadata(root)?.file_type().is_symlink() {
        bail!("bound tree root cannot be a symlink");
    }
    let mut entries = BTreeMap::new();
    collect_tree(root, root, &mut entries)?;
    Ok(BoundInput {
        path: root.to_owned(),
        sha256: digest(&serde_json::to_vec(&entries)?),
        directory: true,
    })
}

pub(crate) fn verify_recipe_member(plan: &Plan, path: &Path, expected_sha256: &str) -> Result<()> {
    let root = plan.manifest.parent().context("recipe manifest has no parent")?;
    let bound = plan
        .inputs
        .iter()
        .find(|input| input.directory && input.path == root)
        .context("recipe tree binding missing")?;
    let mut members = BTreeMap::new();
    collect_tree(root, root, &mut members)?;
    let relative = path
        .strip_prefix(root)?
        .to_str()
        .context("metadata path is not UTF-8")?;
    if !members.contains_key(relative) {
        bail!("loaded metadata is outside the bound recipe tree");
    }
    // Compare against the captured tree, not only today's file: restoration
    // after an intervening change must not disguise different bound metadata.
    members.insert(relative.to_owned(), expected_sha256.to_owned());
    if digest(&serde_json::to_vec(&members)?) != bound.sha256 {
        bail!("captured recipe tree does not bind the loaded operation metadata");
    }
    Ok(())
}
fn collect_tree(root: &Path, directory: &Path, entries: &mut BTreeMap<String, String>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git"
            || name == ".generated"
            || name == ".terraform"
            || name == ".venv"
            || name == ".ansible"
        {
            continue;
        }
        if entries.len() >= 10_000 {
            bail!("package tree exceeds file count limit");
        }
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            bail!("recipe and package trees cannot contain symlinks");
        }
        let path = entry.path();
        if file_type.is_dir() {
            collect_tree(root, &path, entries)?;
            continue;
        }
        if !file_type.is_file() {
            bail!("package tree contains unsupported file type");
        }
        let relative = path
            .strip_prefix(root)?
            .to_str()
            .context("package paths must be UTF-8")?
            .to_owned();
        entries.insert(relative, file_digest(&path)?);
    }
    Ok(())
}
pub(crate) fn confined_file(root: &Path, relative: &Path) -> Result<PathBuf> {
    if relative.as_os_str().is_empty() || relative.components().any(|c| !matches!(c, Component::Normal(_))) {
        bail!("recipe paths must be relative without traversal");
    }
    let mut path = root.to_owned();
    for component in relative.components() {
        path.push(component);
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            bail!("recipe paths cannot contain symlinks");
        }
    }
    if !path.is_file() {
        bail!("recipe path must identify a regular file");
    }
    Ok(path)
}

#[cfg(test)]
mod completion_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fixture(script: &str) -> (tempfile::TempDir, Plan) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(".generated/recipe-completion-tests");
        fs::create_dir_all(&root).unwrap();
        let temporary = tempfile::tempdir_in(root).unwrap();
        let dir = temporary.path();
        fs::create_dir(dir.join("recipe")).unwrap();
        fs::write(dir.join("inventory.ini"), "node_a\n").unwrap();
        fs::write(
            dir.join("recipe/playbook.yml"),
            "---\n- hosts: node_a\n  tasks: []\n",
        )
        .unwrap();
        fs::write(
            dir.join("recipe/README.md"),
            "Inspect actual state after failure.\n",
        )
        .unwrap();
        fs::write(dir.join("driver"), format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(dir.join("driver"), fs::Permissions::from_mode(0o700)).unwrap();
        write_private_json(
            &dir.join("recipe/recipe.json"),
            &serde_json::json!({
                "schema_version":1,"id":"memory","title":"Memory","summary":"Fixture only",
                "status":"ready","requirements":[],"dependencies":[],
                "steps":[{"id":"grow","driver":"ansible","playbook":"playbook.yml"}],
                "verification":[],"recovery":"README.md"
            }),
        )
        .unwrap();
        write_private_json(&dir.join("environment.json"), &serde_json::json!({
            "schema_version":1,"targets":{"test":{"inventory":"inventory.ini","limit":"node_a","lifecycle_owner":"ansible"}},
            "execution":{"allow_execution":true,"allow_mutations":true,"ansible_executable":"driver","timeout_seconds":1}
        })).unwrap();
        let environment = Environment::load(&dir.join("environment.json")).unwrap();
        let plan = plan(
            &environment,
            &dir.join("recipe/recipe.json"),
            "test",
            Operation::Apply,
        )
        .unwrap();
        (temporary, plan)
    }

    #[test]
    fn controlled_completion_preserves_distinct_success_failure_and_unknown_facts() {
        for (script, expected, completed) in [
            ("exit 0", Completion::Succeeded, 1),
            ("exit 2", Completion::Failed { exit_code: 2 }, 0),
            ("exit 124", Completion::Unknown, 0),
            ("kill -KILL $$", Completion::Unknown, 0),
            ("sleep 5", Completion::Unknown, 0),
        ] {
            let (_directory, plan) = fixture(script);
            let attempt = execute_attempt(&plan, &plan.digest, None).unwrap();
            assert_eq!(attempt.completion, expected, "{script}");
            assert_eq!(attempt.report.completed_steps, completed);
            assert_eq!(
                attempt.report.status == "succeeded",
                expected == Completion::Succeeded
            );
        }
    }

    #[test]
    fn invalid_approval_or_disabled_execution_never_becomes_an_attempt() {
        let (_directory, plan) = fixture("exit 0");
        assert!(execute_attempt(&plan, "wrong", None).is_err());
        let mut value: serde_json::Value = read_json(&plan.environment).unwrap();
        value["execution"]["allow_execution"] = false.into();
        fs::write(&plan.environment, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(execute_attempt(&plan, &plan.digest, None).is_err());
    }

    #[test]
    fn failed_report_retention_preserves_known_command_completion() {
        let (directory, initial) = fixture("exit 0");
        let log_dir = directory.path().join("logs");
        fs::write(
            directory.path().join("driver"),
            format!("#!/bin/sh\n: > '{}/execution.json'\nexit 0\n", log_dir.display()),
        )
        .unwrap();
        let environment = Environment::load(&initial.environment).unwrap();
        let updated = plan(&environment, &initial.manifest, "test", Operation::Apply).unwrap();
        let attempt = execute_attempt(&updated, &updated.digest, Some(&log_dir)).unwrap();
        assert_eq!(attempt.completion, Completion::Succeeded);
        assert_eq!(attempt.report.completed_steps, 1);
        assert!(attempt.reporting_error.is_some());
        assert!(attempt.error.is_none());
        fs::remove_dir_all(&log_dir).unwrap();
        assert!(execute_with_logs(&updated, &updated.digest, Some(&log_dir)).is_err());
    }

    #[test]
    fn bound_metadata_must_be_the_loaded_bytes_even_if_the_file_is_restored() {
        let (directory, initial) = fixture("exit 0");
        let metadata = directory.path().join("recipe/operation.json");
        let loaded = b"{\"operation\":\"apply\"}";
        fs::write(&metadata, loaded).unwrap();
        let expected = digest(loaded);
        fs::write(&metadata, b"{\"operation\":\"check\"}").unwrap();
        let environment = Environment::load(&initial.environment).unwrap();
        let changed = plan(&environment, &initial.manifest, "test", Operation::Apply).unwrap();
        fs::write(&metadata, loaded).unwrap();
        assert!(verify_recipe_member(&changed, &metadata, &expected).is_err());
        let stable = plan(&environment, &initial.manifest, "test", Operation::Apply).unwrap();
        verify_recipe_member(&stable, &metadata, &expected).unwrap();
    }
}
