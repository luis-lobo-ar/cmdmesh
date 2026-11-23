# Verified procedure foundations

This work is under development. There is no procedure execute command or MCP
apply tool. Existing Recipe1/Plan1 execution still reports command completion,
not verified infrastructure outcomes.

The first profile is `proxmox-lxc-memory`, version1: grow one explicitly owned
running LXC's memory ceiling. The Proxmox inspector supplies read-only prepare,
preflight and verify phases; its guarded Ansible module independently checks
identity, baseline and capacity before one digest-guarded change. A zero command
exit alone cannot verify a procedure.

## Assessment boundary

Core's experimental `procedure` library parses the fixed composition contract:
exact profile/version/phase, prepared-body hash and subject, required boolean
check IDs, referenced files and a bounded driver transport. Prepare/preflight
require all checks; verification retains trustworthy false postchecks. The
native inspector remains responsible for the domain evidence schema, live
collection and authenticated identity. Parsing caller JSON is not live evidence
or change authority. The future envelope must obtain the assessment through the
configured, SHA-pinned inspection transport and bind it to the reviewed plan.

## Singleton execution context

The context builder compares the environment-selected Ansible executable and
referenced file bytes, then captures the actual collection's mutation module and
both helpers and checks those bytes against the bundled inspector profile. It
copies exactly those three files into a private minimal collection. Original
action plugins and routing metadata cannot intercept the fixed module. Tests
also exercise a duplicate collection on the trusted tool's Python search path.
The builder writes only into a new, empty, owner-only directory outside the
source package tree. Tool runtimes remain trusted; this is not a Python sandbox.

Generated inputs contain exactly one literal host, a fixed direct-SSH policy,
root identity, explicit known-hosts/key/executable paths, no become, fixed Python
and the unchanged prepared intent. The generated one-step Recipe1 invokes only
`cmdmesh.proxmox.lxc_memory`. Legacy target groups, dynamic inventories and vars
are not imported. Private keys are referenced, not copied. Execution permission,
mutation permission, timeout and environment allowlist retain their original
values. The source environment is not rewritten.

Plan1 binds this generated recipe, inventory, vars, derived environment and
minimal collection tree. Adding even an unused plugin to that frozen tree
invalidates the plan. The future outer plan must additionally bind the original
environment, inspector/configuration, assessment and reference files. A caller
that invokes only the context builder has not implemented those outer guarantees.
Trusted tool runtimes and Ansible dependencies remain trusted executable code.

## Still required

The bounded envelope must add approval binding, stable-subject locking, durable
once-only intent before dispatch, fresh preflight, typed command facts,
independent postchecks and retained phase receipts. Unknown command completion
must remain unknown even when later checks pass. No automatic retry, shrink,
restart or rollback belongs to this profile.

Catalog/CLI/MCP composition, real disposable growth and independent restore,
fresh-operator qualification and private adoption remain separate milestones.
Local fixture and inventory tests do not qualify live mutations or recovery.
