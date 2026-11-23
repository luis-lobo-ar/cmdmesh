# CmdMesh extraction plan and roadmap

## The goal

Turn accumulated operating knowledge into reusable, tested components so an
operator or AI agent does not have to rediscover **how to do the same task**.

The source is real DevOps work: conversations, incidents, corrections, scripts,
libraries, runbooks and execution evidence. Start with **Proxmox management**.
Extend proven patterns into broader deployment strategies after the first
Proxmox workflow is qualified.

The finished product should supply the procedure, its implementation, required
inputs, known failure cases, verification and recovery guidance. A private
environment supplies the actual hosts, policy, credentials and state. Each run
still inspects relevant current state; historical instructions are not live facts.

**Status on November 22, 2026:** the reusable foundation exists. P1–P6 below are
planned extraction/adoption work. P7 is a later deployment-extension candidate.
This is a product plan, not a list of commands available today. No dates or
future release numbers are promised.

## A concrete example of the intended result

An agent is asked to increase an LXC disk. Today, an environment-specific process
can require rediscovering which node owns the guest, the correct access method,
which tool owns its disk, the supported growth operation and how to verify it.

The intended CmdMesh flow is:

1. Find the installed, versioned procedure for LXC disk growth and inspect its
   supported inputs, prerequisites, limits and examples.
2. Resolve the named private target and inspect the actual node/guest, ownership,
   current size and available capacity through known components.
3. Prepare the supported operation with explicit inputs. Refuse the wrong owner,
   a shrink request, an ambiguous target or changed reviewed inputs.
4. Run the approved procedure using the configured execution path. Verify the
   actual resulting capacity and guest state, not only the command's exit code.
5. Return a structured result or a known failure explanation with the appropriate
   next inspection/recovery step.

The agent reads one operation contract and its results instead of reconstructing
commands from old chats. The same component works in another private environment
with different node names, access settings and credentials.

Parts of this flow exist today. Complete capability discovery, executable
preconditions/postconditions and the full outcome contract remain planned.

## What we are extracting

| Layer | What becomes reusable | What stays private |
| --- | --- | --- |
| Capability primitives | Target resolution, inspection, access diagnostics, managed scratch, supported tool invocation, artifact checks and structured results | Real addresses, access identities, trust anchors and environment policy |
| Integration modules | Proxmox API, Ansible roles and reviewed native-tool adapters | Provider endpoints, resource identities, credentials and state |
| Operation recipes | A known sequence with prerequisites, inputs, verification, failure handling and recovery | The selected targets, private variables, approvals and actual evidence |
| Agent knowledge/discovery | Searchable task names and aliases, supported versions, examples, known failure signatures and implementation references | Raw conversations, incident details, customer data and operational history |
| Deployment strategies, later | Artifact qualification, staged rollout, convergence observation, compatibility reporting and rollback mechanics | Application-specific policy, topology, migrations, cache decisions and availability requirements |

Ordinary documentation remains useful, but repeating an instruction is not the
whole solution. Where practical, encode a recurring lesson in a typed input,
reusable function, explicit result, validation rule or behavioral regression test.

## What exists today

Core source is **0.3.0**. The Proxmox collection is **0.2.1**; its native
inspector reports **0.4.0** and uses a separately pinned archive. The catalog's
operation sidecars require core 0.2.0; existing recipe schemas remain compatible.
See the [current CLI](cli.md) and [execution contract](execution.md).

