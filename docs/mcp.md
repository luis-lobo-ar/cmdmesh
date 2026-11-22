# MCP guide

CmdMesh's optional MCP server shares the CLI's inventory, comparison, validation,
and static planning functions. It uses stdio, not HTTP, SSE, or a listening port.
There is no MCP execution tool, even when CLI execution is enabled in configuration.

## Configure a client

Build and review the executable first. Replace the example paths with absolute
paths to your binary and private environment. Adapt this generic shape to your
client's configuration format:

```json
{
  "mcpServers": {
    "cmdmesh": {
      "command": "/opt/cmdmesh/bin/cmdmesh",
      "args": ["--config", "/etc/cmdmesh/environment.json", "mcp"]
    }
  }
}
```

The equivalent process command is:

```bash
/opt/cmdmesh/bin/cmdmesh --config /etc/cmdmesh/environment.json mcp
```

Have the client connect to that process's stdin/stdout. Running the command in
a terminal alone does not provide a chat interface. Do not put shell banners,
debug messages, or a proxy's text on stdout; it carries protocol messages only.
The implementation uses newline-delimited JSON-RPC and the 2025-11-25 protocol.

Pass required secret values through the launch environment and the configured
provider allowlist. The core does not load `.env`, decrypt secrets, or accept API
token values as tool arguments. Keep credentials out of chat and public client
configuration. An optional client `env` field is client-specific, not CmdMesh syntax.

## Tools and exact arguments

| Tool | Argument object | Needs server environment |
| --- | --- | --- |
| `cmdmesh_inventory_pull` | `{"provider":"proxmox"}` | Yes |
| `cmdmesh_drift_detect` | `{"desired":"/private/desired.json","inventory":"/private/inventory.json"}` | No |
| `cmdmesh_recipe_list` | `{"catalog":"/opt/cmdmesh-recipes"}` | No |
| `cmdmesh_recipe_inspect` | `{"manifest":"/opt/cmdmesh-recipes/recipes/proxmox-host-baseline/recipe.json"}` | No |
| `cmdmesh_recipe_plan` | `{"manifest":"/opt/cmdmesh-recipes/recipes/proxmox-host-baseline/recipe.json","target":"lab","operation":"check"}` | Yes |
| `cmdmesh_operation` | `{"action":"list","catalog":"/opt/cmdmesh-recipes","query":"host tools","offset":0,"limit":20}` | List/inspect: no; validation/planning: yes |

Use absolute paths for file arguments; the client controls the server's working
directory. Unknown argument fields are rejected. Tool results encode JSON in
MCP text content. Inventory and drift have the same completeness semantics as
the CLI; a successful tool call is not evidence of complete or healthy infrastructure.

`cmdmesh_recipe_plan` also accepts `operation:"apply"`. This only prepares data;
it does not execute the apply. There is no `execute`, `approve`, or `out` argument
on this MCP tool. It returns a plan rather than writing a plan file. The CLI's
`recipe plan --out` is the usual way to create the private file for execution.

`cmdmesh_operation` groups four local actions over the same implementation as
the CLI. `list` accepts only `catalog`, optional `query`, `offset` and `limit`.
`inspect` takes `catalog` and `operation` (canonical ID or alias).
`validate-inputs` and `plan` additionally require `inputs`, a private JSON file
path. They do not accept query/pagination fields. Validation without an environment
returns `configuration-required`; planning requires the server environment.
The tool never echoes private input values or runs prerequisite checks. Planning
returns an existing schema-1 recipe plan, not a file or completed execution.
There is no `execute`, `approve` or output-path argument. See [operation discovery](operations.md).

## Use CLI and MCP together

Use MCP for interactive discovery and inspection. Use the CLI for repeatable
scripts, private output files, and explicitly approved execution. Returned
commands are not instructions for the server to run automatically.

Enabling `execution.allow_execution` or `allow_mutations` does not add an MCP
tool. A client with a separate terminal tool has its own authority boundary;
CmdMesh's read-only MCP is not a sandbox for that client's other capabilities.

The server can read paths available to its OS process and invokes configured
trusted providers. Read-only annotations are not per-client authentication,
filesystem isolation, or a guarantee that untrusted integration code has no
effects. Trust the client, controller, packages, and private configuration.

## Troubleshooting

- For `server --config required`, launch with the intended environment before
  calling inventory or planning. Drift and discovery do not require one.
- For a missing relative file, use its absolute path rather than assuming the
  client's current directory matches your terminal.
- For missing credentials, check the launch environment and named allowlist;
  do not paste the token into a tool call.
- For malformed protocol output, launch the binary directly without a shell
  wrapper that writes status text to stdout.

See [the CLI reference](cli.md) and [operational troubleshooting](troubleshooting.md).
