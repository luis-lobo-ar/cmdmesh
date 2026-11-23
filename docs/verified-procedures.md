# Verify owned LXC memory growth

Use this profile to grow one explicitly owned running LXC and verify both its
configured memory and enforced cgroup limit. CmdMesh collects fresh evidence,
freezes intent, runs one guarded Ansible mutation and independently checks the
result. It does not infer success from an Ansible exit code.

This first bounded procedure is `proxmox-lxc-memory`, profile version1. It does
not qualify swap, shrinking, restarts, application health or arbitrary Proxmox
versions. Real disposable growth and recovery qualification remain a separate
milestone. Existing Recipe1/Plan1 results retain command-completion semantics.

## Prepare private inputs

Copy the [environment template](../examples/procedure/environment.json) and
[memory parameters](../examples/procedure/memory-parameters.json). Replace their
documentation values with qualified node identity, ownership/generation tags,
tool paths and trust files. Obtain fingerprints over an already trusted channel;
never disable TLS or SSH verification. Create the inspector's owner-only scratch
directory before planning.

Configure the matching reviewed `cmdmesh-proxmox` package and SHA-pinned native
inspector supporting `lxc-memory-procedure` profile1. Its `inspection describe`
response supplies the full native request schema and supported versions. The
Ansible path in `controller.tools` must select the environment's actual Ansible.
Runtime interpreters and installed dependencies remain trusted code.

The templates disable execution and mutations. Enable only intended permissions
in your private environment before planning; CmdMesh never changes them. An
unchanged target needs execution permission but no mutation permission. Capacity
reserve is your policy, checked against fresh reported available memory; it is
not a reservation of future capacity.

## Review and run

From your private operations project, with new output filenames:

```bash
install -d -m 700 .generated/procedure-state
cmdmesh --config environment.json procedure plan \
  --inspector proxmox-inspector \
  --input memory-parameters.json \
  --state-directory .generated/procedure-state \
  --out memory-plan.json
```

Planning reads remote state and writes an owner-only frozen context; it does not
mutate the guest. Review the exact subject, desired memory, policy, bindings,
driver and digest in the plan. Then supply that reviewed digest:

```bash
cmdmesh procedure execute --plan memory-plan.json \
  --approve-digest REVIEWED_PLAN_DIGEST
cmdmesh procedure status --plan memory-plan.json
```

Execution exits0 only for `checked` or `verified`. Rejected, unverified or uncertain
results are JSON with exit1. Status exits0 when it reads a valid historical
result, including an uncertain one; inspect its outcome rather than treating
the exit code as health.

## Boundaries and evidence

The outer plan binds the original environment/provider configuration/artifact,
assessment, references, original driver files, frozen inspector and generated
Plan1 context. It fingerprints the running CmdMesh executable; a changed binary
requires a new plan. CLI and MCP share that implementation. Parsing caller JSON
alone never establishes live evidence or mutation authority.

The private context has one literal host, fixed direct SSH, explicit root/key/
known-hosts/executable, no become and fixed Python. Legacy inventories and vars
are not imported. Keys are referenced, never copied. Only three captured,
verified driver/helper files form the minimal collection. Original action
plugins/routing cannot intercept it; later plugin additions invalidate the plan.

Execution checks policy, approval and inputs under a physical-subject lock,
recollects preflight without rewriting intent, and durably consumes the plan
immediately before dispatch. The guarded module independently checks identity,
baseline and capacity and uses the Proxmox configuration digest.

The fixed playbook retains filtered native command facts privately, even when
the module fails, then asserts module success. Controlled-Ansible completion and
native mutation facts remain separate. Missing, malformed or unknown native
facts cannot yield verified, even with passing postchecks. Raw module messages
are excluded from this facts artifact.

| Outcome | Meaning |
| --- | --- |
| `checked` | Fresh preflight confirms the unchanged target; no mutation dispatched |
| `rejected` | Preflight did not qualify before dispatch |
| `verified` | Controlled invocation and native mutation succeeded; required fresh postchecks passed |
| `unverified` | Known command facts and current evidence did not establish the complete procedure |
| `uncertain` | Completion, source, subject or retained final evidence is unknown |

`planned` and `pending` describe incomplete retained progress. A preflight
receipt proves neither controller liveness nor terminal rejection. Every result
is historical (`current_observation:false`). Passing checks after an unknown
mutation cannot establish command completion.

The context retains `plan.json`, `prepare.json`, `preflight.json`, private native
facts, `command.json`, `verify.json`, `result.json` and bounded private driver logs
as each stage completes. Raw logs may contain secrets. Failed preparation keeps
its diagnostic receipt and reports its path. Frozen inspector/configuration
supports attempted postchecks after original artifact loss. A command-evidence
write failure still attempts the postcheck and cannot produce a verified result.

## Recovery and limits

Controllers sharing one owner-only durable state directory coordinate by physical
subject key, independently of aliases. Other directories/machines do not share
the lock. Lock files are never unlinked. Intent bytes and directory entries are
synced before dispatch; this requires a filesystem honoring sync and a durably
provisioned parent path. Tests do not qualify hardware power-loss recovery.

Retain state. A consumed plan cannot run again, including after interruption or
missing final reporting. Inspect receipts and actual state, then make a new plan
only after reconciliation. No automatic retry, shrink, restart or rollback is
provided. Killing local SSH does not prove the remote command stopped.

MCP exposes describe/plan/status through `cmdmesh_procedure`, with no execute
action. Planning writes private artifacts and reads remote state, reflected in
its hints. See [the MCP guide](mcp.md).

Schema2 operation planning can dispatch this profile and bind its exact selected
metadata; see [operation discovery](operations.md). Public catalog delivery,
real disposable growth and independent restore,
fresh-operator evaluation and private adoption remain separate work. Local
fixtures do not complete those milestones.