| Capability | Current status | Remaining extraction work |
| --- | --- | --- |
| Provider inventory and local JSON drift | Implemented, independent of Ansible | Independently qualify API visibility in private workflows |
| Recipe list, inspect and static plan | Implemented | Executable requirements and postconditions |
| Operation discovery | Intent search, typed private target reference, supported entrypoint, known failures/recovery and bound static planning | Broader procedure inputs and executable requirements |
| Native inspections | Seven typed Proxmox actions, pinned code, live memory phases and diagnostic receipts | Broaden qualified profiles without weakening their identity/source contracts |
| Ansible check/apply runner | Implemented, execution denied by default | Broader procedure qualification and outcome evidence |
| MCP | Eight tools including procedure describe/plan/historical-status; no apply tool; planning effects declared | Catalog composition and private workflow qualification |
| Proxmox operations | API provider, eleven roles, shared guard, native target/comparison/controller/SSH/TLS inspections and direct-use Terraform LXC module | Qualify real workflows before replacing private wrappers |
| Catalog | Four ready entries: host baseline, backup schedule, host maintenance and LXC growth | Ready means local contract checks, not universal infrastructure qualification |
| Persistent LXC/ZFS | Draft with no executable playbook | Agree the convention, then qualify retained-data rebuild and independent restore |
| Result/log handling | Memory procedure binds intent, native/controlled facts, independent postchecks and once-only consumption; Plan1 remains command-only | Real workflow and recovery qualification |
| Native Terraform execution in CmdMesh | Not implemented; direct Terraform is documented | Consider a driver only when an audited procedure needs it and its contract is reviewed |

## The extraction method

### 1. Audit the actual source, including failures

Create a **private source ledger** covering all in-scope DevOps/Neo history,
starting with Proxmox-related work. Include available active and archived chats,
scripts, runbooks, tests, issue/PR records and incident evidence.

For every source, record identity/date, availability, reviewed topic ranges,
the operation attempted, repeated discovery, failed approaches, user corrections,
the eventual working procedure and its implementation/evidence references.
Deduplicate copied summaries. Distinguish a historical assertion from a result
that still holds against current code.

Missing or inaccessible transcripts remain visible gaps. Do not describe a
partial local corpus as every DevOps chat reviewed. Obtaining missing archives
or explicitly agreeing to narrower coverage is part of P1, not an assumption.
Raw chats and evidence are not public training data or runtime dependencies.

### 2. Classify before converting

For each candidate, decide whether it is:

- A general capability, an integration-specific adapter, a reusable recipe,
  an agent knowledge entry, or private policy/configuration.
- Already extracted, partially extracted, private-only, superseded or unverified.
- A technical invariant, an environment preference, a release-specific condition,
  or a previously rejected safeguard.

Do not turn every warning from a chat into a universal blocking rule. Procedures
must report technical facts and apply the owner's declared policy, including
authorized exceptions. Preserve relevant corrections and superseded decisions.

### 3. Extract a small, verifiable unit

Replace built-in context with explicit inputs and references. Resolve paths
against a declared workspace; select targets through inventory; obtain access
through configured mechanisms. Keep environment values out of library defaults.

Reuse existing supported tools. First extract a pure calculation or read-only
inspection, then a check path, then a separately authorized bounded change.
Reproduce the relevant historical failure and show how the component detects,
explains or prevents it. Do not claim a generic sandbox around trusted tooling.

### 4. Teach discovery once

A planned operation entry should expose the following information. This table
is a design target, not a list of fields accepted by the current manifest schema.

| Information | Why an agent needs it |
| --- | --- |
| Stable task name and aliases | Find a disk-growth procedure using ordinary task vocabulary |
| Supported capability/tool versions | Avoid reconstructing incompatible commands |
| Typed inputs and named target requirements | Supply actual parameters rather than guess private context |
| Prerequisites and ownership | Know what must be true and which tool may change each field |
| Inspection, plan and execution entrypoints | Use installed implementations instead of inventing shell sequences |
| Expected results and verification | Distinguish successful execution from the intended outcome |
| Known failures and next checks | Diagnose TLS, access, capacity or version problems without repeating old experiments |
| Recovery and uncertainty behavior | Stop and inspect when outcome is unknown; choose a compatible recovery procedure |
| Provenance and qualification scope | Know which version and evidence support the procedure |

### 5. Adopt back into private workflows

Keep existing operator entrypoints where useful, but make them thin adapters to
the qualified public component. Keep only environment values and private policy
in the private repository. Migrate and qualify one call path at a time, then
remove the corresponding duplicated implementation. Preserve rollback access.

