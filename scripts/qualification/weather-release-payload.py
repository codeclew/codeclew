"""Bind installed author-evidence qualification to one official archive."""

import hashlib
import json
from pathlib import Path
import re
import tarfile


ARCHIVE_LIMIT = 2 * 1024**3
ARCHIVE_CONTENT_LIMIT = 8 * 1024**3
META_LIMIT = 4 * 1024**2
SKILL_FILE_LIMIT = 32 * 1024**2
SKILL_LIMIT = 128 * 1024**2
GIT_ID = re.compile(r"[a-f0-9]{40}")
DIGEST = re.compile(r"sha256:[a-f0-9]{64}")
SEED = re.compile(r"codeclew/seed/release-N-([a-f0-9]{40})/seed\.json")
SKILL_PREFIX = "codeclew/source/skills/codeclew/"
METADATA = "codeclew/release.json"
SOURCE = "codeclew/source/release-source.json"
LAUNCHER = "codeclew/bin/clew"


def need(condition, reason):
    if not condition:
        raise ValueError(reason)


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def sha(raw):
    return "sha256:" + hashlib.sha256(raw).hexdigest()


def digest_file(path, limit):
    need(path.is_file() and not path.is_symlink() and 0 < path.stat().st_size <= limit,
         "Missing, unsafe, or oversized release qualification input")
    with path.open("rb") as stream:
        return "sha256:" + hashlib.file_digest(stream, "sha256").hexdigest()


def skill_tree(directory):
    need(directory.is_dir() and not directory.is_symlink(), "Installed skill directory is unavailable")
    result, size = {}, 0
    for path in sorted(directory.rglob("*")):
        relative = path.relative_to(directory)
        if {"node_modules", "__pycache__", ".git"}.intersection(relative.parts):
            continue
        need(not path.is_symlink(), "Installed skill contains a symlink")
        if path.is_dir():
            continue
        need(len(result) < 4096, "Installed skill exceeds qualification file limit")
        if path.is_file() and path.stat().st_size == 0:
            result[relative.as_posix()] = sha(b"")
        else:
            result[relative.as_posix()] = digest_file(path, SKILL_FILE_LIMIT)
        size += path.stat().st_size
        need(size <= SKILL_LIMIT, "Installed skill exceeds qualification byte limit")
    return result


