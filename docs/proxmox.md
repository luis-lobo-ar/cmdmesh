# Use CmdMesh with Proxmox

This guide connects the generic core to the separate Proxmox integration and
recipe catalog. It uses fictional addresses. Replace them in a private
environment; do not run the public examples against an unreviewed host.

## Prepare the packages

From the core repository root, clone new sibling directories and pin the known
0.1.0 companion source revisions. Do not overwrite an existing checkout:

```bash
git clone https://github.com/luis-lobo-ar/cmdmesh-proxmox.git ../cmdmesh-proxmox &&
git -C ../cmdmesh-proxmox checkout --detach 7008595dbbabb691750fcd699dd3c71fb65ae793
git clone https://github.com/luis-lobo-ar/cmdmesh-recipes.git ../cmdmesh-recipes &&
git -C ../cmdmesh-recipes checkout --detach 2ac36649ba54dc2f366be18565d6c7313978a7ce
```

Review source changes before using other revisions. CmdMesh does not install
packages, upgrade these checkouts, or fetch Ansible collections while planning
or executing. The host-baseline example uses built-in Ansible modules. Other
roles may need additional collections; follow the integration's requirements.

Use Python 3.10+ for the stdlib-only API executable. Recipe execution additionally
needs a trusted Ansible controller. The collection declares Core >=2.19.0;
the [README local setup](../README.md#run-a-local-recipe) installs qualified
Core 2.20.4, which requires controller Python >=3.12.

## Keep private inputs separate

The full template expects this layout:

```text
workspace/
  cmdmesh/
  cmdmesh-proxmox/
  cmdmesh-recipes/
  private/
    environment.json
    ca/lab.pem
    ansible/inventory.yml
    ansible/vars.yml
```

The copied inventory is YAML, so keep its `.yml` extension for Ansible's inventory
plugin. These commands are for a new private directory. Stop if it already
contains your configuration; do not overwrite it. From `cmdmesh/`:

```bash
umask 077
mkdir -m 700 ../private &&
mkdir ../private/ca ../private/ansible &&
cp -n examples/environment.json ../private/environment.json &&
cp -n examples/proxmox/inventory.yml ../private/ansible/inventory.yml &&
cp -n examples/proxmox/vars.yml ../private/ansible/vars.yml
```

Stop if these paths already hold your configuration. Review and edit the copies.
Use a private repository for real inventory and policy, with a separate secret
manager and private ignored plan/log locations. No public repository needs your
private environment or credentials.

## Configure read-only API inventory

For API inventory alone, copy [the provider-only template](../examples/proxmox/inventory-only.json)
to `../private/environment.json` instead of the full template. It needs Python
3.10+, API credentials, and TLS trust; it needs no Ansible controller, SSH key,
target inventory, or recipe catalog. Its relative paths use the same sibling
layout. Keep the full template when you also want recipes.

In `environment.json`, review the configured provider executable and set:

- `endpoint` to the real HTTPS origin, such as `https://pve.example.invalid:8006`,
  with no `/api2/json` path, credentials, query, or fragment.
- `ca_file` to a trusted PEM CA file, or omit it for system trust. Obtain private
  CA material through an independently verified channel. Do not export private keys.
- `token_id_env` and `token_secret_env` to the environment-variable names that
  supply a least-privilege read-only token. Include both names in `env_allowlist`.
- `timeout_seconds` to a per-request timeout greater than zero and at most 60.

The certificate must match the endpoint hostname or IP. Redirects, ambient HTTP
proxies, password login, and insecure TLS fallback are disabled. Do not increase
privileges or disable verification merely to make a failing check green.

The API token must see the intended nodes and guests. Independently check that
scope; valid API replies can still hide resources through ACLs. One provider
origin selects one cluster/endpoint, not every standalone host in a lab. Use
separate provider IDs and snapshots for independent clusters.

### Provision and verify the token

An administrator must approve the visibility scope before creating credentials:

1. In Datacenter → Permissions → Users, create a dedicated enabled account,
   such as `inventory@pve`, using your normal password policy.
2. Under API Tokens, add `readonly` for that account. Keep **Privilege Separation**
   enabled, set an expiry, and save the one-time secret in your secret manager.
3. Grant audit access to both the user and token. For approved cluster-wide
   visibility, `PVEAuditor` on `/` with propagation is a read-only starting point;
   it exposes more metadata than this provider needs. For narrower access,
   review `Sys.Audit` on intended `/nodes/<node>` paths and `VM.Audit` on intended
   `/vms/<VMID>` paths against your server's API permissions.
4. Inspect effective permissions with `pveum user permissions inventory@pve` and
   `pveum user token permissions inventory@pve readonly` on the Proxmox host.
   Compare both with your approved scope.

Separated tokens receive only permissions held by both account and token.
See [Proxmox token and permission documentation](https://github.com/proxmox/pve-docs/blob/master/pveum.adoc#api-tokens)
and your version's [API permission reference](https://pve.proxmox.com/pve-docs/api-viewer/).

From the core repository root, after replacing the fictional endpoint and CA:

```bash
export PROXMOX_TOKEN_ID='inventory@pve!readonly'
read -r -s -p 'Token secret: ' PROXMOX_TOKEN_SECRET
export PROXMOX_TOKEN_SECRET
cmdmesh --config ../private/environment.json inventory pull \
  --provider proxmox --out ../private/inventory-01.json &&
jq -e '.complete == true and (.warnings | length) == 0' ../private/inventory-01.json
```

Use a new snapshot filename for each pull. Keep the `&&`: when the output already
exists or the pull fails, the gate must not accept an older snapshot.

List the returned identities for a separate coverage check:

```bash
jq -r '.resources[] | [.kind, .id] | @tsv' ../private/inventory-01.json
```

Have an administrator compare these identities with the same cluster's GUI
node tree and VM/container lists, including offline nodes and stopped guests.
Record every intended node and `node/VMID` in a private expected-resource list;
investigate missing entries and exclusions before accepting coverage. Repeat
after ACL changes, guest moves, or topology changes. A matching count alone
does not prove that the expected resources are visible.

The executable uses GET only. It returns `proxmox.node` identities by node name
and VM/LXC identities as `node/VMID`. Retained node fields include status, CPU,
configured CPU/memory, current memory, and uptime. Guest fields additionally
include name, disk values, node, and VMID when present. API fields retain their
names, such as `maxmem`; the core does not rename them to Ansible variables.

Root/auth/TLS failures never become successful empty Proxmox inventory. A failed
guest collection preserves other resources but marks the snapshot incomplete.
The core protocol wrapper can still exit zero for that valid partial response.

## Configure SSH recipe access

API inventory JSON is not an Ansible inventory. Recipe execution uses the
separate `ansible/inventory.yml` file, SSH identity, and host policy. The API
token does not grant SSH access.

Replace the example host address and key, keep the host under `cmdmesh_targets`,
and verify its SSH host key through your normal trusted procedure. The named
target `lab` selects the inventory alias `node-a`; it is not an IP or provider ID.
The example pins `/usr/bin/python3` remotely and uses a managed remote module
directory under `/var/lib/vz/cmdmesh`. Check that its ownership and capacity fit
your host. The controller's scratch stays private under the environment directory.

Review `ansible/vars.yml`: the example selects only `jq` and `htop`, with no
removals. The public baseline manages host packages, not every guest. Installation
can update apt metadata and start services as normal package behavior on apply.
Check mode needs the Debian/Proxmox host's `python3-apt` package. After reviewing
SSH access, a read-only prerequisite check is
`ssh node-a "/usr/bin/python3 -c 'import apt; print(apt.__file__)'"`.
Here `node-a` must be a separately configured SSH alias; Ansible inventory aliases
are not automatically SSH aliases. Missing support needs a separately reviewed
host preparation step; it is not permission to rerun without check mode.

Set the environment's `ansible_executable` to the supported controller. The full
template points to the README's project-local Ansible environment. Enable
`allow_execution:true` before creating a check plan, but keep mutations false.
Then follow the [README host-baseline workflow](../README.md#plan-a-real-host-baseline).

## Available recipes and roles

The companion catalog currently provides:

| Recipe ID | Status | Scope |
| --- | --- | --- |
| `proxmox-host-baseline` | Ready | Explicit host package install/removal lists |
| `proxmox-backup-schedule` | Ready | Explicit guest backup jobs, modes, retention, and schedules |
| `proxmox-host-maintenance` | Ready | Assessment by default; guarded package maintenance/reboot |
| `proxmox-lxc-growth` | Ready | Ansible-owned memory/swap and rootfs growth |
| `ephemeral-lxc-zfs` | Draft | Persistent-data/recreatable-container design checklist only |

Ready means the contract and local checks passed, not that your environment,
hardware, application, or restore process was qualified. Draft recipes cannot be
planned or executed. Each recipe's README provides its actual variables and recovery.

Maintenance requires a separately approved host operation, recovery access, and
no running guests before upgrades. It does not stop or migrate guests for you.
A backup schedule is not proof of restoration. External bind-mounted datasets
need their own backup design; `vzdump` does not automatically protect them.

The integration also exports eleven operational roles plus `operation_guard`:
baseline, apt repositories, maintenance, backup jobs, NFS backup storage, LXC
resources, LXC storage, LXC DNS, LXC devices, kernel sysctls, and HPE Smart Array
inspection helpers. Use the [full collection reference](https://github.com/luis-lobo-ar/cmdmesh-proxmox/blob/7008595dbbabb691750fcd699dd3c71fb65ae793/ansible/ansible_collections/cmdmesh/proxmox/README.md)
for exact inputs, required collections, and refusal conditions.

## Choose one lifecycle owner

The optional [Terraform LXC module](https://github.com/luis-lobo-ar/cmdmesh-proxmox/tree/7008595dbbabb691750fcd699dd3c71fb65ae793/terraform/modules/lxc)
is used through ordinary Terraform, not CmdMesh execution. Keep its state and
credentials private and review its saved plan independently.

By default it owns guest existence, CPU, memory, root disk, devices, network, and
DNS. Do not let Ansible modify those same fields concurrently. Ansible can still
manage independent files/packages inside Terraform-created guests.

The growth recipe requires an Ansible lifecycle owner in the CmdMesh target and
matching explicit guest declarations. Changing an owner label does not transfer
Terraform state or prove ownership is exclusive. Review that handoff separately;
the core does not migrate state or infer guest IDs.
