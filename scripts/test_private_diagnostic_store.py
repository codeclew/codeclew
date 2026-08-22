#!/usr/bin/env python3

from __future__ import annotations

import hashlib
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import private_diagnostic_store as store
from private_diagnostic_store import DiagnosticStoreError, store_diagnostic


class PrivateDiagnosticStoreTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name).resolve()
        self.control = self.root / "control"
        self.control.mkdir(mode=0o700)
        os.chmod(self.control, 0o700)
        self.source = self.root / "source.stderr"
        self.source.write_bytes(b"private failure\n")
        os.chmod(self.source, 0o600)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_atomic_immutable_store_and_collision_reuse(self) -> None:
        digest = store_diagnostic(self.source, self.control)
        self.assertEqual(digest, "sha256:" + hashlib.sha256(self.source.read_bytes()).hexdigest())
        target = self.control / "diagnostics" / "cold-runtime" / f"{digest[7:]}.stderr"
        self.assertEqual(target.read_bytes(), self.source.read_bytes())
        self.assertEqual(target.stat().st_mode & 0o777, 0o400)
        self.assertEqual(store_diagnostic(self.source, self.control), digest)

    def test_source_symlink_and_unsafe_mode_are_rejected(self) -> None:
        link = self.root / "source-link"
        link.symlink_to(self.source)
        with self.assertRaises(DiagnosticStoreError):
            store_diagnostic(link, self.control)
        os.chmod(self.source, 0o644)
        with self.assertRaises(DiagnosticStoreError):
            store_diagnostic(self.source, self.control)

    def test_oversized_source_is_rejected(self) -> None:
        self.source.write_bytes(b"x" * (1024 * 1024 + 1))
        with self.assertRaises(DiagnosticStoreError):
            store_diagnostic(self.source, self.control)

    def test_path_swap_after_open_cannot_change_descriptor_content(self) -> None:
        original_read = store._read_descriptor
        swapped = False

        def swap_then_read(descriptor, limit):
            nonlocal swapped
            if not swapped:
                swapped = True
                replacement = self.root / "replacement.stderr"
                replacement.write_bytes(b"replacement\n")
                os.chmod(replacement, 0o600)
                os.replace(replacement, self.source)
            return original_read(descriptor, limit)

        with (
            mock.patch.object(store, "_read_descriptor", side_effect=swap_then_read),
            self.assertRaises(DiagnosticStoreError),
        ):
            store_diagnostic(self.source, self.control)

    def test_symlinked_control_home_is_rejected(self) -> None:
        alias = self.root / "alias"
        alias.symlink_to(self.control)
        with self.assertRaises((DiagnosticStoreError, OSError)):
            store_diagnostic(self.source, alias)

    def test_existing_wrong_cas_object_is_rejected(self) -> None:
        digest = "sha256:" + hashlib.sha256(self.source.read_bytes()).hexdigest()
        directory = self.control / "diagnostics" / "cold-runtime"
        directory.mkdir(mode=0o700, parents=True)
        target = directory / f"{digest[7:]}.stderr"
        target.write_bytes(b"wrong")
        os.chmod(target, 0o400)
        with self.assertRaises(DiagnosticStoreError):
            store_diagnostic(self.source, self.control)

    def test_each_created_directory_entry_fsyncs_its_parent(self) -> None:
        flags = os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_NOFOLLOW", 0)
        parent = os.open(self.control, flags)
        try:
            with mock.patch.object(os, "fsync", wraps=os.fsync) as fsync:
                diagnostics = store._open_private_child(parent, "diagnostics")
                try:
                    cold = store._open_private_child(diagnostics, "cold-runtime")
                    os.close(cold)
                finally:
                    os.close(diagnostics)
            synchronized = [call.args[0] for call in fsync.call_args_list]
            self.assertIn(parent, synchronized)
            self.assertEqual(len(synchronized), 2)
            self.assertNotEqual(synchronized[0], synchronized[1])
        finally:
            os.close(parent)


if __name__ == "__main__":
    unittest.main()
