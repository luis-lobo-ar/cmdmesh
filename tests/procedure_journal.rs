use cmdmesh::procedure::Journal;
use std::{
    fs,
    io::{BufRead, BufReader},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
    process::{Command, Stdio},
};

fn fixture() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(".generated/procedure-journal-tests");
    fs::create_dir_all(&root).unwrap();
    let temp = tempfile::tempdir_in(root).unwrap();
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    temp
}

#[test]
fn physical_subject_lock_is_stable_across_aliases_and_reopen() {
    let dir = fixture();
    let subject = "a".repeat(64);
    let first = Journal::open(dir.path()).unwrap();
    let second = Journal::open(dir.path()).unwrap();
    let held = first.lock(&subject).unwrap();
    let lock = dir.path().join("locks").join(format!("{subject}.lock"));
    let inode = fs::metadata(&lock).unwrap().ino();
    assert!(second.lock(&subject).is_err());
    assert!(second.lock(&"b".repeat(64)).is_ok());
    drop(held);
    let again = second.lock(&subject).unwrap();
    assert_eq!(fs::metadata(&lock).unwrap().ino(), inode);
    drop(again);
}

#[test]
fn durable_intent_survives_restart_and_is_never_overwritten() {
    let dir = fixture();
    let plan = "b".repeat(64);
    let subject = "a".repeat(64);
    let journal = Journal::open(dir.path()).unwrap();
    assert!(!journal.consumed(&plan).unwrap());
    let guard = journal.lock(&subject).unwrap();
    let intent = guard.consume(&plan).unwrap();
    assert_eq!(intent.plan_digest, plan);
    assert_eq!(intent.subject_key, subject);
    let path = dir.path().join("intents").join(format!("{plan}.json"));
    let original = fs::read(&path).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o077, 0);
    assert!(guard.consume(&plan).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    drop(guard);
    drop(journal);
    let reopened = Journal::open(dir.path()).unwrap();
    assert!(reopened.consumed(&plan).unwrap());
    assert!(reopened.lock(&subject).unwrap().consume(&plan).is_err());
}

#[test]
fn partial_or_conflicting_intent_is_consumed_not_a_retry_instruction() {
    for mode in ["partial", "symlink", "directory"] {
        let dir = fixture();
        let plan = "b".repeat(64);
        let journal = Journal::open(dir.path()).unwrap();
        let path = dir.path().join("intents").join(format!("{plan}.json"));
        match mode {
            "partial" => fs::write(&path, b"{\"incomplete\":").unwrap(),
            "symlink" => std::os::unix::fs::symlink(dir.path().join("absent"), &path).unwrap(),
            _ => fs::create_dir(&path).unwrap(),
        };
        assert!(journal.consumed(&plan).unwrap(), "{mode}");
        assert!(
            journal.lock(&"a".repeat(64)).unwrap().consume(&plan).is_err(),
            "{mode}"
        );
    }
}

#[test]
fn invalid_identity_or_nonprivate_state_and_symlink_locks_are_refused() {
    let dir = fixture();
    let journal = Journal::open(dir.path()).unwrap();
    for key in ["../escape", "", "A", "abc/def"] {
        assert!(journal.lock(key).is_err());
        assert!(journal.consumed(key).is_err());
    }
    let subject = "a".repeat(64);
    let destination = dir.path().join("unrelated");
    fs::write(&destination, b"unchanged").unwrap();
    std::os::unix::fs::symlink(
        &destination,
        dir.path().join("locks").join(format!("{subject}.lock")),
    )
    .unwrap();
    assert!(journal.lock(&subject).is_err());
    assert_eq!(fs::read(&destination).unwrap(), b"unchanged");
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Journal::open(dir.path()).is_err());
}

#[test]
fn external_lock_owner_blocks_and_termination_releases_without_unlinking() {
    let dir = fixture();
    let journal = Journal::open(dir.path()).unwrap();
    let subject = "a".repeat(64);
    drop(journal.lock(&subject).unwrap());
    let path = dir.path().join("locks").join(format!("{subject}.lock"));
    let inode = fs::metadata(&path).unwrap().ino();
    let mut child=Command::new("/usr/bin/python3").args(["-I","-B","-c","import fcntl,sys; f=open(sys.argv[1],'r+'); fcntl.flock(f,fcntl.LOCK_EX); print('ready',flush=True); sys.stdin.read()",path.to_str().unwrap()]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line, "ready\n");
    assert!(journal.lock(&subject).is_err());
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(journal.lock(&subject).is_ok());
    assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
}