Every later incident feeds this loop: update the component, its known-error
entry and its behavioral test when generalizable, rather than adding another
piece of chat-only memory.

## Proxmox capability families to audit first

These are grounded extraction targets. They are not all shipped CmdMesh features
and their detailed priority must be confirmed in the private candidate ledger.

| Family | Repeated work to make explicit | Planned reusable result |
| --- | --- | --- |
| Target identity and discovery | Node/guest lookup, duplicate guest IDs across nodes, lifecycle ownership and incomplete visibility | Qualified target identity and inspectable current-state facts |
| Access and trust | SSH identity, API reachability, CA/SAN validation, address changes and management certificates | Access diagnosis and a separately qualified repair procedure |
| Controller/tool context | Ansible versions/config, temporary paths, SSH transport limits and private output | Reusable controller preparation and predictable invocation/result handling |
| Baseline and capacity | Host package policy, guest RAM/disk growth, capacity checks and owner selection | Explicit check/change operations with actual postconditions |
| Guest/data lifecycle | Guest creation/rebuild, persistent mounts, UID/GID mapping and identity | A qualified guest/persistence procedure without hidden data destruction |
| Backup and recovery | Backup coverage, independent copies, scratch restores and data validation | A repeatable verification/restore drill with honest recovery evidence |
| Maintenance and availability | Running guests, maintenance scope, reboot effects, service convergence and permitted downtime | A procedure driven by actual state and declared availability policy |
| Delivery into guests | Artifact identity, runtime preparation, health/log checks and compatible rollback | A first reusable guest-deployment procedure; wider strategies follow P7 |

## Gradual delivery plan

P0 is the existing foundation. P1–P6 define the first complete **Proxmox knowledge
extraction and reuse cycle**. P7 is an extension candidate after that cycle; it
does not make the first finish line depend on a general cloud platform.

| Phase | Status | Main owner | Depends on | Observable result |
| --- | --- | --- | --- | --- |
| P0: foundation | Available with current limits | Core, integration, catalog | None | Inventory, comparison, plans and Ansible execution exist |
| P1: history/script audit | Initial selected-source audit complete; source-quality limits retained privately | Maintainers with private-source access | P0 | A traceable source/candidate ledger explains recurring tasks, failures and coverage gaps |
| P2: component and discovery contracts | First versioned recipe-plan contract implemented | Core and integration maintainers | P1 | A stable operation entry describes how to perform a known task without reading its old chat |
| P3: extract Proxmox primitives | First selected batch implemented and qualified in synthetic/CI tests | Proxmox/core maintainers | P2 | Five standalone actions and shared CLI/MCP transport run with explicit inputs and tested failure behavior |
| P4: compose executable procedures | Memory core implemented; catalog composition/qualification in progress | Core, integration, catalog | P3 | Preflight, plan, guarded mutation, independent verification and recovery evidence |
| P5: qualify complete Proxmox workflows | Planned | Maintainers and environment owners | P4 | Representative baseline, capacity, maintenance and recovery procedures work end to end |
| P6: agent evaluation and private adoption | Planned | Core/catalog maintainers and consumers | P5 | A fresh agent and a second private installation reuse the procedures without historical context |
| P7: deployment-strategy extraction | Candidate extension | Core/integration maintainers and deployment owners | P6 and explicit scope selection | One real rollout pattern is generalized and qualified before adding more backends |

### P1: audit and rank what we actually learned

Deliver the private source ledger, a component inventory and an extraction map.
Link chat/incident findings to the actual script or runbook and the current
implementation. Record failed attempts, accepted operator policy and fixes that
were later rejected or superseded. Separate Proxmox work from later deployment
patterns without discarding their relationships.

Rank candidates by repeated rediscovery, failure impact, reuse in another
environment, the amount of hidden context, and available validation. For every
high-priority candidate, record its proposed layer, private inputs, existing code,
evidence and a small implementation slice.

