# Local check recipe

This recipe runs only Ansible's controller-side assertions and a debug message.
It gathers no facts, requests no privilege escalation, and runs no package,
network, file-management, or service tasks. The supplied inventory uses the
local connection. It has no Proxmox dependency or credentials.

Copy `environment.json` and `inventory.yml` from `examples/local` into a new
directory directly under the repository's `.generated` directory. The relative
Ansible executable then resolves to `.generated/ansible/bin/ansible-playbook`.
Follow the [README local tutorial](../../../../README.md#run-a-local-recipe).

The environment permits execution but not mutations. The recipe also refuses
non-check execution. Review the plan and use its digest to approve the check.

## Verification

The private stdout log contains `CmdMesh local check completed` and an Ansible
recap with `changed=0`. The normal CLI result is JSON, not Ansible output.

## Recovery

There is no infrastructure operation to undo. Keep or remove only the generated
demo plan and log directory according to your own retention policy. Use new
output names for another run. Do not delete unrelated private configuration.
