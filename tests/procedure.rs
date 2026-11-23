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

fn executor_fixture(mode: &str) -> (tempfile::TempDir, Environment, Value, std::path::PathBuf) {
    let (temp, _, mut assessment) = fixture();
    let dir = temp.path();
    let marker = dir.join("desired");
    let count = dir.join("dispatch-count");
    let script=format!("#!/usr/bin/python3\nimport sys\nfrom pathlib import Path\np=Path({count:?});p.write_text(p.read_text()+'x' if p.exists() else 'x')\nmode={mode:?}\nif mode not in ('no-change','failure'): Path({marker:?}).touch()\nif mode=='report-failure':\n v=next(v for v in sys.argv if v.startswith('@'))\n (Path(v[1:]).parent.parent/'driver-logs'/'execution.json').touch()\nif mode in ('final-failure','command-failure'):\n v=next(v for v in sys.argv if v.startswith('@'))\n (Path(v[1:]).parent.parent/('result.json' if mode=='final-failure' else 'command.json')).mkdir()\nif mode=='delete-source': Path({inspector:?}).unlink()\nif mode=='unknown':\n import os,signal;os.kill(os.getpid(),signal.SIGKILL)\nsys.exit(2 if mode.startswith('failure') else 0)\n",count=count.to_string_lossy(),marker=marker.to_string_lossy(),inspector=dir.join("inspector").to_string_lossy());
    let facts_code="import json,hashlib\nv=next(v for v in sys.argv if v.startswith('@'))\np=json.loads(Path(v[1:]).read_text())['cmdmesh_memory_prepared']\nfacts={'document_type':'cmdmesh.proxmox.lxc-memory.driver-facts','schema_version':1,'prepared_sha256':hashlib.sha256(json.dumps(p,sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()).hexdigest(),'subject_key':p['subject_key'],'command_outcome':'unknown' if mode=='driver-unknown' else ('failed' if mode.startswith('failure') else 'succeeded'),'mutation_attempted':True,'module_failed':mode.startswith('failure') or mode=='driver-unknown'}\nif mode=='driver-malformed': facts['subject_key']='wrong'\nif mode!='driver-missing':\n target=Path(v[1:]).parent/'driver-facts.json';target.write_text(json.dumps(facts));target.chmod(0o600)\n";
    let script = script
        .replace(
            "if mode=='report-failure':",
            &format!("{facts_code}if mode=='report-failure':"),
        )
        .replace(
            "sys.exit(2 if mode.startswith('failure') else 0)",
            "sys.exit(2 if mode.startswith('failure') or mode=='driver-unknown' else 0)",
        );
    fs::write(dir.join("ansible"), script).unwrap();
    let hash = procedure::fingerprint(&dir.join("ansible")).unwrap();
    assessment["driver"]["executable_sha256"] = json!(hash);
    assessment["references"]["ansible-executable"]["sha256"] = json!(hash);
    assessment["prepared"]["capacity_policy"] =
        json!({"mode":"observed-available-headroom","minimum_remaining_bytes":0});
    if mode == "noop" {
        assessment["prepared"]["change_required"] = json!(false);
    }
    assessment["prepared_sha256"] = json!(procedure::json_fingerprint(&assessment["prepared"]).unwrap());
    let inspector = dir.join("inspector");
    fs::write(&inspector,r#"#!/usr/bin/python3
import copy,json,sys
from pathlib import Path
r=json.load(sys.stdin);c=r['config'];phase=r['input']['phase'];a=copy.deepcopy(c['assessment']);a['phase']=phase
Path(c['calls']).open('a').write(phase+'\n')
outcome='observed';diagnostics=[]
if phase=='verify' and c['mode']=='hold-postcheck':
 import time
 Path(c['post_started']).touch()
 for _ in range(500):
  if Path(c['post_release']).exists():break
  time.sleep(0.01)
if phase=='verify':
 a['driver']=None;a['references'].pop('ansible-executable')
 a['checks'].pop('configuration-preimage');a['checks'].pop('capacity-headroom')
 for key in ('configuration-target','enforced-memory-target','configuration-digest-transition'):a['checks'][key]=Path(c['desired']).exists()
 if c['mode']=='wrong-subject':a['subject_key']='b'*64
 if not all(a['checks'].values()):outcome='incomplete'
if (phase=='prepare' and c['mode']=='prepare-reject') or (phase=='preflight' and c['mode']=='preflight-reject') or (phase=='verify' and c['mode']=='lost-source'):
 a={};outcome='unavailable'
if outcome!='observed':diagnostics=[{'code':'synthetic-unqualified','summary':'Synthetic negative case','next_checks':['Inspect fixture'],'recovery':['Reconcile without retry']}]
print(json.dumps({'document_type':'cmdmesh.inspection.result','schema_version':1,'operation':r['operation'],'outcome':outcome,'data':a,'diagnostics':diagnostics}))
sys.exit(0 if outcome=='observed' else 1)
"#).unwrap();
    fs::set_permissions(&inspector, fs::Permissions::from_mode(0o700)).unwrap();
    let mut config: Value = serde_json::from_slice(&fs::read(dir.join("environment.json")).unwrap()).unwrap();
    config["execution"]["allow_execution"] = json!(true);
    config["execution"]["allow_mutations"] = json!(true);
    config["providers"] = json!({"native":{"executable":inspector,"expected_sha256":procedure::fingerprint(&inspector).unwrap(),"config":{"assessment":assessment,"desired":marker,"calls":dir.join("calls"),"mode":mode,"post_started":dir.join("post-started"),"post_release":dir.join("post-release")}}});
    save(&dir.join("environment.json"), &config);
    let environment = Environment::load(&dir.join("environment.json")).unwrap();
    let parameters = json!({"request":assessment["prepared"]["request"],"desired_memory_mb":512,"capacity_policy":assessment["prepared"]["capacity_policy"]});
    let state = dir.join("state");
    fs::create_dir(&state).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    (temp, environment, parameters, state)
}

#[test]
fn fixed_executor_distinguishes_actual_postconditions_from_command_facts() {
    for (mode, wanted) in [
        ("success", "verified"),
        ("no-change", "unverified"),
        ("failure", "unverified"),
        ("failure-changed", "unverified"),
        ("unknown", "uncertain"),
        ("driver-unknown", "uncertain"),
        ("driver-missing", "uncertain"),
        ("driver-malformed", "uncertain"),
        ("lost-source", "uncertain"),
        ("wrong-subject", "uncertain"),
        ("noop", "checked"),
        ("preflight-reject", "rejected"),
        ("report-failure", "verified"),
        ("delete-source", "verified"),
    ] {
        let (dir, environment, parameters, state) = executor_fixture(mode);
        let plan = procedure::plan(&environment, "native", parameters, &state).unwrap();
        let result = procedure::execute(&plan, &plan.digest).unwrap();
        assert_eq!(
            serde_json::to_value(&result).unwrap()["outcome"],
            wanted,
            "{mode}"
        );
        assert!(!result.current_observation);
        assert_eq!(procedure::status(&plan).unwrap().outcome, result.outcome);
        if mode == "unknown" {
            assert_eq!(result.postconditions_satisfied, Some(true));
        }
        if ["noop", "preflight-reject"].contains(&mode) {
            assert!(!dir.path().join("dispatch-count").exists());
        } else {
            assert_eq!(
                fs::read_to_string(dir.path().join("dispatch-count")).unwrap(),
                "x"
            );
        }
        assert!(
            procedure::execute(&plan, &plan.digest).is_err(),
            "{mode} plan must not run again"
        );
    }
}

#[test]
fn drift_approval_lock_and_consumed_intent_stop_before_dispatch() {
    for mode in ["approval", "identity", "source", "lock", "consumed"] {
        let (dir, environment, parameters, state) = executor_fixture("success");
        let plan = procedure::plan(&environment, "native", parameters, &state).unwrap();
        let journal = procedure::Journal::open(&state).unwrap();
        let held = if mode == "lock" {
            Some(journal.lock(&plan.assessment.subject_key).unwrap())
        } else {
            None
        };
        if mode == "identity" {
            fs::write(dir.path().join("identity"), b"changed").unwrap();
        }
        if mode == "source" {
            fs::write(dir.path().join("inspector"), b"changed").unwrap();
        }
        if mode == "consumed" {
            journal
                .lock(&plan.assessment.subject_key)
                .unwrap()
                .consume(&plan.digest)
                .unwrap();
        }
        let approval = if mode == "approval" { "wrong" } else { &plan.digest };
        assert!(procedure::execute(&plan, approval).is_err(), "{mode}");
        assert!(!dir.path().join("dispatch-count").exists(), "{mode}");
        drop(held);
    }
}

#[test]
fn missing_final_report_does_not_erase_consumption_or_enable_replay() {
    let (dir, environment, parameters, state) = executor_fixture("final-failure");
    let plan = procedure::plan(&environment, "native", parameters, &state).unwrap();
    assert!(procedure::execute(&plan, &plan.digest).is_err());
    assert!(procedure::Journal::open(&state)
        .unwrap()
        .consumed(&plan.digest)
        .unwrap());
    let status = procedure::status(&plan).unwrap();
    assert_eq!(serde_json::to_value(status).unwrap()["outcome"], "uncertain");
    assert!(procedure::execute(&plan, &plan.digest).is_err());
    assert_eq!(
        fs::read_to_string(dir.path().join("dispatch-count")).unwrap(),
        "x"
    );
}

#[test]
fn unchanged_intent_does_not_require_mutation_permission() {
    let (_dir, environment, parameters, state) = executor_fixture("noop");
    let mut config: Value = serde_json::from_slice(&fs::read(&environment.path).unwrap()).unwrap();
    config["execution"]["allow_mutations"] = json!(false);
    save(&environment.path, &config);
    let plan = procedure::plan(&environment, "native", parameters, &state).unwrap();
    let result = procedure::execute(&plan, &plan.digest).unwrap();
    assert_eq!(serde_json::to_value(result).unwrap()["outcome"], "checked");
}

#[test]
fn status_after_an_incomplete_predispatch_assessment_does_not_claim_ready() {
    let (_dir, environment, parameters, state) = executor_fixture("noop");
    let plan = procedure::plan(&environment, "native", parameters, &state).unwrap();
    fs::write(plan.context_directory.join("preflight.json"), b"partial").unwrap();
    assert!(procedure::execute(&plan, &plan.digest).is_err());
    assert_eq!(
        serde_json::to_value(procedure::status(&plan).unwrap()).unwrap()["outcome"],
        "pending"
    );
}

#[test]
fn failed_command_evidence_retention_still_attempts_independent_postcheck() {
    let (dir, environment, parameters, state) = executor_fixture("command-failure");
    let plan = procedure::plan(&environment, "native", parameters, &state).unwrap();
    assert!(procedure::execute(&plan, &plan.digest).is_err());
    assert!(fs::read_to_string(dir.path().join("calls"))
        .unwrap()
        .contains("verify"));
    assert!(plan.context_directory.join("verify.json").is_file());
    assert!(procedure::Journal::open(&state)
        .unwrap()
        .consumed(&plan.digest)
        .unwrap());
    assert!(procedure::execute(&plan, &plan.digest).is_err());
}

#[test]
fn failed_preparation_retains_the_native_diagnostic_receipt() {
    let (_dir, environment, parameters, state) = executor_fixture("prepare-reject");
    let error = procedure::plan(&environment, "native", parameters, &state).unwrap_err();
    assert!(error.to_string().contains("receipt"));
    let retained = fs::read_dir(state.join("plans"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let receipt: Value = serde_json::from_slice(&fs::read(retained.join("prepare.json")).unwrap()).unwrap();
    assert_eq!(
        receipt["result"]["diagnostics"][0]["code"],
        "synthetic-unqualified"
    );
}

#[test]
fn actual_ansible_retains_unknown_native_facts_without_returning_raw_module_output() {
    let Some(ansible) = std::env::var_os("CMDMESH_TEST_ANSIBLE") else {
        return;
    };
    let ansible = Path::new(&ansible).to_path_buf();
    let (temp, _environment, mut value) = fixture();
    let dir = temp.path();
    let module =
        dir.join("package/ansible/ansible_collections/cmdmesh/proxmox/plugins/modules/lxc_memory.py");
    fs::write(&module,"#!/usr/bin/python\nfrom ansible.module_utils.basic import AnsibleModule\nm=AnsibleModule(argument_spec={'prepared':{'type':'dict','required':True},'allow_changes':{'type':'bool'}})\nm.fail_json(msg='SYNTHETIC_SENSITIVE_RESULT',changed=True,command_outcome='unknown',mutation_attempted=True)\n").unwrap();
    value["profile_files"]["ansible/ansible_collections/cmdmesh/proxmox/plugins/modules/lxc_memory.py"] =
        json!(procedure::fingerprint(&module).unwrap());
    let mut config: Value = serde_json::from_slice(&fs::read(dir.join("environment.json")).unwrap()).unwrap();
    config["execution"]["ansible_executable"] = json!(ansible);
    config["execution"]["allow_execution"] = json!(true);
    config["execution"]["allow_mutations"] = json!(true);
    save(&dir.join("environment.json"), &config);
    value["driver"]["executable"] = json!(ansible);
    value["driver"]["executable_sha256"] = json!(procedure::fingerprint(Path::new(&ansible)).unwrap());
    value["references"]["ansible-executable"] =
        json!({"path":ansible,"sha256":procedure::fingerprint(Path::new(&ansible)).unwrap()});
    let environment = Environment::load(&dir.join("environment.json")).unwrap();
    let assessed = procedure::Assessment::parse(value, "prepare").unwrap();
    let context = dir.join("context");
    fs::create_dir(&context).unwrap();
    fs::set_permissions(&context, fs::Permissions::from_mode(0o700)).unwrap();
    let original = procedure::prepare_context(&environment, &assessed, &context).unwrap();
    let inventory = context.join("inventory.json");
    let mut values: Value = serde_json::from_slice(&fs::read(&inventory).unwrap()).unwrap();
    values["all"]["hosts"]["cmdmesh_subject"]["ansible_connection"] = json!("local");
    save(&inventory, &values);
    let derived = Environment::load(&original.environment).unwrap();
    let local = cmdmesh::recipe::plan(
        &derived,
        &original.manifest,
        "subject",
        cmdmesh::recipe::Operation::Apply,
    )
    .unwrap();
    assert!(cmdmesh::recipe::execute(&local, &local.digest).is_err());
    let facts = fs::read_to_string(context.join("driver-facts.json")).unwrap();
    assert!(!facts.contains("SYNTHETIC_SENSITIVE_RESULT"));
    let facts: Value = serde_json::from_str(&facts).unwrap();
    assert_eq!(facts["command_outcome"], "unknown");
    assert_eq!(facts["mutation_attempted"], true);
    assert_eq!(facts["module_failed"], true);
}

#[test]
fn cli_procedure_plans_execute_only_by_digest_and_return_negative_outcomes_nonzero() {
    for (mode, outcome, success) in [
        ("success", "verified", true),
        ("no-change", "unverified", false),
        ("driver-unknown", "uncertain", false),
    ] {
        let (dir, environment, parameters, state) = executor_fixture(mode);
        let input = dir.path().join("parameters.json");
        save(&input, &parameters);
        let output = dir.path().join("plan.json");
        let prepared = std::process::Command::new(env!("CARGO_BIN_EXE_cmdmesh"))
            .arg("--config")
            .arg(&environment.path)
            .args(["procedure", "plan", "--inspector", "native", "--input"])
            .arg(&input)
            .arg("--state-directory")
            .arg(&state)
            .arg("--out")
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            prepared.status.success(),
            "{}",
            String::from_utf8_lossy(&prepared.stderr)
        );
        let plan: Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
        assert_eq!(fs::metadata(&output).unwrap().permissions().mode() & 0o077, 0);
        let executed = std::process::Command::new(env!("CARGO_BIN_EXE_cmdmesh"))
            .args(["procedure", "execute", "--plan"])
            .arg(&output)
            .arg("--approve-digest")
            .arg(plan["digest"].as_str().unwrap())
            .output()
            .unwrap();
        assert_eq!(
            executed.status.success(),
            success,
            "{mode}:{}",
            String::from_utf8_lossy(&executed.stderr)
        );
        let report: Value = serde_json::from_slice(&executed.stdout).unwrap();
        assert_eq!(report["outcome"], outcome);
        let historical = std::process::Command::new(env!("CARGO_BIN_EXE_cmdmesh"))
            .args(["procedure", "status", "--plan"])
            .arg(&output)
            .output()
            .unwrap();
        assert!(historical.status.success());
        let historical: Value = serde_json::from_slice(&historical.stdout).unwrap();
        assert_eq!(historical, report);
    }
}

#[test]
fn physical_subject_lock_remains_held_through_independent_postchecks() {
    use std::{thread, time::Duration};
    let (dir, environment, parameters, state) = executor_fixture("hold-postcheck");
    let first = procedure::plan(&environment, "native", parameters.clone(), &state).unwrap();
    let second = procedure::plan(&environment, "native", parameters, &state).unwrap();
    let active = thread::spawn(move || procedure::execute(&first, &first.digest));
    let ready = dir.path().join("post-started");
    for _ in 0..300 {
        if ready.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let reached = ready.exists();
    let blocked = procedure::execute(&second, &second.digest);
    fs::write(dir.path().join("post-release"), b"release").unwrap();
    let result = active.join().unwrap().unwrap();
    assert!(reached);
    assert!(blocked.is_err());
    assert_eq!(result.outcome, procedure::Outcome::Verified);
    assert_eq!(
        fs::read_to_string(dir.path().join("dispatch-count")).unwrap(),
        "x"
    );
}

fn operation_fixture(
    root: &Path,
    parameters: Value,
    state: &Path,
) -> (std::path::PathBuf, Value, std::path::PathBuf) {
    let catalog = root.join("catalog");
    let directory = catalog.join("operations/owned-lxc-memory");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("operation.json");
    let mut fields = serde_json::Map::new();
    for (name, kind) in [
        ("inspector", "inspector"),
        ("state_directory", "state-directory"),
        ("request", "memory-request"),
        ("desired_memory_mb", "memory-mb"),
        ("capacity_policy", "memory-capacity-policy"),
    ] {
        fields.insert(
            name.into(),
            json!({"type":kind,"required":true,"description":"Explicit private profile input"}),
        );
    }
    save(
        &path,
        &json!({"document_type":"cmdmesh.operation","schema_version":2,"operation_version":1,"id":"owned-lxc-memory","aliases":["grow owned LXC memory"],"title":"Owned memory growth","summary":"Verify one fixed guarded memory change","parameters":fields,"requirements":[{"capability":"verified-procedure","protocol_version":1,"supported_tools":{"cmdmesh":["0.3.0"]}}],"entrypoint":{"driver":"procedure-plan","profile":"proxmox-lxc-memory","profile_version":1},"expected_results":["Independent memory postconditions"],"known_failures":[{"code":"uncertain","summary":"Completion is unknown","next_checks":["Inspect phase receipts"],"recovery":["Reconcile without replay"]}],"provenance":{"references":["https://example.invalid/qualified-profile"],"qualification":"Synthetic composition test only","limitations":["No real infrastructure qualification"]}}),
    );
    let mut values = parameters;
    values["inspector"] = json!("native");
    values["state_directory"] = json!(state);
    (catalog, values, path)
}

#[test]
fn operation_discovery_prepares_the_fixed_procedure_and_binds_exact_metadata() {
    use cmdmesh::operations;
    let (dir, environment, parameters, state) = executor_fixture("success");
    let (catalog, values, path) = operation_fixture(dir.path(), parameters, &state);
    let entry = operations::inspect(&catalog, "grow owned LXC memory").unwrap();
    assert_eq!(entry.schema_version, 2);
    let validation = operations::validate_inputs(&entry, &values, Some(&environment)).unwrap();
    assert!(validation.valid);
    assert!(!validation.requirements_evaluated);
    assert!(!dir.path().join("calls").exists());
    let prepared = operations::prepare(&environment, &catalog, "owned-lxc-memory", &values).unwrap();
    let operations::PreparedPlan::Procedure(plan) = prepared else {
        panic!("expected fixed procedure plan")
    };
    let source = plan.operation_source.as_ref().unwrap();
    assert_eq!(source.id, "owned-lxc-memory");
    assert_eq!(source.document.path, path.canonicalize().unwrap());
    fs::write(&path, b"changed after review").unwrap();
    assert!(procedure::execute(&plan, &plan.digest).is_err());
    assert!(!dir.path().join("dispatch-count").exists());
}

#[test]
fn operation_profile_rejects_unknown_inputs_versions_and_drivers_without_running_code() {
    use cmdmesh::operations;
    let (dir, environment, parameters, state) = executor_fixture("success");
    let (catalog, values, path) = operation_fixture(dir.path(), parameters, &state);
    let entry = operations::inspect(&catalog, "owned-lxc-memory").unwrap();
    for mode in ["extra", "inspector", "memory", "state"] {
        let mut invalid = values.clone();
        match mode {
            "extra" => invalid["execute"] = json!(true),
            "inspector" => invalid["inspector"] = json!("missing"),
            "memory" => invalid["desired_memory_mb"] = json!(true),
            _ => invalid["state_directory"] = json!(false),
        };
        assert!(
            !operations::validate_inputs(&entry, &invalid, Some(&environment))
                .unwrap()
                .valid,
            "{mode}"
        );
    }
    let original: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for mode in ["schema", "profile", "driver", "parameter"] {
        let mut invalid = original.clone();
        match mode {
            "schema" => invalid["schema_version"] = json!(1),
            "profile" => invalid["entrypoint"]["profile_version"] = json!(2),
            "driver" => invalid["entrypoint"]["driver"] = json!("shell"),
            _ => {
                invalid["parameters"]["target"] =
                    json!({"type":"target","required":true,"description":"Unexpected"})
            }
        };
        save(&path, &invalid);
        assert!(
            operations::inspect(&catalog, "owned-lxc-memory").is_err(),
            "{mode}"
        );
    }
    assert!(!dir.path().join("calls").exists());
}

#[test]
fn cli_operation_plans_a_real_bound_procedure_instead_of_a_legacy_recipe() {
    let (dir, environment, parameters, state) = executor_fixture("success");
    let (catalog, values, _) = operation_fixture(dir.path(), parameters, &state);
    let input = dir.path().join("operation-inputs.json");
    save(&input, &values);
    let out = dir.path().join("operation-plan.json");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_cmdmesh"))
        .arg("--config")
        .arg(&environment.path)
        .args(["operation", "plan", "grow owned LXC memory", "--catalog"])
        .arg(&catalog)
        .arg("--inputs")
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let plan: Value = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(plan["document_type"], "cmdmesh.procedure.plan");
    assert_eq!(plan["operation_source"]["id"], "owned-lxc-memory");
    let execution = std::process::Command::new(env!("CARGO_BIN_EXE_cmdmesh"))
        .args(["procedure", "execute", "--plan"])
        .arg(&out)
        .arg("--approve-digest")
        .arg(plan["digest"].as_str().unwrap())
        .output()
        .unwrap();
    assert!(
        execution.status.success(),
        "{}",
        String::from_utf8_lossy(&execution.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&execution.stdout).unwrap()["outcome"],
        "verified"
    );
}

#[test]
fn mcp_operation_plan_dispatches_the_profile_with_its_original_private_parameters() {
    use std::{
        io::{BufRead, BufReader, Write},
        process::{Command, Stdio},
        sync::mpsc,
        thread,
        time::Duration,
    };
    let (dir, environment, parameters, state) = executor_fixture("success");
    let (catalog, values, _) = operation_fixture(dir.path(), parameters, &state);
    let input = dir.path().join("operation-inputs.json");
    save(&input, &values);
    let mut child = Command::new(env!("CARGO_BIN_EXE_cmdmesh"))
        .arg("--config")
        .arg(&environment.path)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let mut request = |value: Value| {
        writeln!(stdin, "{value}").unwrap();
        stdin.flush().unwrap();
    };
    let receive =
        || serde_json::from_str::<Value>(&rx.recv_timeout(Duration::from_secs(20)).unwrap()).unwrap();
    request(
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"operation-profile-test","version":"1"}}}),
    );
    receive();
    request(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    request(
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"cmdmesh_operation","arguments":{"action":"plan","catalog":catalog,"operation":"owned-lxc-memory","inputs":input}}}),
    );
    let response = receive();
    let result = response["result"]["content"][0]["text"]
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    child.kill().unwrap();
    child.wait().unwrap();
    let plan = result.unwrap_or_else(|| panic!("{response}"));
    assert_eq!(plan["document_type"], "cmdmesh.procedure.plan");
    assert_eq!(plan["operation_source"]["id"], "owned-lxc-memory");
    assert_eq!(plan["assessment"]["prepared"]["desired_memory_mb"], 512);
    assert!(!dir.path().join("dispatch-count").exists());
}

