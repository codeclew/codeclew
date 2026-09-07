# Publishing the macOS and Linux pilot

The public distribution is built only from a clean semantic version tag on the
default branch. End-user machines never compile Codeclew.

## Release contents

The `macOS and Linux release` GitHub Actions workflow builds six fixed asset names:

- `codeclew-macos-arm64.tar.gz`;
- `codeclew-macos-x86_64.tar.gz`;
- `codeclew-kotlin23-macos-arm64.tar.gz`;
- `codeclew-kotlin23-macos-x86_64.tar.gz`;
- `codeclew-linux-x86_64.tar.gz`;
- `codeclew-kotlin23-linux-x86_64.tar.gz`;
- one `.sha256` file for each archive;
- `install.sh` and `install.sh.sha256` for offline installation.

The default archive contains the Kotlin 2.4.10 `core` profile. The optional
`kotlin23` archive adds the Kotlin 2.3.0 read-only preview. Each archive contains
one immutable executable runtime capsule, its source-bound seed authority, the
installed launcher, and a minimal hash-closed bootstrap payload. Build-only
component-cache copies and the full Git checkout are excluded. The bootstrap
payload manifest remains bound to the exact Git commit and tree, and the
installed warm path never compiles Codeclew.

Apple Silicon is built on `macos-15`; Intel is built on `macos-15-intel`.
Linux x86_64 is built on `ubuntu-22.04` for a glibc 2.35 baseline and also serves
Windows x64 through WSL2. The build installs Python 3.12 explicitly. End users
need Python 3.11+; Ubuntu 24.04 inside WSL2 is a suitable target. Native Windows,
Linux ARM64, and musl distributions are outside this matrix.

The workflow verifies `uname -s` and `uname -m` before construction and refuses a runtime in
`DEVELOPMENT` mode or a worker set outside the published support profile.
Each profile is installed from its produced archive using `install.sh` in offline
mode with isolated install, bin, and state roots. Installation verifies the CLI
version and runtime capabilities. The core archive also undergoes extracted
branch and annotated-tag navigation checks. Publication waits for all three
platform builds and verifies all six archive checksums.

The workflow filename `release-macos.yml`, builder `build_macos_release.py`, and
shared POSIX launcher directory `packaging/macos` retain their historical names.
The builder selects the package's `operatingSystem` and asset name from its
native host; the same launcher and updater work on both macOS and Linux.

## Publishing

1. Merge only a clean, fully verified default branch.
2. Run `python3 -I -S scripts/language_mutation_pilot.py` and require 6/6,
   3/3 per language, with `runtimeMode: RELEASE`.
3. Confirm the workspace version matches the intended tag.
4. Create and push an annotated semantic version tag:

   ```bash
   git tag -a v0.1.0 -m 'Codeclew v0.1.0 macOS pilot'
   git push origin v0.1.0
   ```

5. Wait for qualification, all three platform jobs and the release publication
   job.
   If GitHub loses the tag event or reports `startup_failure` before creating a
   job, dispatch the same immutable tag manually:

   ```bash
   gh workflow run release-macos.yml --ref main -f version=v0.1.0
   ```

   The workflow checks out that exact tag, and the release builder still
   verifies that the tag points at the packaged commit.
6. Verify the public installer on a clean Apple Silicon Mac, a clean Intel
   Mac, and a Linux x86_64 distribution inside WSL2:

   ```bash
   curl -fsSL https://codeclew.github.io/codeclew/install.sh | sh
   clew capabilities --human
   clew doctor attach --human
   clew pack install kotlin23
   clew pack list
   clew pack remove kotlin23
   clew upgrade
   ```

   The final command must report that the newly installed version is already up
   to date without downloading another release bundle.
   Run the WSL2 checks from the Linux shell, with installation, state, and fixture
   repositories below the Linux home directory, not a Windows-mounted drive.
   The hosted Linux build checks do not establish WSL2-specific filesystem
   behavior; record a WSL2 smoke result separately when qualifying a release.

The workflow creates a normal GitHub Release. The installer resolves the latest
release API response to an immutable tag before fetching both the archive and
checksum, avoiding mixed CDN state during publication. Release notes retain the
`public pilot` qualification. The release builder and public
installer both require `clew --version` to equal the release tag without its
leading `v`; a mismatch fails before the release is published or activated.

## Failure and rollback

If any platform build fails, no GitHub Release is created. Fix the problem in
a new commit and publish a new patch version; do not move an already published
tag. If publication partially succeeds, remove the incomplete release before
publishing the new version, but retain the failed workflow logs.

The installer keeps versioned release directories. Rolling back is a pinned
installation:

```bash
curl -fsSL https://codeclew.github.io/codeclew/install.sh | \
  CODECLEW_VERSION=v0.1.0 sh
```

Do not replace an existing version directory with different bytes. The pilot
uses SHA-256 transport integrity; Apple code signing, notarization, immutable
release enforcement, and independent signature verification remain required
before a general-availability claim.

## Installing from manually downloaded assets

When GitHub API or release downloads are unavailable from the target machine,
download these four files from one Codeclew Release on any machine or through a
browser and copy them
into one local directory:

- `install.sh` and `install.sh.sha256`;
- `codeclew-macos-arm64.tar.gz` (or `codeclew-macos-x86_64.tar.gz`);
- the matching `.tar.gz.sha256` file.

On Linux or WSL2, choose `codeclew-linux-x86_64.tar.gz` and its checksum instead.
Use the version of a release containing those Linux assets. The installer selects
Linux from `uname -s` and checks glibc 2.35+ before downloading anything. It computes
SHA-256 using the required Python interpreter, so installation does not need Perl
or `shasum` on Linux.

Optionally verify the installer before running it:

```bash
shasum -a 256 -c install.sh.sha256
```

Install the exact version without any network access:

```bash
CODECLEW_VERSION=v0.2.19 \
CODECLEW_ASSET_DIR="$PWD" \
/bin/sh ./install.sh
```

The directory must be an absolute, non-symlink path. Local mode refuses
`CODECLEW_VERSION=latest`, selects the archive for the current architecture,
rejects symlinked inputs, verifies SHA-256, and then performs the same embedded
version, profile, and runtime checks as the online installer. Set
`CODECLEW_PACKS=kotlin23` and download the matching `codeclew-kotlin23-*` pair
to install that optional profile.

Upgrade from local files by downloading the newer archive/checksum pair and
re-running the installer with the newer explicit tag. Use the same install and
bin roots as the existing installation; their defaults already match:

```bash
CODECLEW_VERSION=vNEWER \
CODECLEW_ASSET_DIR=/absolute/path/to/newer-assets \
/bin/sh /absolute/path/to/install.sh
clew --version
```

The versions remain side by side under `CODECLEW_INSTALL_ROOT/releases`; the
launcher symlink is switched atomically only after checksum, embedded-version,
profile, and runtime verification pass. Managed state in `CODECLEW_HOME` is
not migrated or rewritten by installation or upgrade.
