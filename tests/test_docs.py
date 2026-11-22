"""Run the public quick starts without network inventory or real hosts."""

import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(os.environ.get("CMDMESH_BIN", ROOT / "target/release/cmdmesh")).resolve()


def cli(*args):
    return subprocess.run(
        [str(BINARY), *map(str, args)], capture_output=True, text=True, timeout=45
    )


def headings(document):
    anchors = set()
    for heading in re.findall(r"^#{1,6} (.+)$", document, re.MULTILINE):
        slug = re.sub(r"[^\w\- ]", "", heading.lower()).replace(" ", "-")
        suffix = 0
        anchor = slug
        while anchor in anchors:
            suffix += 1
            anchor = f"{slug}-{suffix}"
        anchors.add(anchor)
    return anchors


class DocumentationExamples(unittest.TestCase):
    def test_documented_inventory_gates_stop_when_the_pull_fails(self):
        for document in (ROOT / "README.md", ROOT / "docs/proxmox.md"):
            with self.subTest(document=document.name):
                snippet = re.search(
                    r"(cmdmesh --config [^\n]+ inventory pull \\\n[^\n]+\n"
                    r"jq -e [^\n]+)",
                    document.read_text(),
                )
                self.assertIsNotNone(snippet)
                result = subprocess.run(
                    [
                        "bash",
                        "-c",
                        "cmdmesh() { return 17; }\n"
                        "jq() { echo STALE_SNAPSHOT_ACCEPTED; return 0; }\n"
                        + snippet.group(1),
                    ],
                    capture_output=True,
                    text=True,
                    timeout=5,
                )
                self.assertEqual(result.returncode, 17)
                self.assertEqual(result.stdout, "")

    def test_reference_flags_match_real_command_help(self):
        routes = {
            ("inventory", "pull"): ("--provider", "--out"),
            ("drift", "detect"): ("--desired", "--inventory"),
            ("recipe", "list"): ("--catalog",),
            ("recipe", "inspect"): ("MANIFEST",),
            ("recipe", "plan"): ("--target", "--operation", "--out"),
            ("recipe", "execute"): ("--plan", "--approve-digest", "--log-dir"),
            ("operation", "list"): ("--catalog", "--query", "--offset", "--limit"),
            ("operation", "inspect"): ("--catalog", "OPERATION"),
            ("operation", "validate-inputs"): ("--inputs", "--catalog"),
            ("operation", "plan"): ("--inputs", "--catalog", "--out"),
            ("mcp",): ("--config",),
        }
        for route, flags in routes.items():
            with self.subTest(route=route):
                result = cli(*route, "--help")
                self.assertEqual(result.returncode, 0, result.stderr)
                for flag in flags:
                    self.assertIn(flag, result.stdout)

    def test_offline_snapshots_demonstrate_clean_changed_and_incomplete(self):
        for name, clean, complete in (
            ("inventory.json", True, True),
            ("inventory-drift.json", False, True),
            ("inventory-incomplete.json", False, False),
        ):
            with self.subTest(name=name):
                result = cli(
                    "drift",
                    "detect",
                    "--desired",
                    ROOT / "examples/desired.json",
                    "--inventory",
                    ROOT / "examples" / name,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                report = json.loads(result.stdout)
                self.assertEqual(report["clean"], clean)
                self.assertEqual(report["complete"], complete)
                if name == "inventory-drift.json":
                    self.assertEqual(report["items"][0]["fields"], ["maxmem"])

    def test_shared_protocol_fixture_matches_the_reader_example(self):
        self.assertEqual(
            json.loads((ROOT / "examples/inventory.json").read_text()),
            json.loads((ROOT / "tests/fixtures/inventory.json").read_text()),
        )

    def test_local_catalog_is_inspectable_without_environment_or_ansible(self):
        result = cli("recipe", "list", "--catalog", ROOT / "examples/local")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)[0]["id"], "local-check")
        result = cli(
            "recipe", "inspect", ROOT / "examples/local/recipes/local-check/recipe.json"
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["dependencies"], [])

    def test_intent_discovery_prepares_a_private_static_plan(self):
        catalog = ROOT / "examples/local"
        found = cli("operation", "list", "--catalog", catalog, "--query", "local tools")
        self.assertEqual(found.returncode, 0, found.stderr)
        self.assertEqual(json.loads(found.stdout)["items"][0]["id"], "local-check")
        inspected = cli("operation", "inspect", "check local tools", "--catalog", catalog)
        self.assertEqual(inspected.returncode, 0, inspected.stderr)
        entry = json.loads(inspected.stdout)
        self.assertEqual(entry["parameters"]["target"]["type"], "target")
        self.assertEqual(entry["entrypoint"]["operation"], "check")
        generated = ROOT / ".generated"
        generated.mkdir(mode=0o700, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="operations-docs-", dir=generated) as directory:
            private = Path(directory)
            (private / "inventory.yml").write_text("demo ansible_connection=local\n")
            config = private / "environment.json"
            config.write_text(json.dumps({"schema_version": 1, "targets": {"local": {
                "inventory": "inventory.yml", "limit": "demo", "lifecycle_owner": "ansible"}}}))
            inputs = catalog / "operation-inputs.json"
            validated = cli("--config", config, "operation", "validate-inputs", "local-check",
                            "--catalog", catalog, "--inputs", inputs)
            self.assertEqual(validated.returncode, 0, validated.stderr)
            self.assertFalse(json.loads(validated.stdout)["requirements_evaluated"])
            plan = private / "check.plan.json"
            prepared = cli("--config", config, "operation", "plan", "local-check",
                           "--catalog", catalog, "--inputs", inputs, "--out", plan)
            self.assertEqual(prepared.returncode, 0, prepared.stderr)
            self.assertEqual(plan.stat().st_mode & 0o777, 0o600)
            value = json.loads(plan.read_text())
            self.assertEqual(value["operation"], "check")
            self.assertIn("--check", value["commands"][0]["argv"])
            self.assertEqual(value["target"], "local")
            self.assertFalse((private / "execution.json").exists())

    def test_local_check_has_private_output_and_cannot_apply(self):
        generated = ROOT / ".generated"
        generated.mkdir(mode=0o700, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="docs-", dir=generated) as directory:
            directory = Path(directory)
            for name in ("environment.json", "inventory.yml"):
                shutil.copyfile(ROOT / "examples/local" / name, directory / name)
                (directory / name).chmod(0o600)
            manifest = ROOT / "examples/local/recipes/local-check/recipe.json"
            prepared = cli(
                "--config",
                directory / "environment.json",
                "recipe",
                "plan",
                manifest,
                "--target",
                "local",
                "--operation",
                "check",
                "--out",
                directory / "check-plan.json",
            )
            self.assertEqual(prepared.returncode, 0, prepared.stderr)
            digest = json.loads(prepared.stdout)["digest"]
            logs = directory / "run-01"
            result = cli(
                "recipe",
                "execute",
                "--plan",
                directory / "check-plan.json",
                "--approve-digest",
                digest,
                "--log-dir",
                logs,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)["status"], "succeeded")
            self.assertNotIn("local check completed", result.stdout)
            preview = (logs / "step-0001.stdout").read_text()
            self.assertIn("CmdMesh local check completed", preview)
            self.assertIn("changed=0", preview)
            self.assertEqual(logs.stat().st_mode & 0o777, 0o700)
            for path in logs.iterdir():
                self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            prepared = cli(
                "--config",
                directory / "environment.json",
                "recipe",
                "plan",
                manifest,
                "--target",
                "local",
                "--operation",
                "apply",
                "--out",
                directory / "apply-plan.json",
            )
            self.assertEqual(prepared.returncode, 0, prepared.stderr)
            result = cli(
                "recipe",
                "execute",
                "--plan",
                directory / "apply-plan.json",
                "--approve-digest",
                json.loads(prepared.stdout)["digest"],
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("mutations disabled", result.stderr)

    def test_local_markdown_links_and_anchors_exist(self):
        documents = [ROOT / "README.md", *sorted((ROOT / "docs").glob("*.md"))]
        documents.extend((ROOT / "examples/local").rglob("*.md"))
        for document in documents:
            for link in re.findall(
                r"(?<!!)\[[^\]]+\]\(([^\s)]+)\)", document.read_text()
            ):
                target = urlsplit(link)
                if target.scheme or target.netloc:
                    continue
                path = (
                    (document.parent / unquote(target.path)).resolve()
                    if target.path
                    else document
                )
                with self.subTest(document=document.name, link=link):
                    self.assertTrue(path.is_relative_to(ROOT))
                    self.assertTrue(path.exists(), f"Broken link: {link}")
                    if target.fragment and path.suffix == ".md":
                        self.assertIn(
                            unquote(target.fragment), headings(path.read_text())
                        )


if __name__ == "__main__":
    unittest.main()
