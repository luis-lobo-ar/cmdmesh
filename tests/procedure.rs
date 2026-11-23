use cmdmesh::{config::Environment, procedure};
use serde_json::{json, Value};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

fn save(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

fn fixture() -> (tempfile::TempDir, Environment, Value) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(".generated/procedure-tests");
    fs::create_dir_all(&root).unwrap();
    let temp = tempfile::tempdir_in(root).unwrap();
    let dir = temp.path();
    for (name, bytes) in [
        ("ansible", b"#!/bin/sh\nexit 0\n".as_slice()),
        ("ssh", b"#!/bin/sh\nexit 0\n"),
        ("identity", b"SYNTHETIC_PRIVATE"),
        ("known_hosts", b"example.invalid synthetic"),
        ("ca", b"SYNTHETIC_CA"),
    ] {
        fs::write(dir.join(name), bytes).unwrap();
        fs::set_permissions(dir.join(name), fs::Permissions::from_mode(0o700)).unwrap();
    }
    let collection = dir.join("package/ansible/ansible_collections/cmdmesh/proxmox/plugins");
    let mut files = serde_json::Map::new();
    for name in [
        "modules/lxc_memory.py",
        "module_utils/memory_state.py",
        "module_utils/memory_remote.py",
    ] {
        let path = collection.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"# synthetic reviewed code\n").unwrap();
        files.insert(
            format!("ansible/ansible_collections/cmdmesh/proxmox/plugins/{name}"),
            json!(procedure::fingerprint(&path).unwrap()),
        );
    }
    save(
        &dir.join("environment.json"),
        &json!({"schema_version":1,
        "targets":{"unrelated":{"inventory":"DO-NOT-READ","vars_file":"DO-NOT-READ","limit":"poisoned_group","lifecycle_owner":"terraform"}},
        "packages":{"cmdmesh-proxmox":"package"},
        "execution":{"allow_execution":false,"allow_mutations":false,"ansible_executable":"ansible","timeout_seconds":23}}),
    );
    let env = Environment::load(&dir.join("environment.json")).unwrap();
    let mut references = serde_json::Map::new();
    for (role, name) in [
        ("ansible-executable", "ansible"),
        ("ssh-executable", "ssh"),
        ("ssh-identity", "identity"),
        ("ssh-known-hosts", "known_hosts"),
        ("tls-ca", "ca"),
    ] {
        let path = dir.join(name);
        references.insert(
            role.into(),
            json!({"path":path,"sha256":procedure::fingerprint(&path).unwrap()}),
        );
    }
    let prepared = json!({"document_type":"cmdmesh.proxmox.lxc-memory.prepared","schema_version":1,"subject_key":"a".repeat(64),"change_required":true,"request":{"selector":{"origin":"example","node":"node-a","kind":"proxmox.lxc","vmid":101}},"desired_memory_mb":512});
    let checks: serde_json::Map<String, Value> = [
        "host-guest-identity",
        "ownership-generation",
        "host-boot-continuity",
        "runtime-continuity",
        "unchanged-state",
        "fresh-complete",
        "configuration-ready",
        "configuration-preimage",
        "capacity-headroom",
    ]
    .into_iter()
    .map(|s| (s.into(), json!(true)))
    .collect();
    let assessment = json!({"document_type":"cmdmesh.procedure.assessment","schema_version":1,"profile_id":"proxmox-lxc-memory","profile_version":1,"phase":"prepare","subject_key":"a".repeat(64),"prepared_sha256":procedure::json_fingerprint(&prepared).unwrap(),"prepared":prepared,"checks":checks,
        "evidence":{"authenticated_current":true,"observation":{},"source":{}},"mutation_authorized":false,"profile_files":files,"references":references,
        "driver":{"executable":dir.join("ansible"),"executable_sha256":procedure::fingerprint(&dir.join("ansible")).unwrap(),"version":"2.19.4","transport":{"policy":"strict-direct-ssh-v1","ssh_executable":dir.join("ssh"),"host":"pve.example.invalid","port":22,"user":"root","identity_file":dir.join("identity"),"known_hosts_file":dir.join("known_hosts"),"python_executable":"/usr/bin/python3"}}});
    (temp, env, assessment)
}

