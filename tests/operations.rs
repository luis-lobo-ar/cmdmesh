use cmdmesh::{config::Environment, operations};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};
use tempfile::TempDir;

fn temporary() -> TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(".generated/tests");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(root)
        .unwrap()
}

fn document() -> Value {
    json!({
        "document_type":"cmdmesh.operation", "schema_version":1, "operation_version":1,
        "id":"local-check", "aliases":["check local tools"], "title":"Local check",
        "summary":"Prepare an explicitly targeted local check without running tools.",
        "parameters":{"target":{"type":"target", "required":true, "description":"Private environment target name"}},
        "requirements":[{"capability":"ansible-recipe", "protocol_version":1,
            "supported_tools":{"ansible-core":["2.20.4"]}}],
        "entrypoint":{"driver":"recipe-plan", "manifest":"recipe.json", "operation":"check"},
        "expected_results":["A static plan; not evidence of host changes"],
        "known_failures":[{"code":"unknown-target", "summary":"The private target does not exist",
            "next_checks":["Inspect the private environment target map"],
            "recovery":["Correct the reference; prepare a new plan"]}],
        "provenance":{"references":["README.md"], "qualification":"synthetic",
            "limitations":["No live infrastructure qualification"]}
    })
}

fn fixture(root: &Path) {
    fs::create_dir_all(root.join("recipes/local")).unwrap();
    fs::write(root.join("recipes/local/README.md"), "Local recovery\n").unwrap();
    fs::write(
        root.join("recipes/local/playbook.yml"),
        "---\n- hosts: all\n  tasks: []\n",
    )
    .unwrap();
    fs::write(
        root.join("recipes/local/recipe.json"),
        json!({
            "schema_version":1,"id":"local-check","title":"Check","summary":"Local check",
            "status":"ready","requirements":[],"dependencies":[],
            "steps":[{"id":"check","driver":"ansible","playbook":"playbook.yml"}],
            "verification":["Review the result"],"recovery":"README.md"
        })
        .to_string(),
    )
    .unwrap();
    save(root, "local", document());
}

fn save(root: &Path, name: &str, value: Value) {
    let path = root.join("recipes").join(name);
    fs::create_dir_all(&path).unwrap();
    if name != "local" {
        for file in ["recipe.json", "README.md", "playbook.yml"] {
            fs::copy(root.join("recipes/local").join(file), path.join(file)).unwrap();
        }
    }
    fs::write(path.join("operation.json"), value.to_string()).unwrap();
}

#[test]
fn discover_by_intent_and_inspect_canonical_operation_without_execution() {
    let dir = temporary();
    fixture(dir.path());
    let result = operations::search(dir.path(), "local tools", 0, 20).unwrap();
    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].id, "local-check");
    let entry = operations::inspect(dir.path(), " CHECK LOCAL TOOLS ").unwrap();
    assert_eq!(entry.id, "local-check");
    assert!(!dir.path().join("plan.json").exists());
    assert!(operations::search(dir.path(), "unknown", 0, 20)
        .unwrap()
        .items
        .is_empty());
}

#[test]
fn catalog_rejects_alias_collisions_unknown_versions_and_unsupported_drivers() {
    let dir = temporary();
    fixture(dir.path());
    let mut other = document();
    other["id"] = json!("other");
    save(dir.path(), "other", other);
    assert!(operations::search(dir.path(), "", 0, 20)
        .unwrap_err()
        .to_string()
        .contains("alias"));
    fs::remove_dir_all(dir.path().join("recipes/other")).unwrap();
    for (field, value) in [
        ("schema_version", json!(2)),
        ("operation_version", json!(0)),
        ("document_type", json!("other")),
        ("unexpected", json!(true)),
    ] {
        let mut entry = document();
        entry[field] = value;
        save(dir.path(), "local", entry);
        assert!(operations::inspect(dir.path(), "local-check").is_err(), "{field}");
    }
    let mut entry = document();
    entry["entrypoint"]["driver"] = json!("shell");
    save(dir.path(), "local", entry);
    assert!(operations::inspect(dir.path(), "local-check").is_err());
}

#[test]
fn exact_private_target_references_validate_without_echoing_values() {
    let dir = temporary();
    fixture(dir.path());
    let path = dir.path().join("environment.json");
    fs::write(
        &path,
        json!({"schema_version":1,"targets":{"local":{
        "inventory":"inventory.yml","limit":"demo","lifecycle_owner":"ansible"}}})
        .to_string(),
    )
    .unwrap();
    let environment = Environment::load(&path).unwrap();
    let entry = operations::inspect(dir.path(), "local-check").unwrap();
    let valid = operations::validate_inputs(&entry, &json!({"target":"local"}), Some(&environment)).unwrap();
    assert!(valid.valid);
    assert_eq!(valid.execution, "not-run");
    for input in [
        json!({}),
        json!({"target":true}),
        json!({"target":"SYNTHETIC_PRIVATE"}),
        json!({"target":"local","extra":"SYNTHETIC_PRIVATE"}),
    ] {
        let report = operations::validate_inputs(&entry, &input, Some(&environment)).unwrap();
        assert!(!report.valid);
        assert!(!serde_json::to_string(&report)
            .unwrap()
            .contains("SYNTHETIC_PRIVATE"));
    }
    assert!(
        !operations::validate_inputs(&entry, &json!({"target":"local"}), None)
            .unwrap()
            .valid
    );
}

