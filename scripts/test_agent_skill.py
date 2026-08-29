#!/usr/bin/env python3
"""Contract tests for the portable Codeclew Agent Skill."""

from __future__ import annotations

import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
CANONICAL_SKILL = ROOT / "skills" / "codeclew"
SOURCE_LAUNCHER = ROOT / "clew"
INSTALLED_LAUNCHER = ROOT / "packaging" / "macos" / "clew"


def run_skill(*arguments: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [str(SOURCE_LAUNCHER), "skill", "install", *arguments],
        cwd=ROOT,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
        text=True,
    )


class AgentSkillTest(unittest.TestCase):
    def test_repository_discovery_copies_match_canonical_package(self) -> None:
        for root in [
            ROOT / ".agents" / "skills" / "codeclew",
            ROOT / ".claude" / "skills" / "codeclew",
        ]:
            for relative in [Path("SKILL.md"), Path("agents/openai.yaml")]:
                self.assertEqual(
                    (root / relative).read_bytes(),
                    (CANONICAL_SKILL / relative).read_bytes(),
                )

    def test_agent_contract_is_installed_release_only(self) -> None:
        skill = (CANONICAL_SKILL / "SKILL.md").read_text(encoding="utf-8")
        self.assertIn("version: \"0.2.0\"", skill)
        self.assertIn("codeclew-agent-contract/1.0", skill)
        self.assertIn("clew context open", skill)
        self.assertIn("clew doctor", skill)
        self.assertIn("attach` or", skill)
        self.assertIn("doctor task", skill)
        self.assertIn("sourceFallbackAllowed=false", skill)
        self.assertNotIn("use its supported `./clew` launcher", skill)

    def test_source_command_is_idempotent_and_requires_force_for_conflicts(self) -> None:
        with tempfile.TemporaryDirectory() as value:
            destination = Path(value) / "agent-skills"
            first = run_skill("--destination", str(destination))
            self.assertEqual(first.returncode, 0, first.stderr)
            result = json.loads(first.stdout)
            self.assertEqual(result["schema"], "codeclew-skill-install/1.0")
            self.assertEqual(result["installations"][0]["status"], "INSTALLED")
            installed = destination / "codeclew"
            self.assertEqual(
                (installed / "SKILL.md").read_bytes(),
                (CANONICAL_SKILL / "SKILL.md").read_bytes(),
            )

            current = run_skill("--destination", str(destination))
            self.assertEqual(current.returncode, 0, current.stderr)
            self.assertEqual(
                json.loads(current.stdout)["installations"][0]["status"], "CURRENT"
            )

            (installed / "SKILL.md").write_text("different\n", encoding="utf-8")
            rejected = run_skill("--destination", str(destination))
            self.assertEqual(rejected.returncode, 2)
            self.assertIn("use --force", rejected.stderr)
            self.assertEqual((installed / "SKILL.md").read_text(), "different\n")

            replaced = run_skill("--destination", str(destination), "--force")
            self.assertEqual(replaced.returncode, 0, replaced.stderr)
            self.assertEqual(
                json.loads(replaced.stdout)["installations"][0]["status"], "REPLACED"
            )
            self.assertEqual(
                (installed / "SKILL.md").read_bytes(),
                (CANONICAL_SKILL / "SKILL.md").read_bytes(),
            )

    def test_project_install_targets_codex_and_claude(self) -> None:
        with tempfile.TemporaryDirectory() as value:
            project = Path(value)
            completed = run_skill("--project", str(project))
            self.assertEqual(completed.returncode, 0, completed.stderr)
            result = json.loads(completed.stdout)
            self.assertEqual(
                [row["agent"] for row in result["installations"]],
                ["codex", "claude"],
            )
            self.assertTrue(
                (project / ".agents" / "skills" / "codeclew" / "SKILL.md").is_file()
            )
            self.assertTrue(
                (project / ".claude" / "skills" / "codeclew" / "SKILL.md").is_file()
            )

    def test_installed_launcher_dispatches_without_starting_runtime(self) -> None:
        with tempfile.TemporaryDirectory() as value:
            temporary = Path(value)
            release = temporary / "release"
            (release / "bin").mkdir(parents=True)
            launcher = release / "bin" / "clew"
            shutil.copyfile(INSTALLED_LAUNCHER, launcher)
            launcher.chmod(0o500)
            (release / "PROFILE").write_text("core\n", encoding="ascii")
            (release / "VERSION").write_text("v0.1.0\n", encoding="ascii")
            installer = release / "source" / "scripts" / "install_agent_skill.py"
            installer.parent.mkdir(parents=True)
            shutil.copyfile(ROOT / "scripts" / "install_agent_skill.py", installer)
            shutil.copytree(CANONICAL_SKILL, release / "source" / "skills" / "codeclew")
            destination = temporary / "skills"
            completed = subprocess.run(
                [str(launcher), "skill", "install", "--destination", str(destination)],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
                text=True,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertTrue((destination / "codeclew" / "SKILL.md").is_file())


if __name__ == "__main__":
    unittest.main()
