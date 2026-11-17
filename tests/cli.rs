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
    let script = format!("#!/usr/bin/python3\nimport json,sys,os,stat\nassert os.environ['ANSIBLE_CONFIG']=='/dev/null'\nassert 'UNLISTED_PRIVATE_VALUE' not in os.environ\nassert stat.S_IMODE(os.stat(os.environ['TMPDIR']).st_mode)==0o700\nassert os.environ['ANSIBLE_LOCAL_TEMP'].startswith(os.environ['TMPDIR'])\njson.dump(sys.argv[1:],open({:?},'w'))\n", marker.to_str().unwrap());
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
    assert_eq!(names.len(), 5);
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
    let child = binary()
        .args([
            "recipe",
            "execute",
            "--plan",
            plan.to_str().unwrap(),
            "--approve-digest",
            summary["digest"].as_str().unwrap(),
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
    thread::sleep(Duration::from_millis(1400));
    assert!(!marker.exists());
}
