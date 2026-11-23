# CmdMesh

CmdMesh combines infrastructure inventory, desired-state comparisons, reusable
recipes, and reviewed operations. **Ansible is currently its only execution
driver.** The CLI and optional MCP server share the same inventory and planning
functions. Each environment keeps its own private configuration and credentials.

## Why CmdMesh?

### What it does beyond Ansible

These functions do not run Ansible:

| Function | What CmdMesh provides |
| --- | --- |
| Infrastructure inventory | Calls a configured provider, such as the Proxmox API integration, and returns resource snapshots as JSON |
| Desired-state comparison | Compares desired and observed JSON locally, identifies differences, and preserves incomplete-inventory status |
| Recipe discovery and planning | Lists and validates recipe manifests, then builds static plans recording commands and versions of supported input files |

Inventory and drift comparison work without an Ansible installation. Static
planning does not preview live changes. See the [CLI reference](docs/cli.md).

### What it wraps

For recipe execution, CmdMesh starts `ansible-playbook` in check or apply mode.
It checks the supplied plan digest and recorded inputs before execution. It
enforces configured permissions, holds a local target lock, and returns JSON
status. Raw logs are optional. Ansible still performs the package or host work.

Terraform execution remains separate. You still provide trusted playbooks,
private inputs, credentials, approvals, and outcome checks. The
[execution reference](docs/execution.md) describes the exact limits.

### When to use it

Use CmdMesh when you repeat or share infrastructure procedures and want the same
discovery, review, and execution checks across private environments, operators,
and AI clients. It can replace supporting scripts you would otherwise maintain.

For a one-off playbook run, direct Ansible can be enough. If your existing
automation already supplies these checks, CmdMesh is an optional alternative.

### A simple example

You and a colleague want `jq` and `htop` on your respective Proxmox hosts. You
share the host-baseline recipe. Each private environment supplies its own SSH
inventory and [package variables](examples/proxmox/vars.yml) selecting those
packages, with no package removals.

1. Prepare a check plan for the named target `lab`.
2. Review its target, command, and recorded input files.
3. Run the approved check through the CLI. Ansible reports the package changes
   it would make. An apply needs a separate, freshly reviewed apply plan.

