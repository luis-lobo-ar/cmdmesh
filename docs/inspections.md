# Run a configured inspection

Use native inspections for reusable observations and diagnosis that do not need
an Ansible playbook. The integration owns finite actions, typed input schemas and
domain checks. Core owns executable identity, bounded transport and receipts.
The same function serves CLI and MCP. There is no inspection apply action.

## Configure the integration

Build and review a standalone artifact first. For Proxmox, use the builder in
[cmdmesh-proxmox](https://github.com/luis-lobo-ar/cmdmesh-proxmox).
Record its SHA-256. Add a dedicated provider to your private schema-1 environment.
Select the inspection archive, not the legacy inventory executable: protocols differ.

```json
{
  "schema_version": 1,
  "providers": {
    "proxmox-inspector": {
      "executable": "/private/tools/proxmox-inspect.pyz",
      "expected_sha256": "REPLACE_WITH_THE_64_CHARACTER_LOWERCASE_SHA256",
      "config": {"schema_version": 1, "origin": "example"},
      "env_allowlist": []
    }
  }
}
```

The placeholder pin is deliberately invalid. Missing, malformed or mismatched
pins stop before launch. Core captures the pinned bytes in an owned mode-0700
executable under private project scratch and runs that copy. Changing the original
after capture cannot change this invocation's code.

The artifact must contain its implementation. The pin does not cover mutable
sibling imports, interpreters, libraries, external tools, environment credentials
or system trust. Integrations are trusted code, not sandboxed code. Supply secret
environment references through the allowlist only when needed. Never paste values
into inputs, metadata or chat.

## Discover and call an action

```sh
cmdmesh --config /private/environment.json inspection describe --inspector proxmox-inspector
cmdmesh --config /private/environment.json inspection run --inspector proxmox-inspector \
  --operation target-resolve --input /private/target-input.json --out /private/target-receipt.json
```

Describe returns action IDs, aliases, input schemas, expected results, failures
and recovery. It executes the selected pinned integration, so it needs an
environment. It does not search or invoke every provider.

The input file contains only the action's input object:

```json
{
  "source": {"mode": "live"},
  "selector": {"origin": "example", "node": "node-a", "kind": "proxmox.lxc", "vmid": 101}
}
```

Configure API/trust/credential references in private provider `config`. Direct
integration examples may wrap inputs in an envelope. Extract their `input`
object for core. Core supplies the protocol envelope and configuration.

Without `--out`, the command prints a receipt. With `--out`, it writes a new
mode-0600 file and refuses existing paths. The parent must exist. Treat receipts
as private operational data, unsuitable for public commits.

## Interpret outcomes

The receipt contains `inspector`, `executable_sha256`, `configuration_sha256`,
`request_sha256` and `result`. Hashes identify captured code and serialized inputs.
They are not signatures, remote attestations or mutation permission. Environment
credential values are not included in configuration hashes. Domain evidence and
freshness remain integration concerns.

| `result.outcome` | Meaning | CLI exit |
| --- | --- | --- |
| `observed` | Produced the action's scoped observation | 0 |
| `rejected` | Input or asserted evidence failed a condition | 1 |
| `unsupported` | Action, version or requirement unsupported | 1 |
| `incomplete` | Evidence cannot establish the result | 1 |
| `unavailable` | Observation could not be collected | 1 |

Valid negative results still produce receipts with diagnostic codes, next checks
and recovery. A successful process or MCP call alone is insufficient. A UID,
certificate or complete inventory proves neither application availability nor
backup recoverability, independent ACL visibility or mutation authority.

Malformed protocol, inconsistent status, wrong operation, excessive output,
interruption and transport failure produce no qualified receipt. Raw output
is withheld on validation/transport failure. Core never repairs, retries or
rolls back an uncertain observation automatically.

Files and combined requests are bounded to 16 MiB. The runner bounds stdout and
stderr separately to 1 MiB. `--timeout-seconds` is 1..120, default 120. Scratch is
private and project-local, with no system temporary fallback.

## MCP equivalent

First discover, then run the selected action:

```json
{"action":"describe","inspector":"proxmox-inspector"}
```

```json
{"action":"run","inspector":"proxmox-inspector","operation":"target-resolve","inputs":"/private/target-input.json","timeout_seconds":30}
```

These are arguments to `cmdmesh_inspection`. Describe accepts no operation or
inputs. Run requires both. Unknown fields/actions fail. Read-only/open-world
hints reflect that actions can probe configured endpoints. No apply, approval
or cleanup endpoint exists. Inspect `result.outcome` and diagnostics.

## Integration protocol reference

An inspector receives one JSON document on stdin, with no arguments:

```json
{"document_type":"cmdmesh.inspection.request","schema_version":1,"operation":"describe","input":{},"config":{}}
```

It returns only a result document on stdout:

```json
{"document_type":"cmdmesh.inspection.result","schema_version":1,"operation":"describe","outcome":"observed","data":{"actions":[]},"diagnostics":[]}
```

Type, schema and operation must match. Data must be an object. Unknown envelope
or diagnostic fields fail. Exit 0 means observed. Exit 1 means a valid negative
result. Other exits/signals are transport failure.

Negative results require diagnostics. At most 32 diagnostics are accepted. Each
has identifier `code`, nonempty `summary` (at most 1024 bytes), and nonempty
`next_checks` and `recovery` arrays. Each array has at most 16 nonempty strings,
each at most 2048 bytes. Do not emit credentials or raw service responses.

Core injects `CMDMESH_INSPECTION_PROCESS_GROUP=1` into the integration's owned
process group. Cooperative integrations must keep ordinary children in that group
so core can stop them on completion, timeout and cancellation. Validate that the
integration is its process-group leader before relying on this lifecycle contract.
The marker grants no authorization. An integration that deliberately escapes is
outside this trusted-code model.

This protocol is separate from inventory Snapshot 1 and recipe Plan 1. Environment
fields retain their existing meanings. The old inventory entrypoint's optional pin
remains compatible; the inspection entrypoint always requires a pin.