#[test]
fn singleton_context_preserves_policy_and_ignores_legacy_targets_and_vars() {
    let (temp, environment, value) = fixture();
    let assessed = procedure::Assessment::parse(value, "prepare").unwrap();
    let context = temp.path().join("context");
    fs::create_dir(&context).unwrap();
    fs::set_permissions(&context, fs::Permissions::from_mode(0o700)).unwrap();
    let inner = procedure::prepare_context(&environment, &assessed, &context).unwrap();
    assert_eq!(inner.commands.len(), 1);
    let derived = Environment::load(&inner.environment).unwrap();
    assert!(!derived.execution.allow_execution);
    assert!(!derived.execution.allow_mutations);
    assert_eq!(derived.execution.timeout_seconds, 23);
    assert_eq!(derived.targets.len(), 1);
    let selected = &derived.targets["subject"];
    assert_eq!(selected.limit, "cmdmesh_subject");
    let inventory: Value =
        serde_json::from_slice(&fs::read(derived.resolve(&selected.inventory)).unwrap()).unwrap();
    let hosts = inventory["all"]["hosts"].as_object().unwrap();
    assert_eq!(hosts.len(), 1);
    let host = &hosts["cmdmesh_subject"];
    assert_eq!(host["ansible_host"], "pve.example.invalid");
    assert_eq!(host["ansible_connection"], "ssh");
    assert_eq!(host["ansible_become"], false);
    let args = host["ansible_ssh_common_args"].as_str().unwrap();
    for option in [
        "-F none",
        "IdentityAgent=none",
        "CertificateFile=none",
        "ProxyCommand=none",
        "StrictHostKeyChecking=yes",
        "ControlPath=none",
    ] {
        assert!(args.contains(option), "{option}");
    }
    let vars: Value =
        serde_json::from_slice(&fs::read(derived.resolve(selected.vars_file.as_ref().unwrap())).unwrap())
            .unwrap();
    assert_eq!(
        vars.as_object().unwrap().keys().collect::<Vec<_>>(),
        vec!["cmdmesh_memory_prepared"]
    );
    assert!(!fs::read_to_string(&inner.environment)
        .unwrap()
        .contains("SYNTHETIC_PRIVATE"));
    assert_eq!(
        fs::metadata(&inner.environment).unwrap().permissions().mode() & 0o077,
        0
    );
    cmdmesh::recipe::verify_plan(&inner).unwrap();
    let frozen = derived.resolve(&derived.packages["cmdmesh-proxmox"]);
    assert!(frozen.starts_with(&context));
    for (relative, expected) in &assessed.profile_files {
        assert_eq!(procedure::fingerprint(&frozen.join(relative)).unwrap(), *expected);
    }
    let injected = frozen.join("ansible/ansible_collections/cmdmesh/proxmox/plugins/action/lxc_memory.py");
    fs::create_dir_all(injected.parent().unwrap()).unwrap();
    fs::write(injected, b"# unreviewed plugin\n").unwrap();
    assert!(cmdmesh::recipe::verify_plan(&inner).is_err());
}

#[test]
fn malformed_assessments_cannot_establish_a_plan() {
    let (_temp, _environment, original) = fixture();
    for mode in [
        "hash",
        "check-missing",
        "check-extra",
        "check-false",
        "check-string",
        "authority",
        "subject",
        "profile",
        "unknown",
        "transport",
    ] {
        let mut value = original.clone();
        match mode {
            "hash" => value["prepared_sha256"] = json!("0".repeat(64)),
            "check-missing" => {
                value["checks"].as_object_mut().unwrap().remove("fresh-complete");
            }
            "check-extra" => value["checks"]["anything"] = json!(true),
            "check-false" => value["checks"]["fresh-complete"] = json!(false),
            "check-string" => value["checks"]["fresh-complete"] = json!("true"),
            "authority" => value["mutation_authorized"] = json!(true),
            "subject" => value["subject_key"] = json!("b".repeat(64)),
            "profile" => value["profile_version"] = json!(2),
            "unknown" => value["after"] = json!({}),
            "transport" => value["driver"]["transport"]["extra_args"] = json!("-oStrictHostKeyChecking=no"),
            _ => unreachable!(),
        }
        assert!(procedure::Assessment::parse(value, "prepare").is_err(), "{mode}");
    }
}

