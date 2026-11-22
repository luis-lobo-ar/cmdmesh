use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};
use tempfile::TempDir;

fn temporary() -> TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join(".generated/inspection-tests");
    fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(base)
        .unwrap()
}
fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cmdmesh"))
}
fn write(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}
fn fixture(root: &Path, code: &str) -> PathBuf {
    let executable = root.join("inspector");
    fs::write(
        &executable,
        format!("#!/usr/bin/python3\nimport json,sys,os\nr=json.load(sys.stdin)\n{code}\n"),
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let digest = hex::encode(Sha256::digest(fs::read(&executable).unwrap()));
    let config = root.join("environment.json");
    write(
        &config,
        &json!({"schema_version":1,"providers":{"native":{
        "executable":"inspector","expected_sha256":digest,
        "config":{"private":"SYNTHETIC_PRIVATE_CONFIG","original":executable},
        "env_allowlist":["INSPECTION_TEST_TOKEN"]}}}),
    );
    write(&root.join("input.json"), &json!({"selector":"example"}));
    config
}
fn response(outcome: &str, operation: &str) -> Value {
    json!({"document_type":"cmdmesh.inspection.result","schema_version":1,"operation":operation,
           "outcome":outcome,"data":{"checked":true},"diagnostics":[]})
}
fn printer(value: &Value, exit: u8) -> String {
    format!("print({:?})\nsys.exit({exit})", value.to_string())
}
fn invoke(config: &Path) -> Command {
    let mut command = binary();
    command.args([
        "--config",
        config.to_str().unwrap(),
        "inspection",
        "run",
        "--inspector",
        "native",
        "--operation",
        "probe",
        "--input",
        config.parent().unwrap().join("input.json").to_str().unwrap(),
    ]);
    command
}

#[test]
fn inspection_uses_separate_protocol_and_filtered_configuration_with_private_receipt() {
    let dir = temporary();
    let code = format!("assert set(r)=={{'document_type','schema_version','operation','input','config'}}\nassert r['document_type']=='cmdmesh.inspection.request'\nassert r['schema_version']==1\nassert r['operation']=='probe'\nassert r['input']=={{'selector':'example'}}\nassert r['config']['private']=='SYNTHETIC_PRIVATE_CONFIG'\nassert os.environ.get('INSPECTION_TEST_TOKEN')=='SYNTHETIC_CREDENTIAL'\nassert os.environ.get('CMDMESH_INSPECTION_PROCESS_GROUP')=='1'\nassert os.getpgrp()==os.getpid()\nassert 'UNLISTED_SECRET' not in os.environ\n{}", printer(&response("observed", "probe"), 0));
    let config = fixture(dir.path(), &code);
    let out = dir.path().join("receipt.json");
    let output = invoke(&config)
        .env("INSPECTION_TEST_TOKEN", "SYNTHETIC_CREDENTIAL")
        .env("UNLISTED_SECRET", "SYNTHETIC_NOT_ALLOWED")
        .args(["--out", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    let receipt: Value = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(receipt["document_type"], "cmdmesh.inspection.receipt");
    assert_eq!(receipt["result"]["outcome"], "observed");
    assert_eq!(receipt["inspector"], "native");
    assert_eq!(receipt["executable_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(receipt["configuration_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(receipt["request_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(fs::metadata(&out).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(!receipt.to_string().contains("SYNTHETIC_"));
}

#[test]
fn native_negative_outcomes_remain_structured_and_cannot_exit_successfully() {
    for outcome in ["rejected", "unsupported", "incomplete", "unavailable"] {
        let dir = temporary();
        let mut value = response(outcome, "probe");
        value["diagnostics"] = json!([{"code":"source-incomplete","summary":"Source incomplete","next_checks":["Check source"],"recovery":["Recollect"]}]);
        let config = fixture(dir.path(), &printer(&value, 1));
        let output = invoke(&config).output().unwrap();
        assert!(!output.status.success());
        let receipt: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(receipt["result"]["outcome"], outcome);
        assert_eq!(receipt["result"]["diagnostics"][0]["code"], "source-incomplete");
    }
}

#[test]
fn missing_or_mismatched_pin_stops_before_executing_the_inspector() {
    for pin in [Value::Null, json!("0".repeat(64)), json!("invalid")] {
        let dir = temporary();
        let marker = dir.path().join("executed");
        let config = fixture(dir.path(), &format!("open({:?},'w').close()", marker));
        let mut value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        value["providers"]["native"]["expected_sha256"] = pin;
        write(&config, &value);
        let output = invoke(&config).output().unwrap();
        assert!(!output.status.success());
        assert!(!marker.exists());
    }
}

#[test]
fn pinned_code_is_copied_into_private_scratch_before_execution() {
    let dir = temporary();
    let code = format!("from pathlib import Path\np=Path(__file__)\nassert p!=Path(r['config']['original'])\nb=p.read_bytes()\nPath(r['config']['original']).write_text('changed after capture')\nassert p.read_bytes()==b\nassert p.parent.stat().st_mode & 0o77 == 0\n{}", printer(&response("observed", "probe"), 0));
    let config = fixture(dir.path(), &code);
    let output = invoke(&config).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("inspector")).unwrap(),
        "changed after capture"
    );
}

#[test]
fn malformed_protocol_or_exit_mismatch_never_exposes_raw_output() {
    let base = response("observed", "probe");
    let mut examples = vec![];
    for (field, replacement) in [
        ("schema_version", json!(2)),
        ("operation", json!("other")),
        ("document_type", json!("inventory")),
        ("outcome", json!("healthy")),
        ("data", json!([])),
        ("extra", json!("SYNTHETIC_PRIVATE_RAW")),
        (
            "diagnostics",
            json!([{"code":"bad","summary":"SYNTHETIC_PRIVATE_RAW","next_checks":[],"recovery":[],"extra":true}]),
        ),
    ] {
        let mut value = base.clone();
        value[field] = replacement;
        examples.push((value, 0));
    }
    examples.push((response("rejected", "probe"), 0));
    examples.push((response("observed", "probe"), 1));
    examples.push((base, 2));
    for (value, exit) in examples {
        let dir = temporary();
        let config = fixture(dir.path(), &printer(&value, exit));
        let output = invoke(&config).output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("SYNTHETIC_PRIVATE_RAW"));
    }
}

#[test]
fn output_limit_deadline_and_cancellation_are_bounded() {
    for code in ["print('x'*2000000)", "import time\ntime.sleep(30)"] {
        let dir = temporary();
        let config = fixture(dir.path(), code);
        let output = invoke(&config).args(["--timeout-seconds", "1"]).output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    let dir = temporary();
    let marker = dir.path().join("started");
    let config = fixture(
        dir.path(),
        &format!("from pathlib import Path\nimport time\nPath({marker:?}).touch()\ntime.sleep(30)"),
    );
    let child = invoke(&config)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    for _ in 0..200 {
        if marker.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(marker.exists());
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cancelled"));
}

#[test]
fn inspection_describe_and_mcp_run_use_the_same_pinned_transport() {
    let dir = temporary();
    let code = "print(json.dumps({'document_type':'cmdmesh.inspection.result','schema_version':1,'operation':r['operation'],'outcome':'observed','data':{'action':r['operation']},'diagnostics':[]}))";
    let config = fixture(dir.path(), code);
    let output = binary()
        .args([
            "--config",
            config.to_str().unwrap(),
            "inspection",
            "describe",
            "--inspector",
            "native",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let describe: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(describe["result"]["operation"], "describe");
    let cli = invoke(&config).output().unwrap();
    let receipt: Value = serde_json::from_slice(&cli.stdout).unwrap();
    let mut child = binary()
        .args(["--config", config.to_str().unwrap(), "mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let stream = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stream).lines() {
            let _ = sender.send(line.unwrap());
        }
    });
    for message in [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"cmdmesh_inspection","arguments":{"action":"run","inspector":"native","operation":"probe","inputs":dir.path().join("input.json")}}}),
    ] {
        writeln!(input, "{message}").unwrap();
        input.flush().unwrap();
    }
    let mut messages = vec![];
    for _ in 0..3 {
        messages.push(
            serde_json::from_str::<Value>(&receiver.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap(),
        );
    }
    child.kill().unwrap();
    let _ = child.wait();
    let tools = messages.iter().find(|m| m["id"] == 2).unwrap()["result"]["tools"]
        .as_array()
        .unwrap();
    let tool = tools.iter().find(|t| t["name"] == "cmdmesh_inspection").unwrap();
    assert_eq!(tool["annotations"]["readOnlyHint"], true);
    assert_eq!(tool["annotations"]["openWorldHint"], true);
    assert!(!tools
        .iter()
        .any(|t| t["name"].as_str().unwrap().contains("execute")));
    let call = messages.iter().find(|m| m["id"] == 3).unwrap();
    let mcp: Value = serde_json::from_str(call["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(mcp, receipt);
}
