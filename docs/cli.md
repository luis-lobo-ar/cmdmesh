# Command-line reference

All successful data commands print JSON, except `inventory pull --out`, which
writes the snapshot instead. Help and version output are text. Examples assume
`cmdmesh` is on PATH. The CLI supports Unix controllers.

## Global options

```text
cmdmesh [--config FILE] <command>
cmdmesh --help
cmdmesh --version
```

`--config` names a schema-version-1 environment JSON file. It is required by
inventory and planning, not by offline drift or recipe discovery. It can also be
supplied to `mcp` for tools that need an environment. `recipe execute` reads the
environment path already bound into its plan, not a new global configuration.

Command-line paths resolve from the current working directory. Known path fields
inside an environment resolve from that environment file's directory.

## Inventory

```text
cmdmesh --config FILE inventory pull --provider ID [--out NEW_FILE]
```

`ID` selects a key under `providers`, not a hardcoded provider type. The configured
executable receives the inventory protocol request. The result has
`schema_version`, `complete`, `resources`, and `warnings`.

Without `--out`, the snapshot is printed. With `--out`, the parent must exist and
the new file is mode 0600. Existing paths are refused. No implicit cache is read
or updated. A partial valid snapshot can exit zero; check `complete` and warnings.

## Drift

```text
cmdmesh drift detect --desired FILE --inventory FILE
```

This reads local JSON only and returns `schema_version`, `clean`, `complete`,
and `items`. Items describe `missing`, `changed`, or `unmanaged` resources.
Valid non-clean and incomplete reports still exit zero. It never applies a fix.
See [comparison semantics](drift.md).

## Recipe discovery

```text
cmdmesh recipe list --catalog DIRECTORY
cmdmesh recipe inspect MANIFEST
```

A catalog has a `recipes/<name>/recipe.json` layout. `list` returns a JSON array
of validated manifests. `inspect` returns one validated manifest. Neither executes
recipe code or requires Ansible. Draft recipes can be inspected but not planned.

## Recipe planning

```text
cmdmesh --config FILE recipe plan MANIFEST \
  --target ID --operation check|apply --out NEW_FILE
```

`ID` selects a key under `targets`. Its `limit` selects an explicit Ansible host
or group. The command validates the recipe and hashes private inputs, dependencies,
and tool identity. It does not start Ansible, a provider, or Terraform.

The new mode-0600 plan contains absolute resolved paths, the operation, commands,
input hashes, collection roots, managed configuration, digest, and warning.
Stdout contains its `digest`, `out`, and `warning`. Treat the plan as private.
The parent must exist; existing output files are refused.

Planning is permitted with execution disabled. An absent default Ansible tool can
appear in a static plan without a tool hash; install it and make a fresh plan
before execution. An explicitly configured missing executable fails planning.

## Recipe execution

```text
cmdmesh recipe execute --plan FILE --approve-digest DIGEST [--log-dir NEW_DIRECTORY]
```

Execution verifies the digest, bound inputs, environment policy, and target lock.
A check requires `execution.allow_execution`. An apply additionally requires
`execution.allow_mutations`. The successful result contains `schema_version`,
`plan_digest`, `completed_steps`, and `status:"succeeded"`. If logging is enabled,
it also contains `log_dir`. A completed step means its local process succeeded,
not that application or recovery acceptance passed.

The optional new directory is mode 0700 and its files are mode 0600. It holds
bounded `step-0001.stdout`, `step-0001.stderr`, and `execution.json` files. It
must be outside bound input trees. Raw streams can contain secrets and are never
printed in the normal CLI result. Without this option, raw streams are discarded.
See [failure and logging semantics](execution.md).

## MCP

```text
cmdmesh [--config FILE] mcp
```

This starts a newline-delimited JSON-RPC stdio server. Stdout is protocol-only.
It exposes five read-only tools, with no execution endpoint. File paths in tool
arguments should be absolute because the client controls the process directory.
See [MCP configuration](mcp.md).

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | The command returned its normal result, or help/version completed |
| `1` | Configuration, validation, I/O, provider process, planning, or execution failure |
| `2` | Command-line parsing error, such as an unknown flag or missing argument |

**Exit 0 does not assert complete inventory or clean drift.** Gate the result
flags in scripts. A valid provider response with `complete:false` remains a
normal core result. The standalone Proxmox executable has its own documented
direct-CLI exit behavior; do not confuse it with the core protocol wrapper.

There is no `--force`, `--insecure`, generic shell step, automatic drift repair,
global `--dry-run`, Terraform execution, or MCP apply command in this version.
