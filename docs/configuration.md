# Configuration reference

The environment is JSON with `schema_version: 1`. Unknown fields are rejected.
Paths resolve relative to the environment file. There is no automatic `.env`
loading or parent-repository discovery.

## Providers

`providers` maps a provider identifier to:

| Field | Meaning |
| --- | --- |
| `executable` | Explicit path to a locally installed, trusted integration executable. |
| `expected_sha256` | Optional SHA-256 of the executable file. Pin package source separately when an executable imports local modules. |
| `config` | Provider-specific JSON passed through the versioned protocol. |
| `env_allowlist` | Environment variable names forwarded to the child. The default is an empty list. |

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

The runner writes a valid `ansible.cfg` containing `[defaults]` into owner-only
per-process scratch, with file mode 0600. It sets `ANSIBLE_CONFIG` to that file,
so ambient configuration files are not selected. The configuration text is part
of the plan digest. Plans from 0.1.0 must be recreated because they lack that field.
The runner still forces host-key checking and disables automatic diffs.
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