#[test]
fn mcp_procedure_prepares_the_same_cli_consumable_plan_but_has_no_execute_action() {
    use std::{
        io::{BufRead, BufReader, Write},
        process::{Command, Stdio},
        sync::mpsc,
        thread,
        time::Duration,
    };
    let (dir, environment, parameters, state) = executor_fixture("success");
    let input = dir.path().join("parameters.json");
    save(&input, &parameters);
    let mut child = Command::new(env!("CARGO_BIN_EXE_cmdmesh"))
        .arg("--config")
        .arg(&environment.path)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let mut request = |value: Value| {
        writeln!(stdin, "{value}").unwrap();
        stdin.flush().unwrap();
    };
    let receive =
        || serde_json::from_str::<Value>(&rx.recv_timeout(Duration::from_secs(15)).unwrap()).unwrap();
    request(
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"procedure-test","version":"1"}}}),
    );
    receive();
    request(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    request(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}));
    let tools = receive();
    let tool = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "cmdmesh_procedure")
        .unwrap();
    assert_eq!(tool["annotations"]["readOnlyHint"], false);
    assert_eq!(tool["annotations"]["destructiveHint"], false);
    assert_eq!(tool["annotations"]["openWorldHint"], true);
    request(
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"cmdmesh_procedure","arguments":{"action":"describe"}}}),
    );
    let description = receive();
    let description: Value =
        serde_json::from_str(description["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let cli_description = Command::new(env!("CARGO_BIN_EXE_cmdmesh"))
        .args(["procedure", "describe"])
        .output()
        .unwrap();
    assert!(cli_description.status.success());
    assert_eq!(
        description,
        serde_json::from_slice::<Value>(&cli_description.stdout).unwrap()
    );
    request(
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"cmdmesh_procedure","arguments":{"action":"plan","inspector":"native","inputs":input,"state_directory":state}}}),
    );
    let planned = receive();
    let plan: Value =
        serde_json::from_str(planned["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    request(
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"cmdmesh_procedure","arguments":{"action":"execute","approve_digest":plan["digest"]}}}),
    );
    let denied = receive();
    assert!(denied["error"].is_object() || denied["result"]["isError"] == true);
    child.kill().unwrap();
    child.wait().unwrap();
    let retained = Path::new(plan["context_directory"].as_str().unwrap()).join("plan.json");
    let executed = Command::new(env!("CARGO_BIN_EXE_cmdmesh"))
        .args(["procedure", "execute", "--plan"])
        .arg(retained)
        .arg("--approve-digest")
        .arg(plan["digest"].as_str().unwrap())
        .output()
        .unwrap();
    assert!(
        executed.status.success(),
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&executed.stdout).unwrap()["outcome"],
        "verified"
    );
}
