# Execution boundaries and recovery

CmdMesh v0.1 executes approved Ansible check or apply plans through the CLI.
It does not execute Terraform apply, arbitrary shell steps, downloaded code,
or MCP mutation requests.

## What a plan binds

The static plan includes SHA-256 bindings for:

- The private environment file and target selection.
- The recipe directory and each configured dependency package directory.
- The inventory file or directory, including automatic sibling `group_vars` and
  `host_vars` directories for an inventory file.
- The explicit variables file, if configured.
- The resolved Ansible executable when installed.
- The runner-owned Ansible configuration text.

Tree bindings exclude `.git`, `.generated`, `.terraform`, `.venv`, and `.ansible`.
Terraform lockfiles remain bound. Do not place
executable recipe inputs in the excluded directories. Symlinks inside recipe or
package trees are rejected, as are manifest paths with traversal components.
Each bound file is limited to 16 MiB, and a tree is limited to 10,000 files.

The digest covers plan contents, command arguments, and bindings. Before
execution, CmdMesh regenerates the plan from its source and checks the digest
again while holding a local target lock. An altered package, input, target, or
plan fails before a child starts. Lock files remain in place so two processes
cannot acquire different lock inodes for the same target.

A local lock does not coordinate controllers on other machines. Use an external
operations coordinator when several machines can change the same target.

## What approval does not prove

Ansible playbooks, plugins, dynamic inventories, installed tools, and locally
configured providers are trusted executable code. A declared driver or a plan
digest does not contain that code. Check mode is not a sandbox and does not
guarantee that every task is side-effect-free.

The executable fingerprint covers the entrypoint file, not its interpreter or
every installed Ansible module. Installed tools and their transitive dependencies
remain trusted external inputs. Pin and review those installations separately.

Review external includes and runtime dependencies. Bind their directories as
declared packages. CmdMesh cannot infer arbitrary files that trusted code might
read, nor prevent an administrator from modifying files between verification and
execution. Secret values supplied through environment references are not hashed
or included in a plan.

The runner appends `cmdmesh_allow_changes=false` to a check and `true` to an
approved apply. The Proxmox collection also checks that opt-in before mutation.
Terraform owns guest lifecycle fields unless a target explicitly selects
Ansible ownership. The `lifecycle-owner:ansible` recipe requirement enforces that
choice for guest resource and storage changes.

## Private execution output

The runner captures at most one MiB per child output stream. Without `--log-dir`,
it discards raw output and returns safe status only. Provider calls retain that
behavior and have no raw-log option. MCP still exposes no execution operation.

`recipe execute --log-dir <new-directory>` explicitly retains raw tool output.
Its parent must exist. The new directory is mode 0700. Files are mode 0600:

- `step-0001.stdout` and `step-0001.stderr` contain the first step's captured streams.
  Subsequent steps use increasing numbers.
- `execution.json` contains the plan digest, completed-step count, status, and
  log-directory path. A failed step is not counted as completed. Later steps
  do not start after a failure.

Existing destination paths, including symlinks, are refused. A log directory
cannot be inside any bound input tree. Log files are not plan inputs. The option
does not enable diffs or change the approved operation, execution permission,
mutation permission, target lock, or stale-plan checks.

Raw logs can contain secrets and are not redacted. Opting in authorizes local
retention, not publication. Keep them under your private configuration directory.
Do not paste raw inventory, plans, or output into a public issue. The CLI never
copies raw tool streams into its normal stdout or errors, even when logging is on.

Captured prefixes remain after ordinary nonzero exits, timeouts, cancellation,
and output-limit failures. Each stream file is bounded to one MiB. A timeout or
limit failure may leave only a partial prefix. Logs are not a durable audit store:
disk failure or a killed controller can leave incomplete files or no final report.
Deleting local logs does not undo remote operations.

## Failure and interruption

`status: failed` means the local execution did not complete successfully. It does
not prove that remote work stopped or rolled back. A missing report also leaves
the outcome unknown. Inspect the actual target before retrying.

Internally the shared executor retains typed controlled-process facts: success,
an ordinary nonzero exit, or unknown completion after a signal, deadline,
cancellation or supervision failure. Reserved timeout/signal-style statuses
124, 137 and 143 remain unknown conservatively. These facts do not establish
remote postconditions. The existing Plan 1 report and CLI error behavior stay
unchanged; they do not expose a verified-procedure result or authorize a retry.

Local scratch directories are owner-only and are removed after execution.
The managed SSH defaults do not use multiplexing sockets, so long private
directory paths do not require a system-temp socket fallback. This costs an
additional SSH handshake for each operation. Explicit trusted inventory overrides
can change those defaults and remain the operator's responsibility.
Timeout and SIGINT or SIGTERM cancellation terminate the owned local process
group, including SSH children that remain in that group. Code that deliberately
creates a separate session can escape the group. CmdMesh does not contain such
code, but nonblocking pipe cleanup still prevents it from holding the caller open.
Remote commands may already have run
or may outlive their SSH session. A timeout does not imply rollback.

After any uncertain outcome:

1. Inspect the actual target with read-only tools.
2. Compare the result with the recipe's verification instructions.
3. Follow the recipe's recovery instructions.
4. Create a new plan before another attempt.

CmdMesh does not offer generic exactly-once execution or automatic rollback.
Local tests do not qualify live host changes or backup restoration.
See [the verified memory procedure](verified-procedures.md) for the separate
fixed envelope with independent postchecks and once-only intent. Its guarantees
do not apply to Plan1.
