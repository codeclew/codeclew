# Installed Weather author-input qualification

This release qualification reserves the original 45-atom Weather rubric for
evaluation. Its SHA-256 is
`3d9f22966411dde050d16819e1d173a5b8cff570654a2f77bf84675c0a00b059`.
Keep the nine historical operation identifiers in the rubric to preserve the old
denominator. The validator resolves live entry requirements by service and
declared route or root symbol at Weather commit
`c9a4283cb5bbe2409b95561aca4e751a71255ff9`. The companion component list breaks
compound criteria into reviewable parts. Apply all of the original acceptance
text alongside those parts.

The accepted precursor `8e3dbb58f21c0714f882f59ebf376571569ad4e5` delivered
45/45 explicit meanings on the research reader-draft surface, with no author.
Its frozen grades have SHA-256
`8c01c551d32dd3bb433d9d5cd8a3793ae056848b9012ead4b88eaf64a826cdd2`.
Treat those old witnesses as prior results. Every installed run supplies new
verbatim witnesses from its own ordinary author input and new semantic grades.

Use the candidate already authenticated and installed through the release
procedure. Confirm its runtime key and unmodified bundled skill. Capture the
three Weather services at the pinned revision, using their ordinary retained
inputs and the installed documentation runner. The job uses
`"evidenceMode": "PREPARED_FACTS"`. The first command prepares but starts zero
authors. Omit `--job` when collecting from an already prepared frozen run.

```sh
python3 -I -S scripts/qualification/weather-preauthor.py \
  --skill "$INSTALLED_SKILL" --runtime-key "$INSTALLED_RUNTIME_KEY" \
  --job "$JOB" --run-dir "$RUN" > "$EVALUATION/witnesses.json"
```

Keep the rubric, component list, witness catalog and critic output outside the
installed skill and the author's workspace. The witness catalog includes only
cited statements in the required author-facing workspace views, inline prompt,
or actual frozen isolated page prompts. An isolated page witness retains its
prompt location, source alias, coordinates and exact visible source digest.
Sidecars and raw code help verify sources and yield no semantic credit.

Give an independent reviewer the frozen witness catalog and exact pinned sources.
For every rubric ID, the reviewer grades every substantive component and names
the supporting witness IDs. The reviewer must read the complete relevant
assertions together, check that the guards, ordering, effects and assumptions
belong to the same applicable path, and inspect the named source locations.
Runtime claims require authority beyond a static implementation, a declared
link or a candidate dispatch. Scan delivered facts and boundaries for material
false assertions even when another statement elsewhere is correct.

Use the following shape for the full independent review:

```json
{
  "rubricSha256": "from witnesses.json",
  "runDigest": "from witnesses.json",
  "reviewer": "independent reviewer identifier",
  "materialFalseClaims": [],
  "criteria": {
    "route-add-place.html/core/1": {
      "status": "FULL",
      "witnesses": {
        "1": ["sha256:first supporting witness ID"],
        "2": ["sha256:second supporting witness ID"],
        "3": ["sha256:third supporting witness ID"]
      }
    }
  }
}
```

The example shows one of the 45 required rows. Use `PARTIAL`, `ABSENT` or
`CONTRADICTED` and include a `reason` when full meaning is unavailable. A
review can cite multiple supporting statements per component. Record material
false claims with their locations and the source that contradicts them.
When a file-scoped witness supplies one of those statements, include a separate
eligible entry witness that explicitly links its target to the file fact in the
same component. Record the connection in that atom's `scopeLinks` map keyed by
the file witness ID, with `via` set to the supporting entry witness ID and
`symbol` set to the exact shared source symbol. The named symbol and source
must refer to the same target. Sharing a code file alone is insufficient.

```sh
python3 -I -S scripts/qualification/weather-preauthor.py \
  --skill "$INSTALLED_SKILL" --runtime-key "$INSTALLED_RUNTIME_KEY" \
  --run-dir "$RUN" --source-repo "$WEATHER_REPO" \
  --review "$EVALUATION/review.json"
```

Verification recomputes the witness catalog from the installed job's immutable
inputs. It checks the current fact and operation scope, retaining each source
anchor only when its installed citation manifest matches the saved snapshot and
the exact source bytes match the pinned Git line and byte range. An accepted
result requires full independent coverage of all 45 atoms and zero material
false claims. The validator verifies the reviewer’s evidence and accounting.
Interpretation of prose and the absence of contradictions require independent
semantic judgment.

## Release-bound evidence report

For the [draft promotion checkpoint](../../docs/operations/release-draft-promotion.md),
use the exact official archive from the draft and the corresponding authenticated
installation. Add the archive and peeled tag commit to the witness collection
command. The resulting catalog carries the target payload identity. After the
independent review, generate the release report:

```sh
python3 -I -S scripts/qualification/weather-preauthor.py \
  --release-archive "$WEATHER_RELEASE_ARCHIVE" \
  --release-commit "$(git rev-parse "$TASK_RELEASE_VERSION^{commit}")" \
  --skill "$INSTALLED_SKILL" --runtime-key "$INSTALLED_RUNTIME_KEY" \
  --run-dir "$RUN" --source-repo "$WEATHER_REPO" \
  --review "$EVALUATION/review.json" > "$EVALUATION/release-report.json"
```

The report records the archive, source tree, runtime seed, bundled skill, frozen
run, independent grade and witness catalog digests. Keep it with the original
frozen inputs and review. A promotion verifier uses the same command with
`--verify-release-report "$EVALUATION/release-report.json"`, without `--job`.
This reruns only the mechanical validation against the original reviewed
evidence. A stock-release result applies only to its original archive and
installation. Failed or mismatched reports block promotion within this scope.
