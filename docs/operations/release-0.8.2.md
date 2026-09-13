# Next release: 0.8.2 reader and developer guides

Prepared on 13 September 2026. The published baseline is 0.8.1. No 0.8.2 tag,
release or site deployment has been created by this reader update.

## Resume here

The source changes add bounded global navigation, a searchable/paginated document
catalogue, shared reader styling, accessible evidence-limit explanations,
installation/engine help and 20 developer runbooks. Root catalogues refresh on
publication while preserving user edits. Existing frozen snapshots are retained.

Completed checks: 32 documentation unit tests (two compiler tests intentionally
ignored), the CLI publication/recovery integration test (246 seconds), Clippy,
formatting and English content checks. Browser checks cover a 501-document
catalogue (20 results per page, type filtering, six global links), real service
pages, 20 runbooks, evidence tooltip display and Escape dismissal.
Full CI has NOT been run for this change. Final tooltip click-toggle and Escape dismissal were verified. Responsive
breakpoints have not been separately exercised in the browser. Local private
service documentation must stay outside this public repository.

Do not run the deferred 40-service qualification or add GitLab/T14 work to this
release. The release workflow's existing conditional mutation qualification is
part of packaging and is separate from that deferred programme.

## 1. Finish and commit the reader change

Run from the Codeclew checkout. Inspect changes before staging; preserve unrelated
work. The source release launcher is `./clew`; installed `clew` is still 0.8.1.

```sh
git status --short
git diff --check
cargo fmt --all --check
python3 -I -S scripts/check_english_content.py

git add crates/clew/assets/documentation/help.html \
  crates/clew/assets/documentation/runbooks.html \
  crates/clew/assets/documentation/template.html \
  crates/clew/assets/documentation/reader.css \
  crates/clew/assets/documentation/reader.js \
  crates/clew/assets/documentation/limits.js \
  crates/clew/src/documentation/reader.rs \
  crates/clew/src/documentation/render.rs \
  crates/clew/src/documentation/history.rs \
  docs/operations/source-documentation.md \
  docs/operations/release-0.8.2.md
python3 -I -S scripts/check_repository_privacy.py --pre-commit
git diff --cached --stat
git commit -m "Improve documentation navigation and developer guides"
```

If already committed, skip that commit. Finish any local reader render already
in progress before starting another source build. Keep the final private render,
companion generators and original notes committed in their own repository.

## 2. Bump the version and run the release gate

Verify the proposed tag is unused. Do not move an existing release tag.

```sh
git switch main
git status --short
git pull --ff-only
test -z "$(git ls-remote --tags origin refs/tags/v0.8.2)"
python3 - <<'PY'
from pathlib import Path
p = Path('Cargo.toml')
s = p.read_text()
assert s.count('version = "0.8.1"') == 1, 'Inspect current workspace version first'
p.write_text(s.replace('version = "0.8.1"', 'version = "0.8.2"', 1))
PY
# Refresh workspace package versions in Cargo.lock using cached dependencies.
cargo check --workspace --offline
git diff -- Cargo.toml Cargo.lock
./scripts/ci-verify.sh > /tmp/codeclew-082-ci.log 2>&1
tail -n 20 /tmp/codeclew-082-ci.log
```

Require a zero exit status and the final PASSED record. Do not infer success from
an incomplete log. Resolve failures before tagging. The version change also
changes compiler/source module identities: recapture and refresh private docs
with the new release if they become stale; never relabel old evidence CURRENT.

Prepare release notes in a temporary Markdown file, covering navigation,
evidence explanations and the new help/runbooks. Keep existing coverage/runtime
limitations. Remove the pending-release sentence from
`docs/operations/source-documentation.md` once this version is ready.

```sh
git add Cargo.toml Cargo.lock docs/operations/source-documentation.md
python3 -I -S scripts/check_repository_privacy.py --pre-commit
git commit -m "Prepare Codeclew 0.8.2"
git push origin main
release_commit=$(git rev-parse HEAD)
gh run list --workflow ci.yml --commit "$release_commit" --limit 3
# Use the numeric run ID returned above; require its success.
gh run watch CI_RUN_ID --exit-status
```

## 3. Publish the exact tested commit

```sh
test -z "$(git status --porcelain)"
git tag -a v0.8.2 -m 'Codeclew v0.8.2: documentation navigation and developer guides'
git push origin v0.8.2
gh run list --workflow release-macos.yml --limit 5
# Select the run for v0.8.2, not an older release.
gh run watch RELEASE_RUN_ID --exit-status
gh release view v0.8.2 --json tagName,url,isDraft,isPrerelease,assets
```

The release workflow qualifies, builds and smoke-tests all three platforms, then
publishes 14 assets (six archives, six checksums and the installer/checksum).
If the tag event never starts, dispatch the SAME tag once:

```sh
gh workflow run release-macos.yml --ref main -f version=v0.8.2
```

For custom release notes, use a file, preserving the generated public-pilot and
installation details:

```sh
gh release view v0.8.2 --json body --jq .body > /tmp/codeclew-082-release-notes.md
# Edit that file to include the concrete reader changes and verified results.
gh release edit v0.8.2 --notes-file /tmp/codeclew-082-release-notes.md
```

## 4. Update and publish the website

After the release succeeds, update current version labels and links in
`site/index.html` and `site/documentation.html`. Explain the new catalogue,
inline explanations and 20 recipes. Update `site/evidence.html` with the actual
0.8.2 release run URL; retain older results as historical evidence. Do not replace
old versions or measured run IDs blindly across the repository.

```sh
python3 -I -S scripts/build_cli_documentation.py --check
python3 -I -S scripts/build_site_navigation.py --check
python3 -I -S scripts/test_site.py
python3 -I -S scripts/check_english_content.py
git add site/index.html site/documentation.html site/evidence.html site/search-index.json
python3 -I -S scripts/check_repository_privacy.py --pre-commit
git commit -m "Publish Codeclew 0.8.2 documentation on the site"
git push origin main
gh run list --workflow pages.yml --limit 3
gh run watch PAGES_RUN_ID --exit-status
```

Regenerate the site's navigation/search index with
`python3 -I -S scripts/build_site_navigation.py` if its check reports a mismatch,
then rerun the affected site checks. If no Pages run starts, dispatch
`gh workflow run pages.yml --ref main`. Verify the live site against the committed
files, rather than treating a successful deployment alone as a content check.

## 5. Verify the installed consumer experience

```sh
clew upgrade
clew --version
clew pack list
clew skill install --agent codex --force
smoke_root=$(mktemp -d /tmp/codeclew-082-docs.XXXXXX)
clew docs init --root "$smoke_root" --title 'Release smoke'
test -f "$smoke_root/docs/help.html"
test -f "$smoke_root/docs/runbooks.html"
test -f "$smoke_root/docs/catalog.html"
clew upgrade
```

Preserve customized skill content before using `--force`. Verify the new help,
20 recipes, favicon and search navigation in the fresh workspace. The second
upgrade should report that the installed version is already current. Retain the
private documentation repository and its transfer bundle separately; never add
private service source or original user notes to the public release.
