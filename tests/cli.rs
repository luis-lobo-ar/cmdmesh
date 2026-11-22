use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};
use tempfile::TempDir;

fn temporary() -> TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join(".generated/tests");
    fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("cli-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(base)
        .unwrap()
}
fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cmdmesh"))
}
fn write_json(path: &Path, value: Value) {
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}
fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
fn provider_environment(dir: &Path, script: &str) -> PathBuf {
    executable(&dir.join("provider"), script);
    let environment = dir.join("environment.json");
    write_json(
        &environment,
        json!({"schema_version":1,"providers":{"proxmox":{"executable":"provider","config":{},"env_allowlist":[]}}}),
    );
    environment
}

#[test]
fn cli_inventory_accepts_native_protocol_and_writes_private_no_clobber_file() {
    let dir = temporary();
    let environment = provider_environment(dir.path(), concat!("#!/usr/bin/python3\nimport json, os, sys\n", "request=json.load(sys.stdin)\nassert request['operation']=='inventory'\nassert 'UNLISTED_PRIVATE_VALUE' not in os.environ\n", "print(json.dumps({'schema_version':1,'complete':True,'resources':[{'kind':'proxmox.node','id':'node-a','properties':{}}],'warnings':[]}))\n"));
    let out = dir.path().join("inventory.json");
    let output = binary()
        .env("UNLISTED_PRIVATE_VALUE", "synthetic-value")
        .args([
            "--config",
            environment.to_str().unwrap(),
            "inventory",
            "pull",
            "--provider",
            "proxmox",
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    let snapshot: Value = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(snapshot["complete"], true);
    assert_eq!(fs::metadata(&out).unwrap().mode() & 0o777, 0o600);
    let repeated = binary()
        .args([
            "--config",
            environment.to_str().unwrap(),
            "inventory",
            "pull",
            "--provider",
            "proxmox",
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stderr).contains("never overwritten"));
}

#[test]
fn provider_failure_withholds_secret_output_and_limits_output() {
    let dir = temporary();
    let environment = provider_environment(
        dir.path(),
        "#!/usr/bin/python3\nimport sys\nprint('synthetic-sensitive-value',file=sys.stderr)\nsys.exit(2)\n",
    );
    let output = binary()
        .args([
            "--config",
            environment.to_str().unwrap(),
            "inventory",
            "pull",
            "--provider",
            "proxmox",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("synthetic-sensitive-value"));
    executable(
        &dir.path().join("provider"),
        "#!/usr/bin/python3\nprint('x'*2000000)\n",
    );
    let output = binary()
        .args([
            "--config",
            environment.to_str().unwrap(),
            "inventory",
            "pull",
            "--provider",
            "proxmox",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("exceeded limit"));
}

fn recipe_environment(
    dir: &Path,
    allow_execution: bool,
    allow_mutations: bool,
    script: &str,
) -> (PathBuf, PathBuf) {
    fs::create_dir(dir.join("recipe")).unwrap();
    executable(&dir.join("ansible-playbook"), script);
    fs::write(
        dir.join("inventory.ini"),
        "[cmdmesh_targets]\nnode-a ansible_host=192.0.2.10\n",
    )
    .unwrap();
    fs::write(
        dir.join("recipe/playbook.yml"),
        "---\n- hosts: cmdmesh_targets\n  tasks: []\n",
    )
    .unwrap();
    fs::write(dir.join("recipe/README.md"), "Review state after failure.\n").unwrap();
    let manifest = dir.join("recipe/recipe.json");
    write_json(
        &manifest,
        json!({"schema_version":1,"id":"baseline","title":"Baseline","summary":"Host checks","status":"ready","requirements":[],"dependencies":[],"steps":[{"id":"baseline","driver":"ansible","playbook":"playbook.yml"}],"verification":["Review results"],"recovery":"README.md"}),
    );
    let environment = dir.join("environment.json");
    write_json(
        &environment,
        json!({"schema_version":1,"targets":{"lab":{"inventory":"inventory.ini","limit":"node-a","lifecycle_owner":"terraform"}},"execution":{"allow_execution":allow_execution,"allow_mutations":allow_mutations,"ansible_executable":"ansible-playbook","timeout_seconds":1}}),
    );
    (environment, manifest)
}
fn cli_plan(environment: &Path, manifest: &Path, operation: &str, plan: &Path) -> Value {
    let output = binary()
        .args([
            "--config",
            environment.to_str().unwrap(),
            "recipe",
            "plan",
            manifest.to_str().unwrap(),
            "--target",
            "lab",
            "--operation",
            operation,
            "--out",
            plan.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn static_plan_does_not_execute_and_approved_check_runs_only_bound_arguments() {
    let dir = temporary();
    let marker = dir.path().join("called.json");
    let script = format!("#!/usr/bin/python3\nimport json,sys,os,stat,pathlib\ncfg=pathlib.Path(os.environ['ANSIBLE_CONFIG'])\nassert cfg.name=='ansible.cfg'\nassert cfg.read_text().startswith('[defaults]\\n')\nassert stat.S_IMODE(cfg.stat().st_mode)==0o600\nassert str(cfg).startswith(os.environ['TMPDIR'])\nassert 'UNLISTED_PRIVATE_VALUE' not in os.environ\nassert stat.S_IMODE(os.stat(os.environ['TMPDIR']).st_mode)==0o700\nassert os.environ['ANSIBLE_LOCAL_TEMP'].startswith(os.environ['TMPDIR'])\njson.dump(sys.argv[1:],open({:?},'w'))\n", marker.to_str().unwrap());
    let (environment, manifest) = recipe_environment(dir.path(), true, false, &script);
    let plan = dir.path().join("plan.json");
    let summary = cli_plan(&environment, &manifest, "check", &plan);
    assert!(!marker.exists());
    let denied = binary()
        .args([
            "recipe",
            "execute",
            "--plan",
            plan.to_str().unwrap(),
            "--approve-digest",
            "wrong",
        ])
        .output()
        .unwrap();
    assert!(!denied.status.success());
    assert!(!marker.exists());
    let output = binary()
        .env("UNLISTED_PRIVATE_VALUE", "synthetic-value")
        .args([
            "recipe",
            "execute",
            "--plan",
            plan.to_str().unwrap(),
            "--approve-digest",
            summary["digest"].as_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let args: Vec<String> = serde_json::from_slice(&fs::read(marker).unwrap()).unwrap();
    assert!(args.contains(&"--check".to_string()));
    assert!(args.contains(&"node-a".to_string()));
    assert!(args.contains(&"{\"cmdmesh_allow_changes\":false,\"lifecycle_owner\":\"terraform\"}".to_string()));
}

#[test]
fn actual_ansible_check_ignores_ambient_configuration() {
    let Ok(ansible) = std::env::var("CMDMESH_TEST_ANSIBLE") else {
        eprintln!("Set CMDMESH_TEST_ANSIBLE for the actual Ansible regression check");
        return;
    };
    let dir = temporary();
    let (environment, manifest) = recipe_environment(dir.path(), true, false, "#!/bin/sh\nexit 99\n");
    let mut config: Value = serde_json::from_slice(&fs::read(&environment).unwrap()).unwrap();
    config["execution"]["ansible_executable"] = json!(ansible);
    config["execution"]["timeout_seconds"] = json!(30);
    write_json(&environment, config);
    fs::write(
        dir.path().join("inventory.ini"),
        "[cmdmesh_targets]\nnode-a ansible_connection=local\n",
    )
    .unwrap();
    fs::write(dir.path().join("recipe/playbook.yml"), "---\n- hosts: cmdmesh_targets\n  gather_facts: false\n  tasks:\n    - ansible.builtin.assert:\n        that: ansible_check_mode\n    - ansible.builtin.debug:\n        msg: synthetic-private-preview\n").unwrap();
    fs::write(
        dir.path().join("recipe/ansible.cfg"),
        "[defaults]\nstdout_callback=nonexistent_ambient_callback\n",
    )
    .unwrap();
    let plan = dir.path().join("plan.json");
    let summary = cli_plan(&environment, &manifest, "check", &plan);
    let logs = dir.path().join("logs");
    let without_logs = binary()
        .env("ANSIBLE_CONFIG", "/nonexistent/unsafe.cfg")
        .args([
            "recipe",
            "execute",
            "--plan",
            plan.to_str().unwrap(),
            "--approve-digest",
            summary["digest"].as_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        without_logs.status.success(),
        "{}",
        String::from_utf8_lossy(&without_logs.stderr)
    );
    assert!(!logs.exists());
    let output = binary()
        .env("ANSIBLE_CONFIG", "/nonexistent/unsafe.cfg")
        .args([
            "recipe",
            "execute",
            "--plan",
            plan.to_str().unwrap(),
            "--approve-digest",
            summary["digest"].as_str().unwrap(),
            "--log-dir",
            logs.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("synthetic-private-preview"));
    let preview = fs::read_to_string(logs.join("step-0001.stdout")).unwrap();
    assert!(preview.contains("synthetic-private-preview"));
    assert!(preview.contains("PLAY RECAP"));
    assert!(preview.contains("changed=0"));
}

#[test]
fn actual_ansible_ssh_does_not_require_a_multiplexing_socket() {
    let Ok(ansible) = std::env::var("CMDMESH_TEST_ANSIBLE") else {
        return;
    };
    let dir = temporary();
    let marker = dir.path().join("ssh-arguments.json");
    let fake_ssh = dir.path().join("fake-ssh");
    executable(
        &fake_ssh,
        &format!(
            "#!/usr/bin/python3\nimport json,sys\njson.dump(sys.argv[1:],open({:?},'w'))\nsys.exit(255)\n",
            marker.to_str().unwrap()
        ),
    );
    let (environment, manifest) = recipe_environment(dir.path(), true, false, "#!/bin/sh\nexit 99\n");
    let mut config: Value = serde_json::from_slice(&fs::read(&environment).unwrap()).unwrap();
    config["execution"]["ansible_executable"] = json!(ansible);
    config["execution"]["timeout_seconds"] = json!(30);
    config["targets"]["lab"]["inventory"] = json!("ssh-inventory.json");
    write_json(&environment, config);
    write_json(
        &dir.path().join("ssh-inventory.json"),
        json!({"all":{"children":{"cmdmesh_targets":{"hosts":{"node-a":{
            "ansible_connection":"ssh","ansible_host":"192.0.2.10","ansible_user":"operator",
            "ansible_ssh_executable":fake_ssh
        }}}}}}),
    );
    fs::write(
        dir.path().join("recipe/playbook.yml"),
        "---\n- hosts: cmdmesh_targets\n  gather_facts: false\n  tasks:\n    - ansible.builtin.ping:\n",
    )
    .unwrap();
    let plan = dir.path().join("plan.json");
    let summary = cli_plan(&environment, &manifest, "check", &plan);
    let output = binary()
        .args([
            "recipe",
            "execute",
            "--plan",
            plan.to_str().unwrap(),
            "--approve-digest",
            summary["digest"].as_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "the synthetic SSH executable deliberately refuses the connection"
    );
    let args: Vec<String> = serde_json::from_slice(&fs::read(marker).unwrap()).unwrap();
    assert!(args.contains(&"ControlMaster=no".into()), "{args:?}");
    assert!(args.contains(&"ControlPersist=no".into()), "{args:?}");
    assert!(args.contains(&"ControlPath=none".into()), "{args:?}");
}

#[test]
fn recipe_logs_are_opt_in_private_and_preserve_failures_without_terminal_leaks() {
    for code in [0, 7] {
        let dir = temporary();
        let script = format!("#!/usr/bin/python3\nimport sys\nprint('synthetic-secret-stdout')\nprint('synthetic-secret-stderr',file=sys.stderr)\nsys.exit({code})\n");
        let (environment, manifest) = recipe_environment(dir.path(), true, false, &script);
        let plan = dir.path().join("plan.json");
        let summary = cli_plan(&environment, &manifest, "check", &plan);
        for capture in [false, true] {
            let logs = dir.path().join("logs");
            let mut command = binary();
            command.args([
                "recipe",
                "execute",
                "--plan",
                plan.to_str().unwrap(),
                "--approve-digest",
                summary["digest"].as_str().unwrap(),
            ]);
            if capture {
                command.args(["--log-dir", logs.to_str().unwrap()]);
            }
            let output = command.output().unwrap();
            assert_eq!(output.status.success(), code == 0);
            assert!(!String::from_utf8_lossy(&output.stdout).contains("synthetic-secret"));
            assert!(!String::from_utf8_lossy(&output.stderr).contains("synthetic-secret"));
            if !capture {
                assert!(!logs.exists());
                continue;
            }
            assert_eq!(fs::metadata(&logs).unwrap().mode() & 0o777, 0o700);
            for (name, expected) in [
                ("step-0001.stdout", "synthetic-secret-stdout"),
                ("step-0001.stderr", "synthetic-secret-stderr"),
            ] {
                assert_eq!(fs::metadata(logs.join(name)).unwrap().mode() & 0o777, 0o600);
                assert!(fs::read_to_string(logs.join(name)).unwrap().contains(expected));
            }
            let report: Value =
                serde_json::from_slice(&fs::read(logs.join("execution.json")).unwrap()).unwrap();
            assert_eq!(report["status"], if code == 0 { "succeeded" } else { "failed" });
            assert_eq!(report["completed_steps"], if code == 0 { 1 } else { 0 });
        }
    }
}

#[test]
fn private_logs_remain_bounded_on_timeout_and_output_overflow() {
    for (script, error) in [
        (
            "#!/usr/bin/python3\nimport time\nprint('synthetic-private-prefix',flush=True)\ntime.sleep(30)\n",
            "timed out",
        ),
        (
            "#!/usr/bin/python3\nprint('synthetic-private-prefix',flush=True)\nprint('x'*2000000)\n",
            "exceeded limit",
        ),
    ] {
        let dir = temporary();
        let (environment, manifest) = recipe_environment(dir.path(), true, false, script);
        let plan = dir.path().join("plan.json");
        let summary = cli_plan(&environment, &manifest, "check", &plan);
        let logs = dir.path().join("logs");
        let output = binary()
            .args([
                "recipe",
                "execute",
                "--plan",
                plan.to_str().unwrap(),
                "--approve-digest",
                summary["digest"].as_str().unwrap(),
                "--log-dir",
                logs.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains(error));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("synthetic-private-prefix"));
        let raw = fs::read(logs.join("step-0001.stdout")).unwrap();
        assert!(raw.starts_with(b"synthetic-private-prefix"));
        assert!(raw.len() <= 1024 * 1024);
        let report: Value = serde_json::from_slice(&fs::read(logs.join("execution.json")).unwrap()).unwrap();
        assert_eq!(report["status"], "failed");
    }
}

#[test]
fn log_destinations_cannot_overwrite_or_modify_bound_inputs() {
    let dir = temporary();
    let marker = dir.path().join("called");
    let script = format!(
        "#!/usr/bin/python3\nfrom pathlib import Path\nPath({:?}).touch()\n",
        marker.to_str().unwrap()
    );
    let (environment, manifest) = recipe_environment(dir.path(), true, false, &script);
    fs::create_dir(dir.path().join("existing")).unwrap();
    fs::write(dir.path().join("existing/preserve"), "keep").unwrap();
    std::os::unix::fs::symlink(dir.path().join("existing"), dir.path().join("symlink")).unwrap();
    let plan = dir.path().join("plan.json");
    let summary = cli_plan(&environment, &manifest, "check", &plan);
    for name in ["existing", "symlink", "recipe/logs", "missing-parent/logs"] {
        let logs = dir.path().join(name);
        let output = binary()
            .args([
                "recipe",
                "execute",
                "--plan",
                plan.to_str().unwrap(),
                "--approve-digest",
                summary["digest"].as_str().unwrap(),
                "--log-dir",
                logs.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(!output.status.success(), "destination {name} was accepted");
        assert!(!marker.exists());
    }
    assert_eq!(
        fs::read_to_string(dir.path().join("existing/preserve")).unwrap(),
        "keep"
    );
    for approval in ["wrong", summary["digest"].as_str().unwrap()] {
        if approval != "wrong" {
            fs::write(dir.path().join("inventory.ini"), "changed").unwrap();
        }
        let logs = dir.path().join("never-created");
        let output = binary()
            .args([
                "recipe",
                "execute",
                "--plan",
                plan.to_str().unwrap(),
                "--approve-digest",
                approval,
                "--log-dir",
                logs.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!logs.exists());
        assert!(!marker.exists());
    }
}

#[test]
fn changed_or_legacy_runner_configuration_cannot_execute() {
    let dir = temporary();
    let marker = dir.path().join("called");
    let script = format!(
        "#!/usr/bin/python3\nfrom pathlib import Path\nPath({:?}).touch()\n",
        marker.to_str().unwrap()
    );
    let (environment, manifest) = recipe_environment(dir.path(), true, false, &script);
    let plan = dir.path().join("plan.json");
    let summary = cli_plan(&environment, &manifest, "check", &plan);
    let original: Value = serde_json::from_slice(&fs::read(&plan).unwrap()).unwrap();
    for legacy in [false, true] {
        let mut altered = original.clone();
        if legacy {
            altered.as_object_mut().unwrap().remove("ansible_config");
        } else {
            altered["ansible_config"] = json!("[defaults]\nstdout_callback=unsafe\n");
        }
        write_json(&plan, altered);
        let logs = dir.path().join("never-created");
        let output = binary()
            .args([
                "recipe",
                "execute",
                "--plan",
                plan.to_str().unwrap(),
                "--approve-digest",
                summary["digest"].as_str().unwrap(),
                "--log-dir",
                logs.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!logs.exists());
        assert!(!marker.exists());
    }
}

#[test]
fn failed_second_step_records_only_completed_work_and_stops_the_recipe() {
    let dir = temporary();
    let script = "#!/usr/bin/python3\nimport pathlib,sys\nname=pathlib.Path(sys.argv[-1]).name\nprint(name)\nsys.exit(8 if name=='second.yml' else 0)\n";
    let (environment, manifest) = recipe_environment(dir.path(), true, false, script);
    let mut recipe: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    recipe["steps"] = json!([
        {"id":"one","driver":"ansible","playbook":"playbook.yml"},
        {"id":"two","driver":"ansible","playbook":"second.yml"},
        {"id":"three","driver":"ansible","playbook":"third.yml"}
    ]);
    write_json(&manifest, recipe);
    for name in ["second.yml", "third.yml"] {
        fs::write(dir.path().join("recipe").join(name), "---\n").unwrap();
    }
    let plan = dir.path().join("plan.json");
    let summary = cli_plan(&environment, &manifest, "check", &plan);
    let logs = dir.path().join("logs");
    let output = binary()
        .args([
            "recipe",
            "execute",
            "--plan",
            plan.to_str().unwrap(),
            "--approve-digest",
            summary["digest"].as_str().unwrap(),
            "--log-dir",
            logs.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&fs::read(logs.join("execution.json")).unwrap()).unwrap();
    assert_eq!(report["status"], "failed");
    assert_eq!(report["completed_steps"], 1);
    assert!(fs::read_to_string(logs.join("step-0002.stdout"))
        .unwrap()
        .contains("second.yml"));
    assert!(!logs.join("step-0003.stdout").exists());
}

#[test]
fn apply_requires_mutation_permission_and_changed_inputs_invalidate_check() {
    let dir = temporary();
    let (environment, manifest) = recipe_environment(
        dir.path(),
        true,
        false,
        "#!/usr/bin/python3\nraise RuntimeError('must not execute')\n",
    );
    let plan = dir.path().join("apply.json");
    let summary = cli_plan(&environment, &manifest, "apply", &plan);
    let output = binary()
        .args([
            "recipe",
            "execute",
            "--plan",
            plan.to_str().unwrap(),
            "--approve-digest",
            summary["digest"].as_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stderr).contains("mutations disabled"));
    let check_plan = dir.path().join("check.json");
    let summary = cli_plan(&environment, &manifest, "check", &check_plan);
    fs::write(dir.path().join("inventory.ini"), "changed\n").unwrap();
    let output = binary()
        .args([
            "recipe",
            "execute",
            "--plan",
            check_plan.to_str().unwrap(),
            "--approve-digest",
            summary["digest"].as_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stderr).contains("stale"));
}

#[test]
fn timeout_kills_descendants_before_returning() {
    let dir = temporary();
    let marker = dir.path().join("escaped");
    let grandchild = format!(
        "import time,pathlib;time.sleep(1.7);pathlib.Path({:?}).touch()",
        marker.to_str().unwrap()
    );
    let script = format!("#!/usr/bin/python3\nimport subprocess,time\nsubprocess.Popen(['/usr/bin/python3','-c',{grandchild:?}])\ntime.sleep(30)\n");
    let (environment, manifest) = recipe_environment(dir.path(), true, true, &script);
    let plan = dir.path().join("plan.json");
    let summary = cli_plan(&environment, &manifest, "check", &plan);
    let output = binary()
        .args([
            "recipe",
            "execute",
            "--plan",
            plan.to_str().unwrap(),
            "--approve-digest",
            summary["digest"].as_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("timed out"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    thread::sleep(Duration::from_millis(1000));
    assert!(
        !marker.exists(),
        "a descendant survived process-group termination"
    );
}

#[test]
fn real_stdio_mcp_handshake_lists_no_execution_and_calls_shared_drift() {
    let dir = temporary();
    let desired = dir.path().join("desired.json");
    let inventory = dir.path().join("inventory.json");
    write_json(&desired, json!({"schema_version":1,"resources":[]}));
    write_json(
        &inventory,
        json!({"schema_version":1,"complete":false,"resources":[],"warnings":["unreadable scope"]}),
    );
    let mut child = binary()
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
    let mut request = |value: Value| {
        writeln!(input, "{value}").unwrap();
        input.flush().unwrap();
    };
    request(
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"synthetic-test","version":"1"}}}),
    );
    let first = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    let first: Value = serde_json::from_str(&first).unwrap();
    assert_eq!(first["result"]["serverInfo"]["name"], "cmdmesh");
    request(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    request(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}));
    let tools: Value = serde_json::from_str(&receiver.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
    let names: Vec<_> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 7);
    assert!(names.contains(&"cmdmesh_inspection"));
    assert!(names.contains(&"cmdmesh_operation"));
    assert!(names.iter().all(|name| name.starts_with("cmdmesh_")));
    assert!(!names.iter().any(|n| n.contains("execute")));
    for tool in tools["result"]["tools"].as_array().unwrap() {
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert_eq!(tool["annotations"]["destructiveHint"], false);
        assert_eq!(tool["annotations"]["idempotentHint"], true);
    }
    request(
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"cmdmesh_drift_detect","arguments":{"desired":desired,"inventory":inventory}}}),
    );
    let response: Value =
        serde_json::from_str(&receiver.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
    let report: Value =
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(report["clean"], false);
    assert_eq!(report["complete"], false);
    request(
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"cmdmesh_recipe_inspect","arguments":{"manifest":"unused","execute":true}}}),
    );
    let rejected: Value =
        serde_json::from_str(&receiver.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
    assert!(rejected["error"].is_object() || rejected["result"]["isError"] == true);
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn fifo_input_is_rejected_without_waiting_for_a_writer() {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let dir = temporary();
    let fifo = dir.path().join("input-fifo");
    let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let desired = dir.path().join("desired.json");
    write_json(&desired, json!({"schema_version":1,"resources":[]}));
    let mut child = binary()
        .args([
            "drift",
            "detect",
            "--desired",
            desired.to_str().unwrap(),
            "--inventory",
            fifo.to_str().unwrap(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(100));
    let exited = child.try_wait().unwrap().is_some();
    if !exited {
        child.kill().unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(exited, "input file opening blocked on FIFO");
    assert!(String::from_utf8_lossy(&output.stderr).contains("regular file"));
}

#[test]
fn detached_descendant_holding_pipes_cannot_block_direct_child_completion() {
    let dir = temporary();
    let pid_file = dir.path().join("detached.pid");
    let script = format!("#!/usr/bin/python3\nimport subprocess,json,pathlib\np=subprocess.Popen(['/usr/bin/python3','-c','import time;time.sleep(30)'],start_new_session=True)\npathlib.Path({:?}).write_text(str(p.pid))\nprint(json.dumps({{'schema_version':1,'complete':True,'resources':[],'warnings':[]}}))\n", pid_file.to_str().unwrap());
    let environment = provider_environment(dir.path(), &script);
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        sender
            .send(
                binary()
                    .args([
                        "--config",
                        environment.to_str().unwrap(),
                        "inventory",
                        "pull",
                        "--provider",
                        "proxmox",
                    ])
                    .output()
                    .unwrap(),
            )
            .unwrap();
    });
    let response = receiver.recv_timeout(Duration::from_millis(500));
    // The child deliberately leaves the owned process group. The test owns and
    // cleans that synthetic process; CmdMesh is not a process sandbox.
    if let Ok(pid) = fs::read_to_string(pid_file) {
        unsafe {
            libc::kill(pid.parse().unwrap(), libc::SIGKILL);
        }
    }
    let completed_without_pipe_close = response.is_ok();
    let output = response.unwrap_or_else(|_| receiver.recv_timeout(Duration::from_secs(3)).unwrap());
    assert!(
        completed_without_pipe_close,
        "reader cleanup waited for an escaped descendant to close its pipe"
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn explicitly_authorized_apply_sets_role_opt_in_without_check_flag() {
    let dir = temporary();
    let marker = dir.path().join("arguments.json");
    let script = format!(
        "#!/usr/bin/python3\nimport json,sys\njson.dump(sys.argv[1:],open({:?},'w'))\n",
        marker.to_str().unwrap()
    );
    let (environment, manifest) = recipe_environment(dir.path(), true, true, &script);
    let plan = dir.path().join("apply.json");
    let summary = cli_plan(&environment, &manifest, "apply", &plan);
    let output = binary()
        .args([
            "recipe",
            "execute",
            "--plan",
            plan.to_str().unwrap(),
            "--approve-digest",
            summary["digest"].as_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let args: Vec<String> = serde_json::from_slice(&fs::read(marker).unwrap()).unwrap();
    assert!(!args.contains(&"--check".into()));
    assert!(args.contains(&"{\"cmdmesh_allow_changes\":true,\"lifecycle_owner\":\"terraform\"}".into()));
}

#[test]
fn cancellation_stops_the_owned_process_group() {
    let dir = temporary();
    let marker = dir.path().join("survived");
    let started = dir.path().join("started");
    let grandchild = format!(
        "import time,pathlib;time.sleep(1.2);pathlib.Path({:?}).touch()",
        marker.to_str().unwrap()
    );
    let script = format!("#!/usr/bin/python3\nimport subprocess,time,pathlib\nsubprocess.Popen(['/usr/bin/python3','-c',{grandchild:?}])\npathlib.Path({:?}).touch()\ntime.sleep(30)\n", started.to_str().unwrap());
    let (environment, manifest) = recipe_environment(dir.path(), true, false, &script);
    let plan = dir.path().join("check.json");
    let summary = cli_plan(&environment, &manifest, "check", &plan);
    let logs = dir.path().join("cancelled-logs");
    let child = binary()
        .args([
            "recipe",
            "execute",
            "--plan",
            plan.to_str().unwrap(),
            "--approve-digest",
            summary["digest"].as_str().unwrap(),
            "--log-dir",
            logs.to_str().unwrap(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    for _ in 0..40 {
        if started.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(started.exists(), "synthetic child did not start");
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cancelled"));
    assert!(logs.join("step-0001.stdout").is_file());
    let report: Value = serde_json::from_slice(&fs::read(logs.join("execution.json")).unwrap()).unwrap();
    assert_eq!(report["status"], "failed");
    thread::sleep(Duration::from_millis(1400));
    assert!(!marker.exists());
}