Exit: all accessible in-scope sources are indexed and the selected task regions
are reviewed. Missing sources remain tracked; comprehensive coverage requires
obtaining them or explicit owner acceptance of a narrower scope. The prioritized
ledger is auditable, not a feature list invented from memory.

### P2: define the reusable component and knowledge contract

Define stable task identities, aliases, typed parameters, environment references,
supported versions, prerequisites, expected outputs, known failures and recovery.
Design CLI discovery first and thin MCP access to the same implemented functions.
An agent should find the task by intent and learn its exact supported entrypoint.

Include package compatibility, source pins, a starter private layout, installation
instructions and diagnostics needed to reproduce the first component outside the
original environment. Keep credential resolution with existing secret tooling.

Exit: a new operator can inspect one operation and supply valid private inputs
without reading the originating conversation. Unsupported or missing inputs have
actionable errors. Schema/compatibility changes are reviewed; current manifests
must not silently accept new fields or broaden old approvals.

### P3: extract high-value Proxmox primitives

The first selected batch is delivered: exact target resolution, strict observation
comparison, controller diagnosis, explicit SSH identity and verified TLS access.
The standalone archive contains shared inventory/probe code. Core freezes its
mandatory pinned bytes before invocation and preserves structured negative
outcomes. CLI and MCP use the same transport. See [native inspections](inspections.md)
for qualified source pins, schemas, examples and runtime limits.

Tests reproduce duplicate-identity false success, invalid CA fallback, requested
wildcard certificate identity, implicit SSH certificate discovery and detached
probe children surviving cancellation. Real infrastructure qualification and
private adoption remain P5/P6 work. No mutation postcondition is implied by these
read-only results.

Choose the first batch from P1's ranked ledger. Reuse the existing inventory
provider and roles, then extract missing target/access/controller/capacity helpers
as small independently testable components. Parameterize private addresses,
paths, credentials, ownership and availability policy.

Exit: each primitive works without a private parent checkout, has an explicit
input/result contract and reproduces the relevant historical error in a local
or disposable test. Wrong-target, unsupported-version, incomplete-data and
unknown-outcome cases do not become misleading success. No live state is inferred
solely from old chat notes.

### P4: compose procedures with executable checks

Extend the recipe/result contract where the audited tasks need typed requirements,
executable preflight/postconditions and per-step evidence. Keep ordinary supported
tools as the implementation. Initially Ansible remains the execution driver;
other native drivers require a separate justified contract.

Differentiate checked, applied, verified, failed and unknown outcomes as proposed
concepts, not current API status strings. A failed postcondition is not verified
even if the command exited zero. Preserve recovery guidance and stop dependent
steps after failure; do not blindly retry uncertain mutations.

Exit: the first known procedure is discoverable, runnable with private parameters,
and verifies its actual result. A changed bound input invalidates review. New
schemas and old-plan migration/recreation behavior are tested and documented.

### P5: prove the Proxmox procedures end to end

Qualify the selected toolbox on disposable targets before private adoption. The
reference suite covers target/access inspection, host baseline, explicitly owned
LXC growth, guest-aware maintenance, and backup/rebuild/restore behavior.

For persistent data, first agree the dataset/bind-mount/UID-GID/application-identity
convention. Demonstrate both rootfs replacement with retained data and an isolated
restore after making the original disposable data unavailable. Verify meaningful
application data and permissions. A same-storage snapshot or backup timer alone
does not establish independent recovery.

Exit: successful and failing cases have observable evidence, supported versions
and scope are published, and each procedure has recovery guidance. A missing
mount does not initialize an empty application silently. Production destruction
is not a qualification technique; real trials require their owner's authorization.

### P6: prove the agent and reuse benefit

Use fresh operator/agent sessions without the original conversation. Give them
only the installed tool/catalog, ordinary documentation and private environment
references. Ask them to inspect/resolve a target, select a known procedure, prepare
the operation, explain a known failure and verify a result using actual tool data.

Before evaluation, maintainers and environment owners agree on the task set,
permissions and expected outcomes. Include supported valid success cases and
invalid/unsupported refusal cases. Supported cases must complete their declared
workflow and verify the expected result; excessive or false refusals fail the
evaluation. A planning-only MCP task is assessed as planning, not unauthorized apply.

