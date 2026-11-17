use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    os::fd::AsRawFd,
    os::unix::{
        fs::{DirBuilderExt, PermissionsExt},
        process::CommandExt,
    },
    path::Path,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

const MAX_OUTPUT: usize = 1024 * 1024;
static CANCELLED: AtomicBool = AtomicBool::new(false);
extern "C" fn cancel(_: libc::c_int) {
    CANCELLED.store(true, Ordering::Relaxed);
}

pub fn install_cancellation_handlers() {
    // The handler performs only an atomic store; cleanup runs in ordinary code.
    unsafe {
        libc::signal(libc::SIGINT, cancel as *const () as usize);
        libc::signal(libc::SIGTERM, cancel as *const () as usize);
    }
}
pub fn cancelled() -> bool {
    CANCELLED.load(Ordering::Relaxed)
}

struct ChildGroup {
    child: Child,
    terminated: bool,
}
impl ChildGroup {
    fn terminate(&mut self) {
        if self.terminated {
            return;
        }
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.wait();
        self.terminated = true;
    }
}
impl Drop for ChildGroup {
    fn drop(&mut self) {
        self.terminate();
    }
}

pub(crate) struct RunSpec<'a> {
    pub program: &'a Path,
    pub args: &'a [OsString],
    pub input: Option<Vec<u8>>,
    pub allowlist: &'a [String],
    pub extra_env: &'a BTreeMap<String, String>,
    pub cwd: &'a Path,
    pub timeout: Duration,
    pub private_root: &'a Path,
}

pub(crate) fn run(spec: RunSpec<'_>) -> Result<Vec<u8>> {
    if cancelled() {
        bail!("operation cancelled before execution");
    }
    let temporary_root = spec.private_root.join(".generated/cmdmesh-runs");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&temporary_root)
        .context("cannot create private process scratch root")?;
    if fs::symlink_metadata(&temporary_root)?.file_type().is_symlink()
        || fs::metadata(&temporary_root)?.permissions().mode() & 0o077 != 0
    {
        bail!("process scratch root must be private and not a symlink");
    }
    let scratch = tempfile::Builder::new()
        .prefix("run-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(temporary_root)?;
    let local_temp = scratch.path().join("ansible-local");
    let control_dir = scratch.path().join("ssh-control");
    fs::DirBuilder::new().mode(0o700).create(&local_temp)?;
    fs::DirBuilder::new().mode(0o700).create(&control_dir)?;
    let mut command = Command::new(spec.program);
    command
        .args(spec.args)
        .current_dir(spec.cwd)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .env("TERM", "dumb");
    for key in spec.allowlist {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .envs(spec.extra_env)
        .env("TMPDIR", scratch.path())
        .env("TMP", scratch.path())
        .env("TEMP", scratch.path())
        .env("ANSIBLE_LOCAL_TEMP", &local_temp)
        .env("ANSIBLE_SSH_CONTROL_PATH_DIR", &control_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.process_group(0);
    let child = command.spawn().context("cannot launch configured executable")?;
    let mut group = ChildGroup {
        child,
        terminated: false,
    };
    let mut stdin = group.child.stdin.take();
    let mut stdout = group.child.stdout.take().unwrap();
    let mut stderr = group.child.stderr.take().unwrap();
    nonblocking(stdin.as_ref().unwrap())?;
    nonblocking(&stdout)?;
    nonblocking(&stderr)?;
    let payload = spec.input.unwrap_or_default();
    let mut input_offset = 0;
    let mut output = vec![];
    let mut diagnostics = vec![];
    let started = Instant::now();
    let status = loop {
        if cancelled() {
            group.terminate();
            bail!("operation cancelled; remote outcome may be unknown");
        }
        if started.elapsed() >= spec.timeout {
            group.terminate();
            bail!("operation timed out; remote outcome may be unknown");
        }
        drain(&mut stdout, &mut output)?;
        drain(&mut stderr, &mut diagnostics)?;
        if let Some(stream) = &mut stdin {
            if input_offset < payload.len() {
                match stream.write(&payload[input_offset..]) {
                    Ok(count) => input_offset += count,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) if error.kind() == io::ErrorKind::BrokenPipe => input_offset = payload.len(),
                    Err(_) => bail!("cannot write child protocol request"),
                }
            }
            if input_offset == payload.len() {
                stdin.take();
            }
        }
        if let Some(status) = group.child.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_millis(10));
    };
    // A process that leaves the owned group may retain inherited pipes. All
    // reads stay nonblocking; completion never waits for that process or EOF.
    group.terminate();
    drain(&mut stdout, &mut output)?;
    drain(&mut stderr, &mut diagnostics)?;
    if !status.success() {
        bail!(
            "child operation failed (exit {:?}); raw output withheld",
            status.code()
        );
    }
    Ok(output)
}

fn nonblocking(stream: &impl AsRawFd) -> Result<()> {
    let descriptor = stream.as_raw_fd();
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        bail!("cannot configure bounded child I/O");
    }
    Ok(())
}

fn drain(stream: &mut impl Read, output: &mut Vec<u8>) -> Result<()> {
    let mut buffer = [0u8; 8192];
    // A busy writer cannot prevent the caller from checking its deadline.
    for _ in 0..=(MAX_OUTPUT / buffer.len()) {
        let count = match stream.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => bail!("cannot read child output; raw output withheld"),
        };
        if output.len().saturating_add(count) > MAX_OUTPUT {
            bail!("child output exceeded limit; raw output withheld");
        }
        output.extend_from_slice(&buffer[..count]);
    }
    Ok(())
}
