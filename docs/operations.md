# Find and prepare a known operation

Use operation discovery when you know the task but do not remember which recipe,
private inputs or recovery procedure to use. Instead of reconstructing commands
from an old conversation, search the installed catalog and inspect its contract.

This first operation contract prepares existing Ansible recipe plans. It does not
run tools, check the remote host or certify the declared prerequisites. Inventory,
recipe execution and their existing policy remain separate commands.

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

Place `operation.json` beside the runnable recipe's `recipe.json`. Discovery scans
`recipes/<name>/operation.json` and `operations/<name>/operation.json` below the
selected catalog. Existing recipes without a sidecar keep their old behavior;
they simply do not appear in operation discovery.

The [complete local operation](../examples/local/recipes/local-check/operation.json)
shows the schema. All objects reject unknown fields:

| Field | Contract |
| --- | --- |
| `document_type` | Exactly `cmdmesh.operation`; distinct from recipe documents |
| `schema_version`, `operation_version` | Both `1` in this implementation; unsupported versions fail |
| `id`, `aliases` | Stable identifier and intent phrases; trimmed, case-insensitive lookup rejects collisions across IDs and aliases |
| `title`, `summary` | Human task and result, without private environment assumptions |
| `parameters` | Exactly a mandatory `target` with type `target`; this matches the implemented handler |
| `requirements` | Capability ID, protocol version and explicit supported tool versions; declarations, not completed checks |
| `entrypoint` | Driver `recipe-plan`, colocated `recipe.json`, and fixed operation `check` or `apply` |
| `expected_results` | What the operation actually prepares, without claiming downstream verification |
| `known_failures` | Unique code, explanation, next checks and recovery guidance |
| `provenance` | Public source references, qualification scope and explicit limitations |

The only current handler is `recipe-plan`. Arbitrary executable paths, shell
commands, invented parameters and unsupported drivers are rejected. More native
operations require implemented integration contracts and new tests before they
can be advertised. An `apply` entrypoint still only prepares an apply plan; its
execution needs the existing CLI policy and exact approved digest.

Colocation matters: the existing plan fingerprints the recipe tree, including
`operation.json`. Changing these instructions invalidates the plan alongside
playbooks and private inputs. Old manifests/plans are not extended silently.
No approval service or multi-user signature is implied by the local digest.

Do not put credentials, actual targets, internal URLs or raw incident transcripts
in public metadata. Use a private environment target name as the input; its
inventory and variables resolve through the existing private configuration.

## Search and failure behavior

Search matches all query words against IDs, aliases, titles and summaries. It
returns compact summaries, `total` and `next_offset`. Default page size is 20;
`--limit` accepts 1–100. The catalog is validated before any page is returned;
an invalid hidden entry cannot disappear behind pagination.

Invalid parameters return a structured report and CLI exit 1. Diagnostic codes
are `unknown-input`, `missing-input`, `invalid-input-type`,
`configuration-required`, `unknown-target` and `invalid-input-document`.
Unknown values are not echoed. Reports never claim that prerequisites or host
postconditions ran. Unknown aliases, colliding metadata, unsafe paths and
unsupported versions fail before a plan is created. There is no automatic repair.

## Use from an agent

CLI and the grouped `cmdmesh_operation` MCP tool call the same implementation.
Start with `action:"list"` and a query; inspect the returned canonical ID; validate
a private input file; then request `action:"plan"`. MCP returns the static plan
as JSON without writing a plan file. It has no execution action or session state.
Use CLI `operation plan --out` for the private artifact needed by execution.

An agent can now discover the supported command, required inputs and known next
checks from the installed artifact. Whether it uses them better is measured by
the later fresh-agent evaluation; this feature does not claim zero agent errors.