def read_archive(path, revision):
    need(bool(GIT_ID.fullmatch(revision)), "An exact expected release commit is required")
    archive_digest = digest_file(path, ARCHIVE_LIMIT)
    seed_path = f"codeclew/seed/release-N-{revision}/seed.json"
    wanted = {METADATA, SOURCE, LAUNCHER, seed_path}
    parts, skill = {}, {}
    total, content_size = 0, 0
    with tarfile.open(path, "r|gz") as archive:
        for count, member in enumerate(archive, start=1):
            content_size += member.size
            need(count <= 100_000 and 0 <= content_size <= ARCHIVE_CONTENT_LIMIT,
                 "Release archive content limits exceeded")
            name = member.name
            if name not in wanted and not name.startswith(SKILL_PREFIX) and not SEED.fullmatch(name):
                continue
            if member.isdir() and name.startswith(SKILL_PREFIX):
                continue
            limit = SKILL_FILE_LIMIT if name.startswith(SKILL_PREFIX) else META_LIMIT
            need(member.isfile() and 0 <= member.size <= limit, "Invalid qualified release archive member")
            with archive.extractfile(member) as stream:
                raw = stream.read(limit + 1)
            need(len(raw) == member.size, "Truncated qualified release archive member")
            if name.startswith(SKILL_PREFIX):
                relative = name.removeprefix(SKILL_PREFIX)
                need(relative and relative not in skill and "\\" not in relative and
                     all(part not in {"", ".", ".."} for part in relative.split("/")),
                     "Invalid bundled skill path")
                skill[relative] = sha(raw)
                total += member.size
                need(0 < len(skill) <= 4096 and total <= SKILL_LIMIT, "Bundled skill exceeds qualification bounds")
            else:
                need(name in wanted and name not in parts, "Unexpected or duplicate release payload member")
                parts[name] = raw
    need(parts.keys() == wanted, "Qualified archive metadata or launcher is incomplete")
    release, source, seed = (json.loads(parts[name]) for name in (METADATA, SOURCE, seed_path))
    need(all(isinstance(value, dict) for value in (release, source, seed))
         and isinstance(source.get("files"), list), "Release payload metadata has an invalid shape")
    need(release.get("schema") == "codeclew-release/2.0" and release.get("status") == "PILOT_READY"
         and release.get("profile") == "core" and re.fullmatch(r"v\d+\.\d+\.\d+", release.get("version", "")),
         "Qualification requires the produced official core archive")
    tree = release.get("sourceTree", "")
    need(GIT_ID.fullmatch(tree) and all(row.get("sourceRevision") == revision and row.get("sourceTree") == tree
                                      for row in (release, source, seed)), "Release commit or source tree changed")
    payload = source.get("manifestDigest")
    need(source.get("schema") == "codeclew-release-source/1.0" and
         payload == sha(canonical({**source, "manifestDigest": ""})), "Release source manifest changed")
    need(seed.get("schema") == "codeclew-trusted-release-seed/2.0" and seed.get("mode") == "RELEASE"
         and seed.get("sourcePayloadDigest") == payload and seed.get("seedDigest") ==
         sha(canonical({key: value for key, value in seed.items() if key != "seedDigest"}))
         and all(DIGEST.fullmatch(seed.get(key, "")) for key in ("runtimeKey", "manifestDigest")),
         "Release runtime seed is invalid")
    listed = [(item["path"].removeprefix("skills/codeclew/"), item["sha256"])
              for item in source["files"] if item["path"].startswith("skills/codeclew/")]
    need(skill and len(listed) == len(skill) and dict(listed) == skill,
         "Bundled skill differs from the release source manifest")
    proof = {
        "archiveSha256": archive_digest, "version": release["version"], "sourceRevision": revision, "sourceTree": tree,
        "profile": release["profile"], "operatingSystem": release["operatingSystem"], "architecture": release["architecture"],
        "sourcePayloadDigest": payload, "runtimeKey": seed["runtimeKey"], "runtimeManifestDigest": seed["manifestDigest"],
        "skillTreeDigest": sha(canonical(skill)), "launcherSha256": sha(parts[LAUNCHER]),
        "releaseMetadataSha256": sha(parts[METADATA]), "seedSha256": sha(parts[seed_path]),
    }
    return proof, skill


def check_install(proof, expected_skill, job, skill):
    need(skill_tree(skill) == expected_skill, "Qualification skill differs from the target archive")
    launcher = Path(job["clew"]).resolve(strict=True)
    need(launcher.name == "clew" and launcher.parent.name == "bin"
         and digest_file(launcher, META_LIMIT) == proof["launcherSha256"],
         "Job launcher differs from the target installed archive")
    package = launcher.parent.parent
    seed = package / "seed" / ("release-N-" + proof["sourceRevision"]) / "seed.json"
    need(digest_file(package / "release.json", META_LIMIT) == proof["releaseMetadataSha256"]
         and digest_file(seed, META_LIMIT) == proof["seedSha256"]
         and skill_tree(package / "source" / "skills" / "codeclew") == expected_skill,
         "Job does not reference the authenticated target release install")


def check_pins(proof, pins):
    capabilities = pins["clewCapabilities"]
    need(capabilities["runtimeMode"] == "RELEASE" and capabilities["runtimeKey"] == proof["runtimeKey"]
         and capabilities["runtimeManifestDigest"] == proof["runtimeManifestDigest"]
         and pins["clewVersion"] == "clew " + proof["version"].removeprefix("v")
         and sha(canonical(pins["skillFiles"])) == proof["skillTreeDigest"],
         "Frozen run is not from the target runtime and bundled skill")