If someone edits the bound inventory or variables after review, CmdMesh refuses
the old plan before starting Ansible. Without CmdMesh, checking those file
versions needs separate tooling. Your colleague reuses the recipe with their
own inputs.
Follow the [host-baseline walkthrough](#plan-a-real-host-baseline) for commands.

### Why it helps AI agents

Give an agent the CmdMesh MCP connection and your catalog's actual path. With
that private `lab` target already configured, an example request is:

> Using the catalog at `/opt/cmdmesh-recipes`, find the host-baseline recipe and
> prepare a check plan for `lab`. Do not execute it.

The agent can start with `cmdmesh_operation` to search by intent, inspect typed
inputs and known failures, then prepare the supported static plan. Existing
clients can still use `cmdmesh_recipe_list`, `cmdmesh_recipe_inspect`, and
`cmdmesh_recipe_plan` through MCP to discover the actual procedure and return
a static plan for review. It does not need to invent a playbook or SSH command.

- Discoverable recipes give the agent existing procedures to choose from.
- JSON results expose fields such as `complete` and `clean` for the agent to inspect.
- Named targets reuse private configuration instead of repeating host details in prompts.
- The CLI validates the reviewed plan and configured policy before execution.

The same core is available to agents through the CLI. **MCP has no execution
tool.** A client with terminal access has separate permissions; CmdMesh is not
a sandbox for that client. See [MCP setup and tool arguments](docs/mcp.md).

![The same Ansible package task with and without CmdMesh: without it, the operator connects input selection, checks, review validation and execution reporting; with it, CmdMesh supplies the common review and execution workflow. Both require operator verification.](docs/images/with-and-without-cmdmesh.svg)

See the [concrete comparison](docs/why-cmdmesh.md) for the direct Ansible path and
the responsibilities that stay with you.

## Where the project is going

The goal is to turn accumulated DevOps conversations, fixes, scripts and runbooks
into reusable components, starting with Proxmox. Agents should find a tested
procedure instead of repeatedly reconstructing how to do the task from old chats.

The initial source audit is complete within documented coverage. Versioned
operation discovery describes existing recipe planning. Pinned native inspections
now expose target matching, evidence comparison and controller/access diagnosis
through the shared CLI/MCP transport. See [native inspections](docs/inspections.md).
The remaining work will:

- Complete qualification of extracted helpers with explicit private inputs.
- Extend operation contracts with executable requirements
  and postconditions; current discovery does not certify prerequisites or outcomes.
- Qualify Proxmox procedures and adopt them back into private workflows so shared
  logic is maintained in one place. Extend proven patterns into deployments later.

**Those next phases remain planned work.** The
[full extraction plan and roadmap](docs/roadmap.md) defines six core phases, a
later deployment-extension candidate, source-coverage rules and completion gates.

## What is included

Start with the offline examples below. You do not need Proxmox, cloud access,
credentials, or a private parent project to try them.

| Component | What it provides |
| --- | --- |
| **This repository** | Rust CLI, stdio inspection/planning MCP, pinned native transport, JSON drift, static plans, approved Ansible execution and a verified owned-memory procedure |
| [cmdmesh-proxmox](https://github.com/luis-lobo-ar/cmdmesh-proxmox) | GET-only API inventory, seven native inspection actions, eleven Ansible roles, guarded memory module and separate Terraform LXC module |
| [cmdmesh-recipes](https://github.com/luis-lobo-ar/cmdmesh-recipes) | Reviewed procedures that combine integration roles with verification and recovery instructions |

Core source is **0.3.0**. The Proxmox collection is **0.2.1**;
the native inspector reports **0.4.0** and uses a separate archive pin.
Operation discovery needs a catalog revision containing `operation.json`
sidecars. Existing schema-1 recipe commands remain compatible with older catalogs.
CmdMesh does not download plugins during normal operations, automatically
reconcile drift, or run Terraform apply. Execution and mutations default to
disabled. MCP has no execution or apply tool.

## Build from source

Use a Unix system with Git and Rust/Cargo. The repository pins Rust 1.92.0 in
`rust-toolchain.toml`; Rustup-managed Cargo uses that toolchain. Examples use Bash.
Linux is the tested controller platform; Windows is not supported.

```bash
git clone https://github.com/luis-lobo-ar/cmdmesh.git &&
cd cmdmesh &&
cargo build --locked --release &&
export PATH="$PWD/target/release:$PATH" &&
cmdmesh --version
umask 077
mkdir -p .generated
```

The PATH change affects this shell only. Run subsequent examples from this
repository root. For reproducible deployments, review and pin a full source
commit rather than following a moving branch. See [development and installation](docs/development.md).

## Try offline drift detection

This compares two checked-in JSON files. It makes no network calls or changes.

```bash
cmdmesh drift detect \
  --desired examples/desired.json \
  --inventory examples/inventory.json
```

Expected result:

```json
{
  "schema_version": 1,
  "clean": true,
  "complete": true,
  "items": []
}
```

Now try the changed snapshot:

```bash
cmdmesh drift detect \
  --desired examples/desired.json \
  --inventory examples/inventory-drift.json
```

The result is `clean:false`, with a `changed` entry for `node-a/101` and field
`maxmem`. The example expects 512 MiB but observes 1 GiB, expressed in bytes.
Nothing is resized. [Drift semantics and JSON formats](docs/drift.md) explain the comparison.

An incomplete snapshot is also available as `examples/inventory-incomplete.json`.
It produces `complete:false` and can never be clean. Missing data is not proof
that a resource was deleted.

**A successful process exit is not a clean-state assertion.** Valid drift and
partial-inventory reports can return exit code 0. For automation, inspect the JSON.
With `jq` installed:

```bash
cmdmesh drift detect --desired examples/desired.json \
  --inventory examples/inventory.json > .generated/drift-report.json &&
jq -e '.complete == true and .clean == true' .generated/drift-report.json
```

This example report contains synthetic data. Keep reports from real environments private.

## Run a local recipe

To find a task by intent first, use
`cmdmesh operation list --catalog examples/local --query 'local tools'`
and inspect its supported inputs and recovery guidance.
See [operation discovery](docs/operations.md) for the runnable example and contract.

This example runs an assertion and a message on the controller. It connects to
no remote hosts, gathers no facts, installs no packages, and requests no sudo.
CmdMesh still writes its own private plan, lock, scratch, and optional logs.

Install the example's Ansible controller with **Python 3.12 or newer**:

```bash
umask 077
mkdir -p .generated/tmp
python3 -m venv .generated/ansible
TMPDIR="$PWD/.generated/tmp" .generated/ansible/bin/pip install 'ansible-core==2.20.4'
```

Discover and inspect the local recipe, without executing it:

```bash
cmdmesh recipe list --catalog examples/local
cmdmesh recipe inspect examples/local/recipes/local-check/recipe.json
```

Create a new demo directory and copy its private configuration. Stop if the
directory already exists; use a fresh name instead of overwriting old inputs.

```bash
mkdir -m 700 .generated/local-demo &&
cp -n examples/local/environment.json examples/local/inventory.yml .generated/local-demo/ &&
cmdmesh --config .generated/local-demo/environment.json recipe plan \
  examples/local/recipes/local-check/recipe.json \
  --target local --operation check --out .generated/local-demo/check-plan.json
```

Open `check-plan.json`. Review its operation, command, inputs, and warning. Copy
its `digest` into the command below, replacing the quoted placeholder:

```bash
cmdmesh recipe execute --plan .generated/local-demo/check-plan.json \
  --approve-digest '<digest-from-reviewed-plan>' \
  --log-dir .generated/local-demo/run-01
```

Expect a JSON result with `status:"succeeded"` and `completed_steps:1`.
Read `.generated/local-demo/run-01/step-0001.stdout` for the message
`CmdMesh local check completed` and the `changed=0` Ansible recap.

Only this demo configuration sets `allow_execution:true` out of the box. It
keeps `allow_mutations:false`, and its playbook also refuses non-check execution.
Do not copy that execution permission into a real environment without review.

Plans and log directories are never overwritten. Use a new plan filename and
log-directory name for another run. Configuration paths resolve relative to the
configuration file; command-line paths resolve from your current directory.

## Connect real Proxmox inventory

Follow the [Proxmox setup guide](docs/proxmox.md) to clone the companion packages,
create a separate private environment, configure TLS trust, and replace the
fictional endpoint. API inventory and SSH recipe access are independent.

After that setup, export the token ID and read its secret without putting it in
shell history. These commands require your real values and reviewed configuration:

```bash
export PROXMOX_TOKEN_ID='inventory@pve!readonly'
read -r -s -p 'Proxmox token secret: ' PROXMOX_TOKEN_SECRET
export PROXMOX_TOKEN_SECRET
cmdmesh --config ../private/environment.json inventory pull \
  --provider proxmox --out ../private/inventory-01.json &&
jq -e '.complete == true and (.warnings | length) == 0' ../private/inventory-01.json
```

Choose a fresh snapshot filename for every pull. The `&&` prevents a failed pull
from validating an older snapshot. Do not remove it in automation.

Use a least-privilege read-only API token. Chain and hostname verification are
mandatory. There is no insecure TLS or password-login fallback. Inventory is not
automatically cached or converted into an Ansible host inventory.

## Plan a real host baseline

After completing the Proxmox guide, explicitly permit check execution in your
private environment with `allow_execution:true`, while keeping
`allow_mutations:false`. The supplied real-environment template defaults both
to false. Make that edit before creating the plan; later changes invalidate it.

Review the catalog and selected host:

```bash
cmdmesh recipe list --catalog ../cmdmesh-recipes
cmdmesh recipe inspect ../cmdmesh-recipes/recipes/proxmox-host-baseline/recipe.json
cmdmesh --config ../private/environment.json recipe plan \
  ../cmdmesh-recipes/recipes/proxmox-host-baseline/recipe.json \
  --target lab --operation check --out ../private/baseline-check-01.json
```

Review the private plan, then execute its check using the reviewed digest:

```bash
cmdmesh recipe execute --plan ../private/baseline-check-01.json \
  --approve-digest '<digest-from-reviewed-check-plan>' \
  --log-dir ../private/baseline-check-run-01
```

Planning is static, not an Ansible change preview. The check execution can connect
to a host. Trusted Ansible code can have effects even in check mode; it is not a
security sandbox. See [execution boundaries and recovery](docs/execution.md).

### Apply only after separate approval

An apply requires both execution permissions, a new `--operation apply` plan,
and approval of that plan's exact digest. Do not reuse a check digest or change
policy to bypass a failure. Set `allow_mutations:true` only for an approved
change, before creating its new plan. This example is a workflow, not approval
to apply:

```bash
cmdmesh --config ../private/environment.json recipe plan \
  ../cmdmesh-recipes/recipes/proxmox-host-baseline/recipe.json \
  --target lab --operation apply --out ../private/baseline-apply-01.json
cmdmesh recipe execute --plan ../private/baseline-apply-01.json \
  --approve-digest '<digest-from-separately-reviewed-apply-plan>' \
  --log-dir ../private/baseline-apply-run-01
```

The recipe's extra guards, ownership requirements, backups, and recovery procedure
still apply. Turning permissions back off also invalidates existing plans.

## Connect an MCP client

CmdMesh provides **stdio MCP**, not an HTTP server. Start it with an absolute
binary path and a private environment path. The following is a generic client
configuration shape; adapt it to your client's format:

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

The eight tools discover operations, run pinned inspections, read inventory,
compare drift and prepare recipe/procedure plans. `cmdmesh_procedure` also reads
historical status. Its planning writes private artifacts and reads remote state;
the tool hints disclose those effects.
**There is no MCP apply tool.** Inventory
invokes your configured trusted provider. Keep tokens in the launch environment,
not chat messages or tool arguments. See [MCP setup and tool arguments](docs/mcp.md).

## Documentation

| Goal | Guide |
| --- | --- |
| See the extraction process, Proxmox milestones and deployment extension | [Extraction plan and roadmap](docs/roadmap.md) |
| Understand the work CmdMesh replaces and what you still own | [With and without CmdMesh](docs/why-cmdmesh.md) |
| Find every CLI command, output, and exit code | [Command-line reference](docs/cli.md) |
| Grow owned LXC memory with independent postchecks and once-only intent | [Verified memory procedure](docs/verified-procedures.md) |
| Discover and run pinned read-only integration actions | [Native inspections](docs/inspections.md) |
| Understand every environment field and path rule | [Configuration reference](docs/configuration.md) |
| Write desired-state JSON and gate drift reports | [Inventory and drift](docs/drift.md) |
| Connect Proxmox, use recipes, and separate Terraform ownership | [Proxmox guide](docs/proxmox.md) |
| Review approval, logging, limits, failure, and recovery | [Execution reference](docs/execution.md) |
| Configure a client and understand MCP's read-only boundary | [MCP guide](docs/mcp.md) |
| Implement a compatible inventory provider | [Provider protocol](docs/provider-protocol.md) |
| Build, test, contribute, and validate examples | [Development guide](docs/development.md) |
| Diagnose configuration, TLS, stale plans, logs, and disk failures | [Troubleshooting](docs/troubleshooting.md) |

Private inventory, credentials, encrypted secrets, Terraform state, plans, backups,
and real operational logs do not belong in these public repositories. Ready recipes
and passing tests are not production, restore, or hardware certification.