Compare representative tasks with the previous private-script workflow. Record
incorrect target/tool selection, missing-prerequisite detection, unsupported
invented commands and repeated requests for historical instructions. Publish only
measured outcomes and their test conditions; do not promise zero agent failures.

Adopt one Neo/private call path at a time: read first, check next, then separately
authorized bounded changes. Replace duplicate private implementation only after
the shared component is qualified. Prove reuse in a second independent private
installation with the same public packages and different private values.

Exit: the agreed supported Proxmox cases complete and verify, and the agreed
invalid cases are correctly refused, without reconstructing their old chats.
Known failures have reusable diagnostics
and regression evidence. Existing private state/history and a rollback path are
preserved. MCP remains inspection/planning/status; a client with terminal access
retains its own authority and is not sandboxed by CmdMesh.

### P7: extend the method to deployment strategies

This is the next candidate scope, not a promise to ship every deployment backend.
Audit actual deployment history with the same source-to-component method. Select
one repeated strategy before designing a broad execution platform.

Potential reusable pieces include immutable artifact qualification, target/runtime
inspection, rolling versus blue/green versus stop-first selection, compatibility
reporting, staged handoff, convergence-aware health/log observation and rollback
to an identified prior artifact. Start with a guest/service deployment where the
Proxmox components already help; add other runtimes only when justified.

Keep application migrations, cache/session compatibility, release-specific
exceptions and availability requirements in explicit adapters/policy. Report
facts and honor authorized choices rather than forcing every incident's warning
into a universal pause or extra approval loop.

A Terraform execution driver is a candidate only if selected procedures require
it. It must apply the exact reviewed saved plan, preserve private state/artifacts,
honor supported native locking and reconcile uncertain outcomes. Saved plans can
contain sensitive data and locking depends on the backend; consult the native
[plan](https://developer.hashicorp.com/terraform/cli/commands/plan) and
[locking](https://developer.hashicorp.com/terraform/language/state/locking) contracts.
Direct Terraform remains separate until such a driver is implemented and qualified.

Exit: one strategy works through the same typed capability/recipe/evidence model
in a disposable reference environment and an authorized private trial. Operator
policy, runtime adapters and generic mechanics have distinct owners. No new
driver or cloud support is claimed solely because it appears in this plan.

## Ownership, decisions and limits

| Location | Owns |
| --- | --- |
| `cmdmesh` | Generic contracts, discovery, inventory/comparison, plans, qualified execution/result handling, CLI and thin MCP |
| `cmdmesh-proxmox` | Proxmox API, reusable platform roles/adapters, module implementations and platform-specific inspections |
| `cmdmesh-recipes` | Procedures combining capabilities, requirements, verification, known failures and recovery |
| Private environment | Source ledger, raw chats/incidents, real topology, secret references, policy, state, approvals, backups and evidence |

Keep the three public packages and private environments as the initial structure.
Publishing loose private scripts alone leaves context/discovery problems intact.
A mandatory hosted control plane adds identity, scheduling and operating cost
before reuse is proven. Extend the existing CLI-first shared core instead.

Resolve these decisions in the relevant implementation phase: the operation
metadata/schema and versioning contract in P2; the ranked first extraction batch
in P1/P3; the persistence/restore convention and supported matrix in P5; evidence
retention and uncertainty reconciliation in P4/P6; and a single deployment pattern
plus any needed driver contract before P7 implementation.

The initial finish line is P1–P6 for the selected Proxmox toolbox, including its
documented source-coverage limits. Wider deployments follow P7. Additional cloud
providers, applications, remote execution services, signed multi-user approval,
scheduling and automatic remediation require their own demonstrated need and
design. They are not implied by the current inventory or MCP tool surface.

Publishing this plan does not implement its future contracts or authorize a live
change. Every new capability follows local/synthetic validation, disposable-target
qualification and a bounded authorized private adoption before it replaces code.
