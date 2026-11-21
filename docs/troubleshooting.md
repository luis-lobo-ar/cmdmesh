# Troubleshooting

Do not fix a diagnostic by granting broad privileges, disabling TLS/SSH trust,
changing an immutable plan, enabling mutations, or deleting old evidence.
Inspect the real cause, then make a new plan when bound inputs change.

## Command or configuration errors

For exit 2, check `cmdmesh <command> --help`. Flags belong to their documented
subcommand; there is no global force, insecure, dry-run, or Terraform mode.

For `--config is required`, supply an environment to inventory or planning.
For invalid JSON/version/unknown-field errors, compare the file with the
[configuration reference](configuration.md). Do not pass an Ansible inventory or
a snapshot where an environment is expected. Only schema version 1 is accepted.

For a missing file, distinguish command-line paths from environment-relative
paths. Run the README examples from the repository root. Put private output
outside source trees and create its parent directory first.

## Incomplete Proxmox inventory

Inspect `complete` and `warnings`, even after exit 0. Check the HTTPS origin,
token environment names, explicit allowlist, token ID format, read-only ACL
visibility, and connectivity. Do not print secret values to debug their presence.

For TLS failures, verify the trusted CA and certificate hostname/IP match.
Obtain the CA independently, not from an unverified connection. Redirects are
refused; configure the intended origin rather than an API path or login page.
There is no insecure fallback. Do not drive deletions from a partial snapshot.

## No Ansible executable

The generic runner uses its configured executable or fixed system search paths,
not an arbitrary shell PATH. Set `execution.ansible_executable` to the trusted
virtual environment's executable. Install it before creating the execution plan.
The Proxmox collection declares Core >=2.19; the examples qualify 2.20.4 with
controller Python >=3.12. A warning from an older controller is not compatibility proof.

The runner owns a valid private `ansible.cfg`; a user config or exported
`ANSIBLE_*` option is not automatically inherited. Put intended host connection
settings in your reviewed inventory rather than assuming ambient settings apply.

## Execution disabled or approval mismatch

A check needs `allow_execution:true`. An apply also needs `allow_mutations:true`.
Make approved policy changes before planning, then generate the correct operation
and review its new digest. Copy the plan's `digest`, not a hash of the JSON file.
Do not reuse a check digest for an apply or edit the plan to get past verification.

For a lifecycle ownership refusal, inspect the real owner of the guest fields.
Changing a label does not transfer Terraform state or resolve competing writers.

## Stale plans and existing outputs

Bound environment, inventory, variables, packages, recipe, tool, or configuration
content changed. Even whitespace changes in a bound input alter its byte hash.
Create a new plan after reviewing the change. Plans from core 0.1.0 must be
recreated for 0.1.1 because managed configuration is now a required plan field.

If a plan is stale immediately, it may have been written into its own bound
recipe/package/inventory directory. Put outputs outside all bound trees.
Existing output files and log directories are refused. Pick new names; do not
overwrite the previous run or remove a live lock file to force execution.

## Child operation failed

Default errors withhold raw tool output to protect secrets. If another run is
appropriate and approved, use a new private `--log-dir` to retain bounded stdout,
stderr, and an execution report. Read those files locally; do not paste them
into public issues. Logging does not redact secrets or enable Ansible diffs.

Inspect the recap, expected host count, and recipe verification instructions.
Ansible can exit successfully without matching a host, so `completed_steps:1`
is not proof that a host or application was checked. Confirm that the limit
matches an intended host/group under the recipe's `cmdmesh_targets` group.

## SSH control paths or missing credentials

Core 0.1.1 disables SSH multiplexing in its managed defaults to avoid long
control-socket paths. If that failure returns, inspect trusted inventory overrides
such as `ansible_ssh_args`. Do not move sockets to a shared system-temp directory
or turn off host-key checking as a shortcut. Each normal operation has its own
SSH handshake.

API credentials do not authenticate SSH. Check the configured SSH user, key,
verified known-hosts state, and any required `SSH_AUTH_SOCK` allowlist reference.
The core does not decrypt Ansible Vault or SOPS for you. Keep that handling in a
reviewed private credential-loading layer.

## Timeout, cancellation, or output limit

The runner terminates its owned local process group, but remote work may already
have run or may outlive SSH. Detached local sessions can also escape that group.
This is not a sandbox. Inspect actual target state before retrying; failure is
not rollback. Per-step timeouts are configured from 1 to 3600 seconds. Each child
stream is capped at one MiB; private logs may retain only a prefix.

## Disk space or permission errors

Check free bytes, inodes, and directory ownership on both controller and target.
An Ansible module-transfer error can refer to controller scratch, not the remote
host. Runtime scratch is under the environment directory's
`.generated/cmdmesh-runs`; log parents must exist and be writable by the operator.

Do not delete credentials, user data, plans, or rollback bundles to make space.
Remove only positively identified disposable artifacts under your normal change
policy. For a source checkout, its debug build cache is rebuildable using
`cargo clean --profile dev` from that checkout. Source and private inputs are not
replaced by that command.

A disk failure can leave empty/partial logs or no final report. Inspect target
state before retrying, even if an error appears local. Use new plan/log filenames
after recovery; never infer completion or rollback from a missing report.

## MCP does not connect

Use a stdio client with an absolute executable path. Do not configure an HTTP
port or expect the command to open a browser. Keep stdout protocol-only and
use absolute tool file paths. See [the MCP guide](mcp.md) for exact arguments.