#[test]
fn driver_reference_or_collection_drift_is_refused_before_context_creation() {
    for mode in [
        "ansible-mismatch",
        "identity-changed",
        "collection-changed",
        "symlink",
    ] {
        let (temp, env, value) = fixture();
        let assessed = procedure::Assessment::parse(value, "prepare").unwrap();
        match mode {
            "ansible-mismatch" => {
                let path = temp.path().join("other-ansible");
                fs::write(&path, b"other").unwrap();
                let mut cfg: Value = serde_json::from_slice(&fs::read(&env.path).unwrap()).unwrap();
                cfg["execution"]["ansible_executable"] = json!(path);
                save(&env.path, &cfg);
            }
            "identity-changed" => fs::write(temp.path().join("identity"), b"changed").unwrap(),
            "collection-changed" => fs::write(
                temp.path().join(
                    "package/ansible/ansible_collections/cmdmesh/proxmox/plugins/modules/lxc_memory.py",
                ),
                b"changed",
            )
            .unwrap(),
            "symlink" => {
                let path = temp.path().join(
                    "package/ansible/ansible_collections/cmdmesh/proxmox/plugins/modules/lxc_memory.py",
                );
                fs::remove_file(&path).unwrap();
                std::os::unix::fs::symlink(temp.path().join("identity"), path).unwrap();
            }
            _ => unreachable!(),
        }
        let current = Environment::load(&env.path).unwrap();
        let context = temp.path().join("context");
        fs::create_dir(&context).unwrap();
        assert!(
            procedure::prepare_context(&current, &assessed, &context).is_err(),
            "{mode}"
        );
        assert!(fs::read_dir(context).unwrap().next().is_none(), "{mode}");
    }
}

#[test]
fn verify_false_checks_remain_an_assessment_but_wrong_phase_does_not() {
    let (_temp, _env, mut value) = fixture();
    value["phase"] = json!("verify");
    value["driver"] = Value::Null;
    value["references"]
        .as_object_mut()
        .unwrap()
        .remove("ansible-executable");
    value["checks"]
        .as_object_mut()
        .unwrap()
        .remove("configuration-preimage");
    value["checks"]
        .as_object_mut()
        .unwrap()
        .remove("capacity-headroom");
    for key in [
        "configuration-target",
        "enforced-memory-target",
        "configuration-digest-transition",
    ] {
        value["checks"][key] = json!(false);
    }
    assert!(!procedure::Assessment::parse(value.clone(), "verify")
        .unwrap()
        .satisfied());
    assert!(procedure::Assessment::parse(value, "preflight").is_err());
}

