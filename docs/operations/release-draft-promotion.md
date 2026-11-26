# Verify and promote an official release draft

The release workflow qualifies and packages the exact source commit, verifies
checksums and requires successful CI before staging a GitHub **draft** release.
It never automatically makes that draft public. Build-only candidates do not
create a release. They run the large Linux Java producer case only when the
explicit `qualify_large_java` input is true.

For a correction to an existing, unpublished tag, provide its current peeled
commit as `expected_existing_tag_commit` on the build-only dispatch. The
candidate helper requires that both the disposable checkout and origin still
have that target, then binds the version locally to the workflow commit.
Build jobs have read-only repository permissions; the helper never pushes.
This permits official candidate and large-producer acceptance before an
explicitly authorized public tag correction. Preserve the old annotated tag
object and all failed evidence. Do not overwrite an existing public release.
Only after independent candidate acceptance may an authorized operator move
the public tag, using a lease on the exact old tag object. The final tagged
workflow still qualifies its official packages and stages a draft.

For a tagged release, a fresh Linux job downloads the final official Linux
artifact, verifies its checksum and runs the owned Java producer qualification
once. Before allocating swap or extracting, it checks RAM and the full disk
requirement: 8 GiB swap plus 8 GiB remaining launch reserve plus the archive's
uncompressed member sizes and 1 GiB extraction allowance. The selected three conventional Maven compilations
must actually exceed both former aggregate limits: 524,288 matching-domain facts
and 268,435,456 CAS payload bytes. Arithmetic predictions and incomplete captures
cannot pass. Runner resources, a fixed 8-GiB swap envelope, 45-minute wall limit,
512-MiB disk reserve, exact source/archive identity, frozen inputs, stdout/stderr,
live progress and sampled resources are retained as `java-large-producer-evidence`.
That evidence artifact is excluded from release assets. A failure blocks staging;
there is no automatic retry, resource escalation or bypass.

Independently inspect the exact draft assets and primary qualification artifacts.
Use the packaged launcher and preserved archive modes, not a source build. Test
small compiler semantic closure, failure persistence, retained snapshot reads and
affected SOURCE paths. Preserve failed results. Confirm all required jobs passed
for this exact commit and source tree, and all eight expected public assets and
checksums match the independently accepted archives.

## Installed documentation author inputs

Run the [Weather checkpoint](../../scripts/qualification/weather-preauthor.md)
before making a draft public whenever the release changes installed documentation
preparation, how it selects retained evidence, or what goes into the author packet.
Verify and install one official core archive produced for that draft, then use
a run prepared by that authenticated installation. The independent semantic
review requires 45/45 full atoms and zero material false claims. Keep the frozen
run, independent review and release-bound report outside the installed package
and release assets.

Verify the report against the same downloaded archive, the peeled release
commit, the authenticated installation and the original frozen evidence:

```sh
python3 -I -S scripts/qualification/weather-preauthor.py \
  --release-archive "$WEATHER_RELEASE_ARCHIVE" \
  --release-commit "$(git rev-parse "$TASK_RELEASE_VERSION^{commit}")" \
  --skill "$INSTALLED_SKILL" --runtime-key "$INSTALLED_RUNTIME_KEY" \
  --run-dir "$RUN" --source-repo "$WEATHER_REPO" \
  --review "$EVALUATION/review.json" \
  --verify-release-report "$EVALUATION/release-report.json"
```

A nonzero result blocks this qualification. Verification checks the archive,
seed, bundled skill and frozen inputs, then checks the independent grades and
retained source proof again. It starts no author and performs no capture. A
different archive, a stock release run or a standalone PASS summary cannot
substitute for the matching frozen evidence.

When every applicable qualification passes, promote the existing draft without rebuilding or
uploading replacement assets:

```sh
gh release view VERSION --json tagName,isDraft,assets,url
gh release edit VERSION --draft=false --latest
```

Replace VERSION with the exact accepted tag. Then verify public tag/commit,
platform assets/checksums and supported isolated installation/update. Promotion
uses the release authorization already granted for that slice; it is not a new
human approval layer. A failed required gate still blocks promotion. Never
promote because packaging completed while semantic acceptance is outstanding.
The workflow refuses to overwrite an already public release on a rerun.
