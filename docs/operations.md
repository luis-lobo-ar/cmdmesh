# Find and prepare a known operation

Use operation discovery when you know the task but do not remember which recipe,
private inputs or recovery procedure to use. Instead of reconstructing commands
from an old conversation, search the installed catalog and inspect its contract.

Schema1 prepares existing static Ansible recipe plans. Schema2 prepares the fixed
verified memory profile: planning reads current remote state and writes private
context, but never dispatches a mutation. Discovery and input validation stay
local and do not certify declared prerequisites. Execution remains CLI-only.

## Try the local example

From the repository root after building CmdMesh:

```bash
cmdmesh operation list --catalog examples/local --query 'local tools'
cmdmesh operation inspect 'check local tools' --catalog examples/local
cmdmesh --config examples/local/environment.json operation validate-inputs local-check \
  --catalog examples/local --inputs examples/local/operation-inputs.json
```

The result identifies `local-check`, its required private `target`, supported tool
versions, exact `recipe-plan` entrypoint, expected result and known failures.
Validation reports `valid:true`, `execution:"not-run"` and
`requirements_evaluated:false`. It checks the target reference without echoing
input values. It does not prove Ansible is installed or that any host is healthy.

Use [the local setup](../README.md#run-a-local-recipe) to create `.generated/local`
and its Ansible controller, then prepare a new private plan:

```bash
cmdmesh --config .generated/local/environment.json operation plan local-check \
  --catalog .generated/local --inputs .generated/local/operation-inputs.json \
  --out .generated/local/operation.plan.json
```

The plan uses the existing schema-1 recipe contract and `check` mode. Execute it
only through `recipe execute --plan ... --approve-digest ...`, following the
existing execution policy. This metadata adds no execution or approval bypass.
The output parent must exist; output is mode 0600 and refuses existing paths.

## Author an operation contract

For schema1, place `operation.json` beside the runnable recipe's `recipe.json`.
Schema2 needs no recipe file; the reviewed core owns its fixed implementation.
Discovery scans
`recipes/<name>/operation.json` and `operations/<name>/operation.json` below the
selected catalog. Existing recipes without a sidecar keep their old behavior;
they simply do not appear in operation discovery.

The [complete local operation](../examples/local/recipes/local-check/operation.json)
shows the schema. All objects reject unknown fields:

| Field | Contract |
| --- | --- |
| `document_type` | Exactly `cmdmesh.operation`; distinct from recipe documents |
| `schema_version`, `operation_version` | Schema1 for recipe-plan; schema2 for the fixed procedure-plan; operation_version remains1 |
| `id`, `aliases` | Stable identifier and intent phrases; trimmed, case-insensitive lookup rejects collisions across IDs and aliases |
| `title`, `summary` | Human task and result, without private environment assumptions |
| `parameters` | Exact mandatory typed parameters for the selected handler; see below |
| `requirements` | Capability ID, protocol version and explicit supported tool versions; declarations, not completed checks |
| `entrypoint` | Schema1: `recipe-plan` with colocated recipe.json/check or apply. Schema2: `procedure-plan`, profile `proxmox-lxc-memory`, profile_version1 |
| `expected_results` | What the operation actually prepares, without claiming downstream verification |
| `known_failures` | Unique code, explanation, next checks and recovery guidance |
| `provenance` | Public source references, qualification scope and explicit limitations |

Arbitrary executables, shell commands, invented parameters and unsupported
schema/driver/profile combinations are rejected. A recipe `apply` entrypoint
only prepares data; it does not bypass CLI execution policy or digest approval.

Colocation matters: the existing plan fingerprints the recipe tree, including
`operation.json`. Changing these instructions invalidates the plan alongside
playbooks and private inputs. Planning also checks that the captured tree binds
the exact metadata bytes loaded for the entrypoint, including an intervening
change followed by restoration. Old Plan1 wire fields stay unchanged.
For schema2 the exact operation bytes loaded during discovery are bound to the
outer procedure plan; later metadata changes prevent dispatch. No approval
service or multi-user signature is implied by the local digest.

Do not put credentials, actual targets, internal URLs or raw incident transcripts
in public metadata. Use a private environment target name as the input; its
inventory and variables resolve through the existing private configuration.

## Prepare the verified memory profile

A schema2 entry declares the `verified-procedure` capability, protocol1, and:

```json
{"driver":"procedure-plan","profile":"proxmox-lxc-memory","profile_version":1}
```

It requires exactly these private inputs:

| Parameter | Declared type | Consumed as |
| --- | --- | --- |
| `inspector` | `inspector` | Existing pinned provider reference |
| `state_directory` | `state-directory` | Owner-only durable journal/context location |
| `request` | `memory-request` | Native exact target/identity/ownership request |
| `desired_memory_mb` | `memory-mb` | Desired memory ceiling |
| `capacity_policy` | `memory-capacity-policy` | Explicit observed-headroom policy |

Input validation checks top-level shapes and references, without invoking an
integration. Full nested domain constraints, source identity and current capacity
are qualified by the pinned native profile during planning. The procedure
consumes all five inputs; metadata cannot substitute a command or SSH option.

Run ordinary `operation plan` with that private input file. Its output is a
`cmdmesh.procedure.plan`, consumed through `procedure execute --plan ...
--approve-digest ...`, rather than a legacy recipe plan. MCP uses the same
dispatch function. See [configuration, outcomes and recovery](verified-procedures.md).

The Rust `operations::plan` API retains its legacy recipe return type.
`operations::prepare` dispatches both contracts and serializes legacy Plan1 JSON
unchanged for existing entries.

## Search and failure behavior

Search matches all query words against IDs, aliases, titles and summaries. It
returns compact summaries, `total` and `next_offset`. Default page size is 20;
`--limit` accepts 1–100. The catalog is validated before any page is returned;
an invalid hidden entry cannot disappear behind pagination.

Invalid parameters return a structured report and CLI exit 1. Diagnostic codes
are `unknown-input`, `missing-input`, `invalid-input-type`,
`configuration-required`, `unknown-target`, `unknown-inspector` and `invalid-input-document`.
Unknown values are not echoed. Reports never claim that prerequisites or host
postconditions ran. Unknown aliases, colliding metadata, unsafe paths and
unsupported versions fail before a plan is created. There is no automatic repair.

## Use from an agent

CLI and the grouped `cmdmesh_operation` MCP tool call the same implementation.
Start with `action:"list"` and a query; inspect the returned canonical ID; validate
a private input file; then request `action:"plan"`. Schema1 returns a static
plan as JSON without writing a plan file. Schema2 retains private procedure
context/plan.json and returns that bound plan. Its grouped tool hints reflect
local writes and remote reads. No execution action or session is exposed.
Use CLI `operation plan --out` or the retained procedure plan for reviewed execution.

An agent can now discover the supported command, required inputs and known next
checks from the installed artifact. Whether it uses them better is measured by
the later fresh-agent evaluation; this feature does not claim zero agent errors.
