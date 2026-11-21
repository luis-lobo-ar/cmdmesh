# Inventory and drift

CmdMesh compares two local JSON documents. It does not query a provider during
comparison, understand Terraform state, convert units, or reconcile changes.

## Snapshot format

```json
{
  "schema_version": 1,
  "complete": true,
  "resources": [
    {
      "kind": "proxmox.lxc",
      "id": "node-a/101",
      "properties": {"name": "example-app", "maxmem": 536870912}
    }
  ],
  "warnings": []
}
```

`kind` and `id` together identify a resource. `properties` is an object of observed
values. Duplicate identities, unsupported versions, and malformed documents fail
validation. The core permits custom kinds; the Proxmox integration emits nodes,
VMs, and LXCs. See [the provider protocol](provider-protocol.md).

## Desired format

```json
{
  "schema_version": 1,
  "resources": [
    {
      "kind": "proxmox.lxc",
      "id": "node-a/101",
      "spec": {"name": "example-app", "maxmem": 536870912}
    }
  ]
}
```

`spec` is an object of fields you want to compare. Only specified fields are
checked. Object matching is recursive: extra observed keys are permitted, even
inside nested objects. Scalars and arrays use exact JSON-value equality; arrays
must match as whole values. An empty `spec` checks resource presence only.

Proxmox `maxmem` is expressed in bytes; 536870912 is 512 MiB. It is not the
Ansible growth role's MiB input. `mem`, CPU usage, and uptime are volatile readings,
so omit them unless you deliberately want those changes reported.

## Results

| Change | Meaning |
| --- | --- |
| `missing` | A desired identity is absent from the supplied snapshot |
| `changed` | At least one desired field differs; `fields` names the top-level fields |
| `unmanaged` | A snapshot identity has no desired entry |

These are observations, not creation, update, or deletion commands. An empty
desired list reports all observed resources as unmanaged. There are no built-in
ignore rules, expressions, or automatic fixes.

`complete` is copied from the snapshot. `clean` is true only when the snapshot
is complete and `items` is empty. Snapshot warnings are not copied into the drift
report, so inspect them before comparison. A provider must report completeness
honestly; the bundled Proxmox provider marks warned collections incomplete.

## Use the checked-in examples

From the repository root:

```bash
cmdmesh drift detect --desired examples/desired.json --inventory examples/inventory.json
cmdmesh drift detect --desired examples/desired.json --inventory examples/inventory-drift.json
cmdmesh drift detect --desired examples/desired.json --inventory examples/inventory-incomplete.json
```

The first is clean. The second changes `maxmem`. The third cannot be clean.
All three are valid reports and exit zero.

## Gate automation

With `jq`, inspect both flags rather than only the command's exit code:

```bash
cmdmesh drift detect --desired private/desired.json \
  --inventory private/inventory.json > private/drift-report.json &&
jq -e '.complete == true and .clean == true' private/drift-report.json
```

Before generating that report, check the inventory's `complete` flag, warnings,
and expected visibility. `complete:true` means the integration finished its
requested scope, not proof that token ACLs reveal all resources or that every
independent cluster was queried. Never infer deletion from incomplete data.

Compare independent clusters separately unless you deliberately namespace their
identities. Blindly combining two clusters with the same node/VMID creates
duplicate identities. No cluster-merging or fleet-cache command is provided.
