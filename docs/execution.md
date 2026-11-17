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

## Failure and interruption

The runner captures at most one MiB per child output stream. It does not return
raw output on failure or write raw logs. Results contain safe status only.
This protects secrets but limits diagnosis. Inspect the trusted tool locally
under your own logging policy when more detail is needed. Never paste raw
inventory, plans, or tool output into a public issue.

Local scratch directories are owner-only and are removed after execution.
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
