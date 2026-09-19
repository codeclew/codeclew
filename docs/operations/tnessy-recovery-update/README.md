# Proposed tnessy guidance update

This directory contains a patch and review copies for three existing files in the sibling `kbt-skills` repository. The source repository was read only; this session cannot write outside its launch workspace. No skill was installed or reinstalled, no runtime code or schema changed, and no package version was bumped.

## Files

- `tnessy/nessy-skill/execute-claude-plan/SKILL.md`: focused implementation, actual-only command evidence, unique owned fixtures, concrete blockers and limits, iteration versus terminal verification, preservation of partial work.
- `tnessy/claude-skill/orchestrate-nessy-plan/SKILL.md`: concrete small plans, correct test-level ownership, schema/command inspection, accepted predecessor routing and honest review.
- `tnessy/README.md`: limits of VALID and operational guidance consistent with both skills.

`tnessy-guidance.patch` uses paths relative to the `kbt-skills` repository root. `source-sha256.json` records the inspected base files. The executor workspace placeholder and both allowed-tools lists are unchanged. Safety prohibitions are not relaxed.

## Apply from a normal shell

First inspect any current changes to these three target files. The following commands check and apply only this local documentation patch; no commit or installation is performed:

```sh
git -C /path/to/kbt-skills apply --check /path/to/codeclew/docs/operations/tnessy-recovery-update/tnessy-guidance.patch
git -C /path/to/kbt-skills apply /path/to/codeclew/docs/operations/tnessy-recovery-update/tnessy-guidance.patch
```

Run outside the current sandbox, whose writable boundary excludes that repository. Alternatively start a new tclaude session from `kbt-skills` and ask it to inspect/apply this patch. Do not force application if the base changed.

`git apply --check` against the real target succeeded during preparation. Review copies were checked for trailing whitespace, balanced fenced blocks, unchanged tool permissions, and the single executor workspace placeholder. These are documentation checks, not executor behavior tests.

For a distributable upgrade, follow `tnessy/README.md`'s maintainer procedure: choose/bump the package version and verify installation in a disposable workspace. That release/install operation is intentionally not part of this patch. The shared lightweight validator is unchanged and still requires manual plan/report/event comparison; the new text makes that limitation explicit rather than promising validation it does not implement.
