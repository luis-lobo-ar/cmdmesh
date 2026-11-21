# CmdMesh

CmdMesh connects private infrastructure configuration to reusable operations
recipes. The Rust CLI and stdio MCP server share the same inventory, drift, and
static planning code. Integrations and recipes remain separate packages.

- [cmdmesh-proxmox](https://github.com/luis-lobo-ar/cmdmesh-proxmox) provides a
  read-only Proxmox API executable, an Ansible collection, and an LXC Terraform module.
- [cmdmesh-recipes](https://github.com/luis-lobo-ar/cmdmesh-recipes) combines those
  tools into reviewed workflows.
- Your own repository supplies inventory, secrets, targets, and policy.

v0.1 supports approved Ansible check and apply execution through the CLI.
Execution defaults to denied. MCP has no execution tool. Terraform saved-plan
execution remains a direct Terraform workflow, outside CmdMesh v0.1.

## Build and try offline drift

CmdMesh supports Unix systems. The checked-in toolchain pins Rust 1.92.0 for
repeatable builds and formatting. Python 3 is needed for the
synthetic executable tests and the Proxmox integration.

```sh
cargo build --locked --release
./target/release/cmdmesh --help
./target/release/cmdmesh drift detect \
  --desired examples/desired.json \
  --inventory tests/fixtures/inventory.json
```

The example reports no drift. Both files contain synthetic data, and the command
does not contact infrastructure.

## Use a private environment

Copy [examples/environment.json](examples/environment.json) into your private
configuration repository. Resolve paths relative to that file, not your current
directory. Replace the example paths and supply credentials through the explicit
environment allowlist.

```sh
export PROXMOX_TOKEN_ID='inventory@pve!readonly'
read -r -s PROXMOX_TOKEN_SECRET
export PROXMOX_TOKEN_SECRET
cmdmesh --config ./private/environment.json inventory pull \
  --provider proxmox --out ./private/inventory.json
```

The integration requires HTTPS with verified certificates and a read-only API
token. A private CA can be supplied through `ca_file`. The core invokes only the
configured executable. It never downloads an executable when it reads a recipe.

Inventory output includes `complete`. A partial inventory remains partial,
and drift detection never reports it as clean. Keep inventory and plans private.
`--out` creates an owner-only file and refuses to overwrite an existing file.

## Plan and execute a recipe

Planning is static. It hashes the recipe, dependencies, tool identity, and private
inputs without starting Ansible, Terraform, or a provider. A plan describes the
command to run. It is not an Ansible change preview.

```sh
cmdmesh recipe list --catalog ./cmdmesh-recipes
cmdmesh recipe inspect ./cmdmesh-recipes/recipes/proxmox-host-baseline/recipe.json
cmdmesh --config ./private/environment.json recipe plan \
  ./cmdmesh-recipes/recipes/proxmox-host-baseline/recipe.json \
  --target lab --operation check --out ./private/baseline-plan.json
```

Review the private plan and trusted playbooks. To permit execution, set
`execution.allow_execution` in the private environment **before** creating the
plan. An apply operation also requires `execution.allow_mutations`.

```sh
cmdmesh recipe execute --plan ./private/baseline-plan.json \
  --approve-digest <digest-returned-by-plan>
```

To retain Ansible task output and the recap, explicitly add
`--log-dir ./private/baseline-run-01`. The directory must not exist. CmdMesh
creates it owner-only and saves bounded raw streams and a safe execution report.
Raw logs may contain credentials. They are not redacted or printed on the terminal.
Without this option, raw output is discarded. See
[execution output](docs/execution.md#private-execution-output).

Version 0.1.1 creates a valid private `ansible.cfg` instead of pointing Ansible
at `/dev/null`. The configuration content is bound into each plan. Recreate plans
made by 0.1.0 before executing with 0.1.1. Execution and mutation permissions
remain denied by default.

Use a plan output directory outside recipe and package source trees. Changes to
bound configuration, inventory, variables, dependencies, or executables invalidate
the plan. Check mode can still have effects when trusted Ansible tasks, plugins,
or inventory code perform them. Approval is acknowledgement, not a sandbox.

CmdMesh uses a single explicit inventory host or group limit. It rejects `all`,
wildcards, and compound host expressions. Recipes that manage guest resource
fields require `lifecycle-owner:ansible`. Do not assign those same fields to
Terraform and Ansible.

## Connect an MCP client

Configure your MCP client to launch:

```json
{
  "mcpServers": {
    "cmdmesh": {
      "command": "/opt/cmdmesh/bin/cmdmesh",
      "args": ["--config", "/etc/cmdmesh/environment.json", "mcp"]
    }
  }
}
```

The SDK-backed server uses newline-delimited JSON-RPC on stdio. Stdout contains
protocol messages only. Available tools are `cmdmesh_inventory_pull`, `cmdmesh_drift_detect`,
`cmdmesh_recipe_list`, `cmdmesh_recipe_inspect`, and `cmdmesh_recipe_plan`. There is no apply or recipe
execution tool.

## Validate the build

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Set `CMDMESH_TEST_ANSIBLE` to a trusted Ansible executable to also run the
actual localhost executor regression test. CI installs Core 2.20.4 and requires
that test. No live inventory is used.

Tests use synthetic files and local executable doubles. They cover a real SDK
MCP handshake, CLI commands, private output permissions, stale plans, denied
execution, bounded child output, and process-group timeout cleanup. They do not
prove live infrastructure compatibility or disaster recovery.

See [the configuration reference](docs/configuration.md),
[execution limits](docs/execution.md), and
[the provider protocol](docs/provider-protocol.md).
