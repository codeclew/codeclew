#!/usr/bin/env python3
from __future__ import annotations

import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import threading
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import cold_cache_authority as cache


AUTHORITY = "sha256:" + "a" * 64
OTHER_AUTHORITY = "sha256:" + "b" * 64


class ColdCacheAuthorityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name).resolve()
        self.store = self.root / "store"
        self.store.mkdir(mode=0o700)
        self.candidate_number = 0

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def candidate(self, payload: bytes = b"crate") -> Path:
        self.candidate_number += 1
        candidate = self.store / f".candidate-{self.candidate_number:04d}"
        candidate.mkdir(mode=0o700)
        (candidate / ".cargo" / "registry").mkdir(parents=True, mode=0o700)
        (candidate / ".gradle" / "wrapper").mkdir(parents=True, mode=0o700)
        cargo = candidate / ".cargo" / "registry" / "crate"
        cargo.write_bytes(payload)
        gradle = candidate / ".gradle" / "wrapper" / "gradle"
        gradle.write_bytes(b"gradle")
        gradle.chmod(0o700)
        return candidate

    def publish(
        self, authority: str = AUTHORITY, payload: bytes = b"crate"
    ) -> tuple[Path, dict[str, object]]:
        return cache.publish_seed(self.candidate(payload), self.store, authority)

    def locator(self, authority: str = AUTHORITY) -> dict[str, object]:
        path = self.store / "locators" / f"{authority.removeprefix('sha256:')}.json"
        return json.loads(path.read_bytes())

    def test_publish_is_content_addressed_and_locator_bound(self) -> None:
        seed, published = self.publish()
        self.assertEqual(seed.parent, self.store / "objects")
        self.assertEqual(seed.name, published["contentDigest"].removeprefix("sha256:"))
        self.assertEqual(
            self.locator(),
            {
                "authorityKey": AUTHORITY,
                "contentDigest": published["contentDigest"],
                "schema": cache.LOCATOR_SCHEMA,
            },
        )
        self.assertEqual(published["entries"], 6)
        self.assertEqual(cache.resolve_seed(self.store, AUTHORITY), (seed, published))
        self.assertEqual(cache.validate_seed(seed, AUTHORITY), published)
        self.assertEqual(stat.S_IMODE(seed.stat().st_mode), 0o500)
        self.assertEqual(
            stat.S_IMODE(
                (self.store / "locators" / f"{'a' * 64}.json").stat().st_mode
            ),
            0o400,
        )

    def test_publish_validate_and_cow_clone_preserve_exact_content(self) -> None:
        seed, published = self.publish()
        clone = self.root / "clone"
        cloned = cache.clone_seed(seed, clone, AUTHORITY)
        self.assertEqual(cloned, published)
        self.assertNotEqual(os.stat(seed).st_ino, os.stat(clone).st_ino)
        self.assertEqual((clone / ".cargo" / "registry" / "crate").read_bytes(), b"crate")
        self.assertTrue(os.access(clone / ".gradle" / "wrapper" / "gradle", os.X_OK))

    def test_cache_artifacts_are_hashed_streamingly(self) -> None:
        candidate = self.candidate(b"x" * (3 * 1024 * 1024))
        with (
            mock.patch.object(cache, "_hash_file_at", wraps=cache._hash_file_at) as hashed,
            mock.patch.object(cache, "_read_file_at", wraps=cache._read_file_at) as read_small,
        ):
            cache.publish_seed(candidate, self.store, AUTHORITY)
        self.assertGreater(hashed.call_count, 0)
        self.assertTrue(
            all(call.args[2] <= cache.MAX_MANIFEST_BYTES for call in read_small.call_args_list)
        )

    def test_same_content_can_be_located_by_two_runtime_authorities(self) -> None:
        first, first_manifest = self.publish(AUTHORITY)
        second, second_manifest = self.publish(OTHER_AUTHORITY)
        self.assertEqual(first, second)
        self.assertEqual(first_manifest, second_manifest)
        self.assertEqual(len(list((self.store / "objects").iterdir())), 1)
        self.assertEqual(self.locator(OTHER_AUTHORITY)["contentDigest"], first_manifest["contentDigest"])

    def test_volatile_locks_are_removed_before_publication(self) -> None:
        candidate = self.candidate()
        (candidate / ".cargo" / ".package-cache").write_bytes(b"locked")
        (candidate / ".gradle" / "cache.lock").write_bytes(b"locked")
        (candidate / ".gradle" / "daemon").mkdir()
        (candidate / ".gradle" / "daemon" / "state").write_bytes(b"volatile")
        seed, _published = cache.publish_seed(candidate, self.store, AUTHORITY)
        self.assertFalse((seed / ".cargo" / ".package-cache").exists())
        self.assertFalse((seed / ".gradle" / "cache.lock").exists())
        self.assertFalse((seed / ".gradle" / "daemon").exists())

    def test_concurrent_publishers_singleflight_one_content_object(self) -> None:
        candidates = [self.candidate(), self.candidate()]
        barrier = threading.Barrier(3)
        results: list[tuple[Path, dict[str, object]]] = []
        errors: list[BaseException] = []

        def publish(candidate: Path) -> None:
            try:
                barrier.wait(timeout=5)
                results.append(cache.publish_seed(candidate, self.store, AUTHORITY))
            except BaseException as error:
                errors.append(error)

        threads = [
            threading.Thread(target=publish, args=(candidate,))
            for candidate in candidates
        ]
        with mock.patch.object(
            cache, "_rename_no_replace", wraps=cache._rename_no_replace
        ) as rename:
            for thread in threads:
                thread.start()
            barrier.wait(timeout=5)
            for thread in threads:
                thread.join(10)
            self.assertEqual(rename.call_count, 1)
        self.assertFalse(errors)
        self.assertEqual(len(results), 2)
        self.assertEqual(results[0], results[1])
        self.assertEqual(len(list((self.store / "objects").iterdir())), 1)
        self.assertEqual(len(list((self.store / "locators").iterdir())), 1)
        self.assertTrue(all(not candidate.exists() for candidate in candidates))

    def test_tampered_seed_and_locator_refuse_validation_and_clone(self) -> None:
        seed, published = self.publish()
        artifact = seed / ".cargo" / "registry" / "crate"
        artifact.chmod(0o600)
        artifact.write_bytes(b"tampered")
        artifact.chmod(0o400)
        with self.assertRaises(cache.CacheAuthorityError):
            cache.validate_seed(seed, AUTHORITY)
        with self.assertRaises(cache.CacheAuthorityError):
            cache.clone_seed(seed, self.root / "clone", AUTHORITY)

        seed.chmod(0o700)
        (seed / "unbound-empty-directory").mkdir(mode=0o500)
        seed.chmod(0o500)
        with self.assertRaises(cache.CacheAuthorityError):
            cache.validate_seed(seed, AUTHORITY)
        seed.chmod(0o700)
        (seed / "unbound-empty-directory").rmdir()
        seed.chmod(0o500)

        artifact.chmod(0o600)
        artifact.write_bytes(b"crate")
        artifact.chmod(0o400)
        locator = self.store / "locators" / f"{'a' * 64}.json"
        locator.chmod(0o600)
        value = self.locator()
        value["contentDigest"] = "sha256:" + "9" * 64
        locator.write_text(
            json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
        locator.chmod(0o400)
        with self.assertRaises(cache.CacheAuthorityError):
            cache.validate_seed(seed, AUTHORITY)
        self.assertEqual(published["contentDigest"], "sha256:" + seed.name)

    def test_symlink_and_toc_tou_mutation_are_refused(self) -> None:
        outside = self.root / "outside"
        outside.write_bytes(b"outside")
        candidate = self.candidate()
        (candidate / "link").symlink_to(outside)
        with self.assertRaises(cache.CacheAuthorityError):
            cache.publish_seed(candidate, self.store, AUTHORITY)
        cache._discard(candidate)
        self.assertEqual(outside.read_bytes(), b"outside")

        candidate = self.candidate(b"x" * (2 * 1024 * 1024))
        artifact = candidate / ".cargo" / "registry" / "crate"
        original_read = cache.os.read
        changed = False

        def mutate_after_read(descriptor: int, size: int) -> bytes:
            nonlocal changed
            block = original_read(descriptor, size)
            if not changed and block:
                changed = True
                with artifact.open("ab") as stream:
                    stream.write(b"changed")
            return block

        with mock.patch.object(cache.os, "read", side_effect=mutate_after_read):
            with self.assertRaises(cache.CacheAuthorityError):
                cache.publish_seed(candidate, self.store, AUTHORITY)

    def test_hardlinked_candidate_file_is_refused_without_mutating_external_inode(self) -> None:
        outside = self.root / "outside-hardlink"
        outside.write_bytes(b"external-authority")
        outside.chmod(0o600)
        candidate = self.candidate()
        alias = candidate / ".cargo" / "registry" / "external-alias"
        os.link(outside, alias)

        with self.assertRaises(cache.CacheAuthorityError):
            cache.publish_seed(candidate, self.store, AUTHORITY)

        self.assertEqual(outside.read_bytes(), b"external-authority")
        self.assertEqual(stat.S_IMODE(outside.stat().st_mode), 0o600)
        self.assertEqual(outside.stat().st_nlink, 1)

    def test_crash_after_object_rename_recovers_exact_publication(self) -> None:
        candidate = self.candidate()
        original_validate = cache._validate_object_fd
        failed = False

        def fail_once(object_fd, digest_value, *, recover_root):
            nonlocal failed
            if recover_root and not failed:
                failed = True
                raise OSError("injected after object rename")
            return original_validate(
                object_fd, digest_value, recover_root=recover_root
            )

        with mock.patch.object(cache, "_validate_object_fd", side_effect=fail_once):
            with self.assertRaises(OSError):
                cache.publish_seed(candidate, self.store, AUTHORITY)
        objects = list((self.store / "objects").iterdir())
        self.assertEqual(len(objects), 1)
        self.assertEqual(stat.S_IMODE(objects[0].stat().st_mode), 0o700)
        self.assertEqual(len(list((self.store / "pending").iterdir())), 1)
        self.assertEqual(len(list((self.store / "locators").iterdir())), 0)

        with cache.seed_creation_lock(self.store, AUTHORITY):
            recovered = cache.recover_seed(self.store, AUTHORITY)
        self.assertIsNotNone(recovered)
        seed, manifest = recovered
        self.assertEqual(seed, objects[0])
        self.assertEqual(cache.validate_seed(seed, AUTHORITY), manifest)
        self.assertEqual(stat.S_IMODE(seed.stat().st_mode), 0o500)
        self.assertEqual(list((self.store / "pending").iterdir()), [])

    def test_locator_publication_crash_recovers_without_second_object(self) -> None:
        original_write = cache._write_immutable_json_at
        calls = 0

        def fail_locator(parent_fd, name, value):
            nonlocal calls
            calls += 1
            if calls == 3:
                raise OSError("injected before locator commit")
            return original_write(parent_fd, name, value)

        with mock.patch.object(cache, "_write_immutable_json_at", side_effect=fail_locator):
            with self.assertRaises(OSError):
                cache.publish_seed(self.candidate(), self.store, AUTHORITY)
        self.assertEqual(len(list((self.store / "objects").iterdir())), 1)
        self.assertEqual(len(list((self.store / "pending").iterdir())), 1)
        seed, manifest = self.publish()
        self.assertEqual(cache.validate_seed(seed, AUTHORITY), manifest)
        self.assertEqual(len(list((self.store / "objects").iterdir())), 1)

    def test_existing_locator_recovers_matching_pending_tail(self) -> None:
        seed, manifest = self.publish()
        pending = self.store / "pending" / f"{'a' * 64}.json"
        pending.write_bytes(
            cache._canonical(cache._pending(AUTHORITY, str(manifest["contentDigest"])))
        )
        pending.chmod(0o400)
        observed_seed, observed_manifest = self.publish()
        self.assertEqual(observed_seed, seed)
        self.assertEqual(observed_manifest, manifest)
        self.assertFalse(pending.exists())

    def test_read_only_validation_does_not_recreate_missing_store_state(self) -> None:
        seed, _manifest = self.publish()
        pending = self.store / "pending"
        pending.rmdir()
        with self.assertRaises(FileNotFoundError):
            cache.validate_seed(seed, AUTHORITY)
        self.assertFalse(pending.exists())

    def test_read_only_validation_does_not_recreate_missing_lifecycle_lock(self) -> None:
        seed, _manifest = self.publish()
        lifecycle = self.store / "locks" / "lifecycle.lock"
        companion = self.store / "locks" / "lifecycle.lock.companion"
        lifecycle.unlink()
        with self.assertRaises(cache.CacheAuthorityError):
            cache.validate_seed(seed, AUTHORITY)
        self.assertFalse(lifecycle.exists())
        self.assertTrue(companion.exists())
        candidate = self.candidate()
        with self.assertRaises(cache.CacheAuthorityError):
            cache.publish_seed(candidate, self.store, AUTHORITY)
        self.assertFalse(lifecycle.exists())
        self.assertTrue(companion.exists())
        self.assertFalse(candidate.exists())

    def test_lock_creation_recovers_only_journaled_candidate_states(self) -> None:
        for linked_primary in (False, True):
            with self.subTest(linked_primary=linked_primary):
                store = self.root / ("store-primary" if linked_primary else "store-candidate")
                store_fd, children = cache._open_store(store, create=True)
                lock = None
                try:
                    name = "crash.lock"
                    candidate = ".crash.lock.candidate"
                    descriptor = os.open(
                        candidate,
                        os.O_RDWR | os.O_CREAT | os.O_EXCL,
                        0o600,
                        dir_fd=children["locks"],
                    )
                    os.close(descriptor)
                    if linked_primary:
                        os.link(
                            candidate,
                            name,
                            src_dir_fd=children["locks"],
                            dst_dir_fd=children["locks"],
                            follow_symlinks=False,
                        )
                    lock = cache._lock_file(
                        children["locks"], name, shared=False, create=True
                    )
                    lock.revalidate()
                    self.assertFalse(
                        (store / "locks" / candidate).exists()
                    )
                    primary = os.stat(name, dir_fd=children["locks"])
                    companion = os.stat(
                        name + ".companion", dir_fd=children["locks"]
                    )
                    self.assertEqual(primary.st_nlink, 2)
                    self.assertEqual(
                        (primary.st_dev, primary.st_ino),
                        (companion.st_dev, companion.st_ino),
                    )
                finally:
                    if lock is not None:
                        lock.close()
                    for child in children.values():
                        os.close(child)
                    os.close(store_fd)

    def test_lock_creation_refuses_unjournaled_half_pair(self) -> None:
        store_fd, children = cache._open_store(self.store, create=True)
        try:
            descriptor = os.open(
                "half.lock",
                os.O_RDWR | os.O_CREAT | os.O_EXCL,
                0o600,
                dir_fd=children["locks"],
            )
            os.close(descriptor)
            with self.assertRaisesRegex(cache.CacheAuthorityError, "incomplete"):
                cache._lock_file(
                    children["locks"], "half.lock", shared=False, create=True
                )
            self.assertFalse((self.store / "locks" / ".half.lock.candidate").exists())
            self.assertFalse((self.store / "locks" / "half.lock.companion").exists())
        finally:
            for child in children.values():
                os.close(child)
            os.close(store_fd)

    def test_manifest_mutation_after_closure_scan_is_refused(self) -> None:
        seed, _manifest = self.publish()
        manifest = seed / cache.SEED_MANIFEST
        original_scan = cache._scan_tree_fd
        mutated = False

        def mutate_after_scan(*args, **kwargs):
            nonlocal mutated
            value = original_scan(*args, **kwargs)
            if kwargs.get("exclude_root") == {cache.SEED_MANIFEST} and not mutated:
                mutated = True
                manifest.chmod(0o600)
                manifest.write_bytes(b"{}\n")
                manifest.chmod(0o400)
            return value

        with mock.patch.object(cache, "_scan_tree_fd", side_effect=mutate_after_scan):
            with self.assertRaises(cache.CacheAuthorityError):
                cache.validate_seed(seed, AUTHORITY)

    def test_creation_lock_recovers_stale_authority_candidate(self) -> None:
        with cache.seed_creation_lock(self.store, AUTHORITY):
            candidate = cache.create_seed_candidate(self.store, AUTHORITY)
            (candidate / "partial").write_bytes(b"partial")
        self.assertTrue(candidate.exists())
        with cache.seed_creation_lock(self.store, AUTHORITY):
            self.assertIsNone(cache.recover_seed(self.store, AUTHORITY))
            self.assertFalse(candidate.exists())

    def test_pending_before_object_rename_recovers_sealed_candidate(self) -> None:
        with cache.seed_creation_lock(self.store, AUTHORITY):
            candidate = cache.create_seed_candidate(self.store, AUTHORITY)
            (candidate / ".cargo" / "registry").mkdir(parents=True, mode=0o700)
            (candidate / ".cargo" / "registry" / "crate").write_bytes(b"crate")
            store_fd, children = cache._open_store(self.store)
            lifecycle = None
            candidate_fd = None
            try:
                lifecycle = cache._lifecycle_lock(
                    children["locks"], shared=False, create=True
                )
                candidate_fd = os.open(
                    candidate.name, cache._directory_flags(), dir_fd=store_fd
                )
                manifest = cache._prepare_candidate(candidate_fd)
                pending_name = f"{'a' * 64}.json"
                self.assertTrue(
                    cache._write_immutable_json_at(
                        children["pending"],
                        pending_name,
                        cache._pending(AUTHORITY, str(manifest["contentDigest"])),
                    )
                )
            finally:
                if candidate_fd is not None:
                    os.close(candidate_fd)
                if lifecycle is not None:
                    lifecycle.close()
                for descriptor in children.values():
                    os.close(descriptor)
                os.close(store_fd)

        self.assertTrue(candidate.exists())
        self.assertEqual(len(list((self.store / "objects").iterdir())), 0)
        with cache.seed_creation_lock(self.store, AUTHORITY):
            recovered = cache.recover_seed(self.store, AUTHORITY)
        self.assertIsNotNone(recovered)
        seed, recovered_manifest = recovered
        self.assertFalse(candidate.exists())
        self.assertEqual(recovered_manifest, manifest)
        self.assertEqual(cache.validate_seed(seed, AUTHORITY), manifest)
        self.assertEqual(list((self.store / "pending").iterdir()), [])

    def test_conflicting_authority_candidate_is_discarded(self) -> None:
        self.publish()
        conflicting = self.candidate(b"different")
        with self.assertRaises(cache.CacheAuthorityError):
            cache.publish_seed(conflicting, self.store, AUTHORITY)
        self.assertFalse(conflicting.exists())

    def test_replaced_lifecycle_lock_refuses(self) -> None:
        seed, _manifest = self.publish()
        lock = self.store / "locks" / "lifecycle.lock"
        lock.unlink()
        lock.symlink_to(self.root / "outside-lock")
        with self.assertRaises((cache.CacheAuthorityError, OSError)):
            cache.validate_seed(seed, AUTHORITY)

    def test_lock_primary_replacement_refuses_holder_and_contender(self) -> None:
        store_fd, children = cache._open_store(self.store)
        holder = None
        try:
            holder = cache._lock_file(
                children["locks"], "replacement-primary.lock", shared=True, create=True
            )
            primary_before = os.stat(
                "replacement-primary.lock", dir_fd=children["locks"]
            )
            companion_before = os.stat(
                "replacement-primary.lock.companion", dir_fd=children["locks"]
            )
            self.assertEqual(
                (primary_before.st_dev, primary_before.st_ino),
                (companion_before.st_dev, companion_before.st_ino),
            )
            self.assertEqual(primary_before.st_nlink, 2)
            self.assertEqual(companion_before.st_nlink, 2)
            os.unlink("replacement-primary.lock", dir_fd=children["locks"])
            replacement = os.open(
                "replacement-primary.lock",
                os.O_CREAT | os.O_EXCL | os.O_WRONLY,
                0o600,
                dir_fd=children["locks"],
            )
            os.close(replacement)
            with self.assertRaises(cache.CacheAuthorityError):
                cache._lock_file(
                    children["locks"],
                    "replacement-primary.lock",
                    shared=True,
                    create=False,
                )
            with self.assertRaises(cache.CacheAuthorityError):
                holder.revalidate()
        finally:
            if holder is not None:
                holder.close()
            for descriptor in children.values():
                os.close(descriptor)
            os.close(store_fd)

    def test_lock_companion_replacement_refuses_holder_and_contender(self) -> None:
        store_fd, children = cache._open_store(self.store)
        holder = None
        try:
            holder = cache._lock_file(
                children["locks"], "replacement-peer.lock", shared=True, create=True
            )
            companion = "replacement-peer.lock.companion"
            os.unlink(companion, dir_fd=children["locks"])
            replacement = os.open(
                companion,
                os.O_CREAT | os.O_EXCL | os.O_WRONLY,
                0o600,
                dir_fd=children["locks"],
            )
            os.close(replacement)
            with self.assertRaises(cache.CacheAuthorityError):
                cache._lock_file(
                    children["locks"],
                    "replacement-peer.lock",
                    shared=True,
                    create=False,
                )
            with self.assertRaises(cache.CacheAuthorityError):
                holder.revalidate()
        finally:
            if holder is not None:
                holder.close()
            for descriptor in children.values():
                os.close(descriptor)
            os.close(store_fd)

    def test_initial_lock_revalidation_failure_closes_retained_parent_fd(self) -> None:
        store_fd, children = cache._open_store(self.store)
        duplicated: list[int] = []
        original_dup = cache.os.dup

        def capture_dup(descriptor: int) -> int:
            observed = original_dup(descriptor)
            duplicated.append(observed)
            return observed

        try:
            with (
                mock.patch.object(cache.os, "dup", side_effect=capture_dup),
                mock.patch.object(
                    cache.AuthorityLock,
                    "revalidate",
                    side_effect=cache.CacheAuthorityError("injected revalidation failure"),
                ),
            ):
                with self.assertRaises(cache.CacheAuthorityError):
                    cache._lock_file(
                        children["locks"],
                        "revalidation-failure.lock",
                        shared=True,
                        create=True,
                    )
            self.assertEqual(len(duplicated), 1)
            with self.assertRaises(OSError):
                os.fstat(duplicated[0])
        finally:
            for descriptor in children.values():
                os.close(descriptor)
            os.close(store_fd)

    def test_cow_probe_is_bounded_and_leaves_no_probe_tree(self) -> None:
        probe = self.root / "probe"
        cache.probe_cow(probe)
        self.assertEqual(list(probe.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
