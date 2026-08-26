# Publishing the macOS pilot

The public distribution is built only from a clean semantic version tag on the
default branch. End-user machines never compile Codeclew.

## Release contents

The `macOS release` GitHub Actions workflow builds two fixed asset names:

- `codeclew-macos-arm64.tar.gz`;
- `codeclew-macos-x86_64.tar.gz`;
- one `.sha256` file for each archive.

Each archive contains a clean, credential-free source checkout, an immutable
release runtime capsule, its source-bound seed authority, and the installed
launcher. The source checkout remains necessary because the current bootstrap
verifies the release seed against the exact Git commit and tree; it does not
compile on the installed warm path.

Apple Silicon is built on `macos-15`; Intel is built on `macos-15-intel`. The
workflow verifies `uname -m` before construction and refuses a runtime in
`DEVELOPMENT` mode or a worker set outside the published support profile.

## Publishing

1. Merge only a clean, fully verified default branch.
2. Confirm the workspace version matches the intended tag.
3. Create and push an annotated semantic version tag:

   ```bash
   git tag -a v0.1.0 -m 'Codeclew v0.1.0 macOS pilot'
   git push origin v0.1.0
   ```

4. Wait for both architecture jobs and the release publication job.
5. Verify the public installer on a clean Apple Silicon Mac and a clean Intel
   Mac:

   ```bash
   curl -fsSL https://codeclew.github.io/codeclew/install.sh | sh
   clew capabilities --human
   clew doctor --human
   clew upgrade
   ```

   The final command must report that the newly installed version is already up
   to date without downloading another release bundle.

The workflow creates a normal GitHub Release so the stable
`releases/latest/download/...` endpoint used by the installer resolves. Release
notes retain the `public pilot` qualification.

## Failure and rollback

If either architecture fails, no GitHub Release is created. Fix the problem in
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
