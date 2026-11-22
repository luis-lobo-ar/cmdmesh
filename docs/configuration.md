# Configuration reference

The environment is JSON with `schema_version: 1`. Unknown fields are rejected.
There is no automatic `.env` loading, private-parent discovery, or secret-store
decryption. Copy [the full template](../examples/environment.json) into your own
private environment. See [Proxmox setup](proxmox.md) for its expected directory layout.

## Root fields

| Field | Required or default | Meaning |
| --- | --- | --- |
| `schema_version` | Required, `1` | Environment format version |
| `providers` | `{}` | Named inventory or inspection integrations |
| `packages` | `{}` | Named local dependency roots |
| `targets` | `{}` | Named recipe targets |
| `execution` | Denied by default | CLI execution policy |

An inventory-only environment needs no targets or Ansible. Offline drift needs
no environment at all. Each target must supply its inventory, limit, and owner;
`vars_file` is optional. Unknown keys and incorrectly typed values are rejected.

## Path rules

| Path | Base directory |
| --- | --- |
| CLI `--config`, manifest, catalog, desired, inventory, plan, and output arguments | Current working directory |
| Provider `executable`, package roots, target `inventory` and `vars_file`, execution `ansible_executable` | Environment file's directory |
| Nonempty provider `config.ca_file` string | Normalized by the core relative to the environment file |
| Step `playbook` and recipe `recovery` | Recipe directory |

Other fields inside provider-specific `config` have integration-defined semantics;
the core does not resolve arbitrary custom path keys. Provider subprocesses run
from the environment directory. Recipe processes run from the recipe directory.
Absolute paths remain absolute. Plans record canonical resolved inputs, so moving
or editing bound files generally requires a new plan.

## Providers

`providers` maps a provider identifier to:

| Field | Meaning |
| --- | --- |
| `executable` | Explicit path to a locally installed, trusted integration executable. |
| `expected_sha256` | Optional SHA-256 of the executable file. Pin package source separately when an executable imports local modules. |
| `config` | Provider-specific JSON passed through the versioned protocol. |
| `env_allowlist` | Environment variable names forwarded to the child. The default is an empty list. |

Provider `config` defaults to JSON null; the integration defines its required
fields. `expected_sha256` is optional and covers only the executable entrypoint,
not every imported module or dependency. Pin and review the whole integration
checkout separately. Executables are local trusted code, never auto-downloaded.

Native inspections use a separate entrypoint in a dedicated provider slot. For
`inspection describe/run`, the executable pin is mandatory and `config` must be
an object. Core captures pinned bytes before launch. Use a self-contained artifact
to cover its implementation. See [native inspection configuration](inspections.md).

The runner supplies a fixed `PATH`, `LANG=C.UTF-8`, and `TERM=dumb`. It creates
private temporary directories and sets `TMPDIR`, `TMP`, `TEMP`,
`ANSIBLE_LOCAL_TEMP`, and `ANSIBLE_SSH_CONTROL_PATH_DIR`. It clears other inherited
variables. Code-loading variables such as `LD_PRELOAD`, `PYTHONPATH`,
`ANSIBLE_CONFIG`, and `BASH_ENV` cannot be allowlisted.

Provider calls have a 120-second outer timeout and a one-MiB limit per output
stream. The Proxmox integration also bounds individual network calls. An
integration is trusted code, not isolated code.

## Packages and targets

`packages` maps dependency names to locally checked-out package directories.
For example, `proxmox` can point to a pinned `cmdmesh-proxmox` checkout. A package
containing `ansible/ansible_collections` supplies that collection search root.

`targets` maps an explicit target name to:

| Field | Meaning |
| --- | --- |
| `inventory` | Ansible inventory file or directory. |
| `limit` | Single explicit host or group. `all`, `ungrouped`, wildcards, and expressions are rejected. |
| `vars_file` | Optional explicit Ansible variables file. |
| `lifecycle_owner` | `terraform` or `ansible`. Growth recipes require the latter. |

Targets, providers, packages, and recipe identifiers use letters, digits,
underscores, and hyphens. v0.1 host limits use those characters too. Use an
inventory alias for a dotted hostname or an address.

## Execution policy

`execution` contains:

| Field | Default | Meaning |
| --- | --- | --- |
| `allow_execution` | `false` | Permit CLI execution of approved Ansible plans. |
| `allow_mutations` | `false` | Also permit an `apply` plan. |
| `ansible_executable` | Fixed search path | Optional explicit executable path, useful for a pinned virtual environment. |
| `timeout_seconds` | `300` | Per-step timeout, between 1 and 3600 seconds. |
| `env_allowlist` | `[]` | Explicit execution environment references, such as `SSH_AUTH_SOCK`. |

Without an explicit `ansible_executable`, the runner searches `/usr/local/bin`,
`/usr/bin`, and `/bin`, in that order. It does not search your shell's custom PATH.
Set this field when using a virtual environment. Ansible Core 2.20.4 is the
qualified example controller; the companion Proxmox collection declares a
minimum of 2.19.0. Its Python controller requirement is separate from the
stdlib-only Proxmox API executable.

The per-step timeout must be an integer from 1 to 3600 seconds. Environment
variable names must use uppercase ASCII letters, digits, and underscores, and
cannot start with a digit. `PATH`, `LD_*`, `DYLD_*`, `PYTHON*`, `ANSIBLE_*`,
`BASH_ENV`, and `ENV` are refused as allowlist entries. Forward only what the
selected tool needs. Secret values are not embedded in plans, but same-user or
privileged processes can inspect environments. Trust the controller.

The runner writes a valid `ansible.cfg` into owner-only
per-process scratch, with file mode 0600. It sets `ANSIBLE_CONFIG` to that file,
so ambient configuration files are not selected. The configuration text is part
of the plan digest. Plans from 0.1.0 must be recreated because they lack that field.
The runner still forces host-key checking and disables automatic diffs.
Its SSH defaults disable multiplexing with `ControlMaster=no`, `ControlPersist=no`,
and `ControlPath=none`. This avoids OpenSSH's Unix-socket path-length limit in
nested private work directories. Each operation opens its own SSH connection.
Trusted inventory can still override connection settings.
It supplies the dependency collection search paths explicitly.
It does not inherit arbitrary Ansible options or pass through shell arguments.
Installed Ansible modules, default plugin locations, SSH configuration, and
declared recipe code remain trusted inputs. This is not a sandbox.

The private configuration is part of the plan digest. Changing policy after
planning requires a new plan. Credentials referenced through environment
variables are resolved at execution and are not embedded in a plan.

The final command supplies `cmdmesh_allow_changes` from the approved operation
and `lifecycle_owner` from the selected target. These values override matching
values in an earlier variables file. Target ownership remains authoritative.
