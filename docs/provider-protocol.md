# Provider protocol version 1

The core launches an explicitly configured executable with no command arguments.
It writes one JSON request to stdin and closes stdin. The executable returns
one JSON response on stdout. Diagnostics belong on stderr and must not contain
credentials or API response bodies.

```json
{"schema_version":1,"operation":"inventory","config":{"endpoint":"https://hypervisor.example.invalid:8006"}}
```

```json
{"schema_version":1,"complete":true,"resources":[{"kind":"proxmox.node","id":"node-a","properties":{"status":"online"}}],"warnings":[]}
```

An executable returns exit code 0 when it emits a valid response, including a
partial response with `complete: false`. A nonzero exit means a process or
protocol failure. The core withholds child output on such failures.

Resource identities are `(kind, id)` pairs. Both strings must be nonempty.
`properties` is an object. Duplicate identities and unsupported schema versions
are rejected. Proxmox uses `proxmox.node`, `proxmox.vm`, and `proxmox.lxc`.

`complete: false` means some requested scope could not be read. Partial inventory
must not imply the missing resources were deleted. Drift results remain not
clean even if no individual resource difference is found.

The core limits each child output stream to one MiB and inventory to 100,000
resources. It terminates the process group on timeout, cancellation, or excess
output. Executable manifests are not automatically downloaded or installed.

The synthetic [native JSON fixture](../tests/fixtures/inventory.json) is shared
with CLI and protocol tests. It is not a captured infrastructure snapshot.
