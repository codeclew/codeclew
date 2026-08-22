#!/usr/bin/env python3
"""Portable effective CPU and memory authority for Codeclew admission."""

from __future__ import annotations

import os
from pathlib import Path
import stat


class HostResourceError(RuntimeError):
    pass


def _read_limit(path: Path) -> str | None:
    try:
        descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
        try:
            metadata = os.fstat(descriptor)
            if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > 4096:
                return None
            encoded = os.read(descriptor, 4097)
            after = os.fstat(descriptor)
            if (
                len(encoded) > 4096
                or (metadata.st_dev, metadata.st_ino, metadata.st_size, metadata.st_mtime_ns)
                != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns)
            ):
                return None
        finally:
            os.close(descriptor)
        value = encoded.decode("ascii").strip()
    except (FileNotFoundError, PermissionError, OSError, UnicodeError):
        return None
    return value or None


def _memberships(proc_root: Path) -> dict[str, str]:
    try:
        rows = (proc_root / "self/cgroup").read_text(encoding="ascii").splitlines()
    except (FileNotFoundError, PermissionError, OSError, UnicodeError):
        return {}
    memberships: dict[str, str] = {}
    for row in rows:
        fields = row.split(":", 2)
        if len(fields) != 3 or not fields[2].startswith("/"):
            continue
        relative = fields[2].lstrip("/")
        if ".." in Path(relative).parts:
            continue
        controllers = fields[1].split(",") if fields[1] else ["unified"]
        for controller in controllers:
            memberships[controller] = relative
    return memberships


def _ancestor_directories(root: Path, relative: str) -> list[Path]:
    directories = [root]
    current = root
    for component in Path(relative).parts:
        if component in {"", ".", ".."}:
            raise HostResourceError("cgroup membership authority is invalid")
        current = current / component
        directories.append(current)
    return directories


def _controller_root(root: Path, controller: str) -> Path:
    direct = root / controller
    if direct.is_dir():
        return direct
    try:
        candidates = sorted(
            path
            for path in root.iterdir()
            if path.is_dir() and controller in path.name.split(",")
        )
    except OSError:
        return direct
    return candidates[0] if candidates else direct


def _positive_int(value: str | None) -> int | None:
    if value is None:
        return None
    try:
        parsed = int(value)
    except ValueError:
        return None
    return parsed if parsed > 0 else None


def _cpu_set_count(value: str | None) -> int | None:
    if value is None:
        return None
    observed: set[int] = set()
    try:
        for token in value.split(","):
            bounds = token.split("-", 1)
            start = int(bounds[0])
            end = int(bounds[-1])
            if start < 0 or end < start or end - start > 1_000_000:
                return None
            observed.update(range(start, end + 1))
    except ValueError:
        return None
    return len(observed) or None


def _quota_cores(quota: int | None, period: int | None) -> int | None:
    if quota is None or period is None:
        return None
    return max(1, quota // period)


def effective_host_resources(
    cgroup_root: Path = Path("/sys/fs/cgroup"),
    proc_root: Path = Path("/proc"),
) -> dict[str, object]:
    online = os.cpu_count()
    if online is None or online <= 0:
        raise HostResourceError("online CPU authority is unavailable")
    affinity = None
    if hasattr(os, "sched_getaffinity"):
        try:
            affinity = len(os.sched_getaffinity(0))
        except (AttributeError, OSError):
            affinity = None
    memberships = _memberships(proc_root)
    unified_dirs = _ancestor_directories(
        cgroup_root, memberships.get("unified", "")
    )
    cpuset_candidates = [
        _cpu_set_count(_read_limit(path / "cpuset.cpus.effective"))
        for path in unified_dirs
    ]
    if not any(cpuset_candidates):
        cpuset_root = _controller_root(cgroup_root, "cpuset")
        cpuset_dirs = _ancestor_directories(
            cpuset_root, memberships.get("cpuset", "")
        )
        cpuset_candidates = [
            _cpu_set_count(_read_limit(path / "cpuset.cpus")) for path in cpuset_dirs
        ]
    cpuset_values = [value for value in cpuset_candidates if value]
    cpuset = min(cpuset_values) if cpuset_values else None

    quota_candidates = []
    for path in unified_dirs:
        cpu_max = _read_limit(path / "cpu.max")
        if cpu_max is None:
            continue
        tokens = cpu_max.split()
        if len(tokens) == 2 and tokens[0] != "max":
            quota_candidates.append(
                _quota_cores(_positive_int(tokens[0]), _positive_int(tokens[1]))
            )
    if not any(quota_candidates):
        cpu_root = _controller_root(cgroup_root, "cpu")
        cpu_dirs = _ancestor_directories(cpu_root, memberships.get("cpu", ""))
        quota_candidates = [
            _quota_cores(
                _positive_int(_read_limit(path / "cpu.cfs_quota_us")),
                _positive_int(_read_limit(path / "cpu.cfs_period_us")),
            )
            for path in cpu_dirs
        ]
    quota_values = [value for value in quota_candidates if value]
    quota_cores = min(quota_values) if quota_values else None
    cpu_candidates = [value for value in (online, affinity, cpuset, quota_cores) if value]
    logical = min(cpu_candidates)

    try:
        pages = os.sysconf("SC_PHYS_PAGES")
        page_size = os.sysconf("SC_PAGE_SIZE")
        physical_memory = int(pages * page_size)
    except (AttributeError, OSError, ValueError) as error:
        raise HostResourceError("physical memory authority is unavailable") from error
    if physical_memory <= 0:
        raise HostResourceError("physical memory authority is invalid")
    memory_candidates = [
        _positive_int(value) if value != "max" else None
        for value in (_read_limit(path / "memory.max") for path in unified_dirs)
    ]
    if not any(memory_candidates):
        memory_root = _controller_root(cgroup_root, "memory")
        memory_dirs = _ancestor_directories(
            memory_root, memberships.get("memory", "")
        )
        memory_candidates = [
            _positive_int(_read_limit(path / "memory.limit_in_bytes"))
            for path in memory_dirs
        ]
    memory_values = [value for value in memory_candidates if value]
    cgroup_memory = min(memory_values) if memory_values else None
    # Some v1 hosts expose a huge sentinel rather than an unlimited token.
    if cgroup_memory is not None and cgroup_memory > physical_memory:
        cgroup_memory = None
    total_memory = min(
        value for value in (physical_memory, cgroup_memory) if value is not None
    )
    return {
        "cpu": {
            "affinityCores": affinity,
            "cgroupQuotaCores": quota_cores,
            "cpusetCores": cpuset,
            "onlineCores": online,
        },
        "logicalCores": logical,
        "memory": {
            "cgroupLimitBytes": cgroup_memory,
            "physicalBytes": physical_memory,
        },
        "schema": "codeclew-effective-host-resources/1.0",
        "totalMemoryBytes": total_memory,
    }