#[test]
fn entrypoint_paths_cannot_escape_catalog_or_follow_symlinks() {
    let dir = temporary();
    fixture(dir.path());
    let mut entry = document();
    entry["entrypoint"]["manifest"] = json!("../outside.json");
    save(dir.path(), "local", entry);
    assert!(operations::inspect(dir.path(), "local-check").is_err());
    save(dir.path(), "local", document());
    fs::rename(
        dir.path().join("recipes/local/recipe.json"),
        dir.path().join("saved.json"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        dir.path().join("saved.json"),
        dir.path().join("recipes/local/recipe.json"),
    )
    .unwrap();
    assert!(operations::inspect(dir.path(), "local-check").is_err());
}

#[test]
fn dangling_operation_manifest_is_an_invalid_catalog_not_an_empty_success() {
    let dir = temporary();
    fixture(dir.path());
    let manifest = dir.path().join("recipes/local/operation.json");
    fs::remove_file(&manifest).unwrap();
    std::os::unix::fs::symlink(dir.path().join("missing.json"), &manifest).unwrap();
    assert!(operations::search(dir.path(), "", 0, 20).is_err());
}

#[test]
fn entrypoint_requires_its_real_parameter_contract_and_bounded_discovery() {
    let dir = temporary();
    fixture(dir.path());
    for parameters in [
        json!({}),
        json!({"count":{"type":"integer","required":true,"description":"Count"}}),
        json!({"target":{"type":"target","required":false,"description":"Target"}}),
    ] {
        let mut entry = document();
        entry["parameters"] = parameters;
        save(dir.path(), "local", entry);
        assert!(operations::inspect(dir.path(), "local-check").is_err());
    }
    save(dir.path(), "local", document());
    assert!(operations::search(dir.path(), "", 0, 0).is_err());
    assert!(operations::search(dir.path(), "", 0, 101).is_err());
}

#[test]
fn operation_planning_uses_existing_bound_recipe_contract_including_metadata() {
    let dir = temporary();
    fixture(dir.path());
    fs::write(
        dir.path().join("inventory.yml"),
        "demo ansible_connection=local\n",
    )
    .unwrap();
    let environment_path = dir.path().join("environment.json");
    fs::write(
        &environment_path,
        json!({"schema_version":1,"targets":{"local":{
        "inventory":"inventory.yml","limit":"demo","lifecycle_owner":"ansible"}}})
        .to_string(),
    )
    .unwrap();
    let environment = Environment::load(&environment_path).unwrap();
    let plan = operations::plan(
        &environment,
        dir.path(),
        "check local tools",
        &json!({"target":"local"}),
    )
    .unwrap();
    assert_eq!(plan.operation, cmdmesh::recipe::Operation::Check);
    assert_eq!(plan.schema_version, 1);
    cmdmesh::recipe::verify_plan(&plan).unwrap();
    let mut changed = document();
    changed["summary"] = json!("Changed operational instructions");
    save(dir.path(), "local", changed);
    assert!(cmdmesh::recipe::verify_plan(&plan).is_err());
}

#[test]
fn cli_discovers_operations_and_exits_unsuccessfully_for_invalid_private_inputs() {
    let dir = temporary();
    fixture(dir.path());
    let output = Command::new(env!("CARGO_BIN_EXE_cmdmesh"))
        .args([
            "operation",
            "list",
            "--catalog",
            dir.path().to_str().unwrap(),
            "--query",
            "local tools",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["items"][0]["id"], "local-check");
    let inputs = dir.path().join("inputs.json");
    fs::write(&inputs, json!({"target":"SYNTHETIC_PRIVATE"}).to_string()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cmdmesh"))
        .args([
            "operation",
            "validate-inputs",
            "local-check",
            "--catalog",
            dir.path().to_str().unwrap(),
            "--inputs",
            inputs.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["valid"], false);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("SYNTHETIC_PRIVATE"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("SYNTHETIC_PRIVATE"));
}

#[test]
fn mcp_operation_discovery_matches_cli_and_rejects_mutation_fields() {
    let dir = temporary();
    fixture(dir.path());
    let expected =
        serde_json::to_value(operations::search(dir.path(), "local tools", 0, 20).unwrap()).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_cmdmesh"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let mut input = child.stdin.take().unwrap();
    let mut request = |v: Value| {
        writeln!(input, "{v}").unwrap();
        input.flush().unwrap();
    };
    request(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"synthetic","version":"1"}}}));
    receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    request(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    request(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
        "name":"cmdmesh_operation","arguments":{"action":"list","catalog":dir.path(),"query":"local tools"}}}));
    let response: Value =
        serde_json::from_str(&receiver.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
    let text = response["result"]["content"][0]["text"].as_str();
    let observed = text.and_then(|text| serde_json::from_str::<Value>(text).ok());
    request(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
        "name":"cmdmesh_operation","arguments":{"action":"list","catalog":dir.path(),"execute":true}}}));
    let rejected: Value =
        serde_json::from_str(&receiver.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(observed, Some(expected), "{response}");
    assert!(rejected["error"].is_object() || rejected["result"]["isError"] == true);
}
