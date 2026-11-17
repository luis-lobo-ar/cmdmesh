use cmdmesh::{config::Environment, inventory, recipe};
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt, path::Path};
use tempfile::TempDir;

fn write_json(path: &Path, value: serde_json::Value) {
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}
fn temporary() -> TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join(".generated/tests");
    fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("contracts-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(base)
        .unwrap()
}

#[test]
fn drift_distinguishes_missing_changed_unmanaged_and_incomplete() {
    let desired = json!({"schema_version":1,"resources":[
        {"kind":"proxmox.lxc","id":"node-a/101","spec":{"memory":512}},
        {"kind":"proxmox.lxc","id":"node-a/102","spec":{}}
    ]});
    let actual = json!({"schema_version":1,"complete":false,"warnings":["scope unreadable"],"resources":[
        {"kind":"proxmox.lxc","id":"node-a/101","properties":{"memory":1024,"status":"running"}},
        {"kind":"proxmox.vm","id":"node-a/103","properties":{}}
    ]});
    let report = inventory::drift_values(desired, actual).unwrap();
    assert!(!report.clean);
    assert!(!report.complete);
    assert_eq!(
        report.items.iter().map(|i| i.change.as_str()).collect::<Vec<_>>(),
        vec!["changed", "missing", "unmanaged"]
    );
}

#[test]
fn duplicate_inventory_and_unsupported_versions_are_rejected() {
    let empty = json!({"schema_version":1,"resources":[]});
    let duplicate = json!({"schema_version":1,"complete":true,"warnings":[],"resources":[
        {"kind":"proxmox.lxc","id":"node-a/101","properties":{}},
        {"kind":"proxmox.lxc","id":"node-a/101","properties":{}}
    ]});
    assert!(inventory::drift_values(empty.clone(), duplicate).is_err());
    assert!(inventory::drift_values(
        empty,
        json!({"schema_version":2,"complete":true,"resources":[],"warnings":[]})
    )
    .is_err());
}

#[test]
fn static_plan_binds_dependencies_private_inputs_and_target() {
    let dir = temporary();
    fs::create_dir(dir.path().join("package")).unwrap();
    fs::write(dir.path().join("package/role.yml"), "---\n").unwrap();
    fs::write(
        dir.path().join("inventory.ini"),
        "node-a ansible_host=192.0.2.10\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("playbook.yml"),
        "---\n- hosts: all\n  tasks: []\n",
    )
    .unwrap();
    write_json(
        &dir.path().join("recipe.json"),
        json!({
            "schema_version":1,"id":"baseline","title":"Baseline","summary":"Check a host", "status":"ready",
            "requirements":[],"dependencies":["proxmox"],"steps":[{"id":"baseline","driver":"ansible","playbook":"playbook.yml"}],
            "verification":["Inspect result"],"recovery":"README.md"
        }),
    );
    fs::write(dir.path().join("README.md"), "Recovery: review host state.\n").unwrap();
    write_json(
        &dir.path().join("environment.json"),
        json!({
            "schema_version":1,"providers":{},"packages":{"proxmox":"package"},
            "targets":{"lab":{"inventory":"inventory.ini","limit":"node-a","lifecycle_owner":"terraform"}}
        }),
    );
    let env = Environment::load(&dir.path().join("environment.json")).unwrap();
    let plan = recipe::plan(
        &env,
        &dir.path().join("recipe.json"),
        "lab",
        recipe::Operation::Check,
    )
    .unwrap();
    assert_eq!(plan.target, "lab");
    assert!(!plan.digest.is_empty());
    assert!(!plan.commands.is_empty());
    assert!(recipe::execute(&plan, &plan.digest)
        .unwrap_err()
        .to_string()
        .contains("execution disabled"));
    fs::write(dir.path().join("package/role.yml"), "changed\n").unwrap();
    assert!(recipe::verify_plan(&plan).is_err());
    let manifest_path = dir.path().join("recipe.json");
    let mut manifest: serde_json::Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["requirements"] = json!(["lifecycle-owner:ansible"]);
    write_json(&manifest_path, manifest);
    assert!(
        recipe::plan(&env, &manifest_path, "lab", recipe::Operation::Apply)
            .unwrap_err()
            .to_string()
            .contains("Ansible lifecycle ownership")
    );
}

#[test]
fn recipe_rejects_path_escape_unknown_driver_and_wildcard_targets() {
    let dir = temporary();
    let manifest = json!({"schema_version":1,"id":"bad","title":"Bad","summary":"bad","status":"ready",
        "requirements":[],"dependencies":[],"steps":[{"id":"bad","driver":"shell","playbook":"../escape.yml"}],
        "verification":[],"recovery":"README.md"});
    write_json(&dir.path().join("recipe.json"), manifest);
    assert!(recipe::inspect(&dir.path().join("recipe.json")).is_err());
    write_json(
        &dir.path().join("environment.json"),
        json!({"schema_version":1,"providers":{},"targets":{
            "lab":{"inventory":"inventory.ini","limit":"all","lifecycle_owner":"ansible"}
        }}),
    );
    assert!(Environment::load(&dir.path().join("environment.json")).is_err());
}