#[test]
fn actual_ansible_inventory_selects_only_the_literal_subject() {
    let Some(ansible) = std::env::var_os("CMDMESH_TEST_ANSIBLE") else {
        return;
    };
    let inventory_tool = Path::new(&ansible).parent().unwrap().join("ansible-inventory");
    let (temp, environment, value) = fixture();
    let assessment = procedure::Assessment::parse(value, "prepare").unwrap();
    let context = temp.path().join("context");
    fs::create_dir(&context).unwrap();
    fs::set_permissions(&context, fs::Permissions::from_mode(0o700)).unwrap();
    let inner = procedure::prepare_context(&environment, &assessment, &context).unwrap();
    fs::write(context.join("ansible.cfg"), "[defaults]\n").unwrap();
    let output = std::process::Command::new(inventory_tool)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .env("ANSIBLE_CONFIG", context.join("ansible.cfg"))
        .env("ANSIBLE_LOCAL_TEMP", context.join("ansible-local"))
        .env("TMPDIR", &context)
        .current_dir(&context)
        .args(["--inventory", inner.commands[0].argv[1].as_str(), "--list"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    let hosts = result["_meta"]["hostvars"].as_object().unwrap();
    assert_eq!(hosts.len(), 1);
    assert!(hosts.contains_key("cmdmesh_subject"));
    assert_eq!(hosts["cmdmesh_subject"]["ansible_connection"], "ssh");
    assert_eq!(hosts["cmdmesh_subject"]["ansible_become"], false);
    assert_eq!(hosts["cmdmesh_subject"]["ansible_host"], "pve.example.invalid");
}

#[test]
fn actual_ansible_cannot_intercept_the_fixed_module_with_extra_collection_plugins() {
    let Some(ansible) = std::env::var_os("CMDMESH_TEST_ANSIBLE") else {
        return;
    };
    for mode in ["action", "routing", "sys-path"] {
        let (temp, _environment, mut value) = fixture();
        let dir = temp.path();
        let marker = dir.join("intercepted");
        let package = dir.join("package/ansible/ansible_collections/cmdmesh/proxmox");
        let shadow = dir.join("installed-shadow");
        let collection = if mode == "sys-path" {
            shadow.join("ansible_collections/cmdmesh/proxmox")
        } else {
            package.clone()
        };
        let action = collection.join(if mode == "routing" {
            "plugins/action/redirected.py"
        } else {
            "plugins/action/lxc_memory.py"
        });
        fs::create_dir_all(action.parent().unwrap()).unwrap();
        fs::write(&action,format!("from pathlib import Path\nfrom ansible.plugins.action import ActionBase\nclass ActionModule(ActionBase):\n def run(self,tmp=None,task_vars=None):\n  Path({:?}).write_text('intercepted')\n  return {{'changed':False}}\n",marker.to_string_lossy())).unwrap();
        if mode == "routing" {
            fs::create_dir_all(package.join("meta")).unwrap();
            fs::write(
                package.join("meta/runtime.yml"),
                "plugin_routing:\n  modules:\n    lxc_memory:\n      redirect: cmdmesh.proxmox.redirected\n",
            )
            .unwrap();
        }
        let executable = if mode == "sys-path" {
            let wrapper = dir.join("ansible-wrapper");
            let original = fs::read_to_string(Path::new(&ansible)).unwrap();
            let shebang = original.lines().next().unwrap();
            fs::write(&wrapper,format!("{shebang}\nimport sys\nsys.path.insert(0,{:?})\nfrom ansible.cli.playbook import main\nmain()\n",shadow.to_string_lossy())).unwrap();
            fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
            wrapper
        } else {
            Path::new(&ansible).to_path_buf()
        };
        let mut config: Value =
            serde_json::from_slice(&fs::read(dir.join("environment.json")).unwrap()).unwrap();
        config["execution"]["ansible_executable"] = json!(executable);
        config["execution"]["allow_execution"] = json!(true);
        config["execution"]["allow_mutations"] = json!(true);
        save(&dir.join("environment.json"), &config);
        value["driver"]["executable"] = json!(executable);
        value["driver"]["executable_sha256"] = json!(procedure::fingerprint(&executable).unwrap());
        value["references"]["ansible-executable"] =
            json!({"path":executable,"sha256":procedure::fingerprint(&executable).unwrap()});
        let environment = Environment::load(&dir.join("environment.json")).unwrap();
        let assessment = procedure::Assessment::parse(value, "prepare").unwrap();
        let context = dir.join("context");
        fs::create_dir(&context).unwrap();
        fs::set_permissions(&context, fs::Permissions::from_mode(0o700)).unwrap();
        let inner = procedure::prepare_context(&environment, &assessment, &context).unwrap();
        let result = cmdmesh::recipe::execute(&inner, &inner.digest);
        assert!(!marker.exists(), "{mode} intercepted the profiled module");
        assert!(
            result.is_err(),
            "{mode} synthetic transport/module cannot legitimately succeed"
        );
        let generated = Environment::load(&inner.environment).unwrap();
        let frozen = generated.resolve(&generated.packages["cmdmesh-proxmox"]);
        assert!(frozen.starts_with(&context));
        assert!(!frozen
            .join("ansible/ansible_collections/cmdmesh/proxmox/plugins/action")
            .exists());
        assert!(!frozen
            .join("ansible/ansible_collections/cmdmesh/proxmox/meta")
            .exists());
    }
}
