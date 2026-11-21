# Development and installation

CmdMesh is a Rust library with a CLI and thin stdio MCP adapter. Integration
executables, Ansible roles, Terraform modules, and recipe catalogs live in
separate repositories. No private parent repository is needed.

## Build and install

From a reviewed checkout on a Unix controller:

```bash
cargo build --locked --release
./target/release/cmdmesh --version
```

`rust-toolchain.toml` pins Rust 1.92.0, rustfmt, and Clippy. Rustup-managed Cargo
uses that file. The lockfile pins Rust dependencies. Source builds are the path
used by these examples; they do not require a hosted binary release.

To install the reviewed local checkout into Cargo's user binary directory:

```bash
cargo install --locked --path .
```

Add Cargo's binary directory to your shell's PATH if needed. Do not install with
sudo merely to solve a PATH problem. A release build is also usable directly or
can be copied to an operator-managed location. No integration is downloaded at runtime.

Before building for real operations, record and review the exact source commit,
toolchain, package revisions, and external tools. Keep modifications and private
configuration outside public dependency trees. Do not reset a dirty checkout to
force an upgrade. Preserve the last working installation for rollback.

## Repository map

| Path | Responsibility |
| --- | --- |
| `src/config.rs` | Environment parsing, path resolution, policy and environment-name checks |
| `src/inventory.rs` | Provider invocation, snapshot validation, and drift comparison |
| `src/recipe.rs` | Recipe validation, content-bound planning and approved execution |
| `src/process.rs` | Bounded child I/O, private scratch/config, logs and process-group cleanup |
| `src/main.rs` | CLI argument parsing and JSON output |
| `src/mcp.rs` | Read-only MCP adapter over the shared functions |
| `examples/` | Synthetic data and explicit public templates |
| `tests/` | CLI, contract, protocol and documentation-example checks |

## Run verification

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --release
```

Set `CMDMESH_TEST_ANSIBLE` to the absolute path of a trusted Ansible executable
before `cargo test` to run the actual controller regressions as well as doubles:

```bash
CMDMESH_TEST_ANSIBLE="$PWD/.generated/ansible/bin/ansible-playbook" cargo test --locked
```

Without that variable, the two actual-Ansible checks return early; do not count
that as Ansible compatibility evidence. CI installs Core 2.20.4 and sets the
variable. The real SSH adapter test uses a synthetic SSH executable, not a host.
Generated SDK documentation examples are ignored and are not executed tests.

After the [README Ansible setup](../README.md#run-a-local-recipe), run the
documentation checks with a built binary:

```bash
CMDMESH_BIN="$PWD/target/release/cmdmesh" \
  python3 -m unittest discover -s tests -p test_docs.py -v
```

They run clean/changed/incomplete snapshots, local catalog inspection, a
localhost-only check with private logs, and denied apply. They also verify local
Markdown links and anchors. The example's relative Ansible path is exercised
from a fresh private directory under `.generated`, just as in the README.

No documentation test needs real credentials, Proxmox, or cloud services. Keep
such tests synthetic. Passing tests do not certify a host upgrade, application
workflow, hardware behavior, or backup restore.

## Contribute an integration or recipe

An inventory provider is a separately installed executable implementing the
[provider protocol](provider-protocol.md). Add its named declaration to a private
environment; the core does not need a new hardcoded provider for that protocol.
Pin the executable and its supporting source. Test malformed input, partial
scope, auth/TLS failure, bounded output, and secret-safe diagnostics.

A recipe uses a `recipes/<id>/recipe.json` manifest plus reviewed playbooks and
recovery instructions. Current manifest fields are `schema_version`, `id`,
`title`, `summary`, `status`, `requirements`, `dependencies`, `steps`,
`verification`, and `recovery`. A step has `id`, `driver:"ansible"`, and a
relative `playbook`. Paths must stay inside the recipe; symlinked source entries
are refused. A ready recipe needs at least one step. Drafts remain inspectable.

Requirements and verification text describe operator work; they are not an
automatic certification engine. The literal requirement `lifecycle-owner:ansible`
has a core ownership gate. Other operational requirements need recipe guards
and independent evidence. `recovery` names a file, not an automatic rollback step.

Use the [local example recipe](../examples/local/recipes/local-check/recipe.json)
as a minimal manifest and the companion catalog for real procedures. Review
[execution boundaries](execution.md) before adding behavior. There is no generic
shell-step or Terraform execution driver, and no MCP execution endpoint.

Never commit credentials, encrypted secret files, real inventory, private
topology, Terraform state/plans, backups, or operational logs to these public
repositories. Public examples use synthetic data and reserved documentation addresses.

## Build storage

Rust debug and release caches can require several GiB. Check controller space
before compiling. `cargo clean --profile dev` removes rebuildable debug outputs,
not source or private environments. Do not use broad directory deletion as a
cleanup shortcut, and do not remove a rollback installation without a retention
decision. See [disk-failure recovery](troubleshooting.md#disk-space-or-permission-errors).
