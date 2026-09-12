# Repository consolidation after 0.8.0

Date: 2026-09-13. Scope: reconcile committed release work, open pull requests,
local drafts and the public site. No new runtime feature or qualification run.

## Current baseline

The release is [v0.8.0](https://github.com/codeclew/codeclew/releases/tag/v0.8.0),
commit `d9715b94dafcf610cfc61945552b2f2f92a0bdc1`.
`main` incorporates that exact release history. Subsequent consolidation changes
update public documentation and preserve research; the runtime remains 0.8.0.
The immutable tag and distribution assets are unchanged.

The [release workflow](https://github.com/codeclew/codeclew/actions/runs/34722794301)
passed for macOS arm64/x86_64 and Linux x86_64. The released macOS arm64 archive
and installer matched their public checksums; an isolated installed launcher
reported 0.8.0, RELEASE, INSTALLED_RELEASE and skill package 0.8.0. A three-service
local documentation check returned CURRENT with no affected fragments or
unresolved services. Business meaning remained UNASSESSED. Private application
source, generated service documentation and transfer bundles remain outside
this public repository.

## Pull requests

- [PR #8](https://github.com/codeclew/codeclew/pull/8): all three original commits,
  ending at `a2dcefe2296e65f8f98e1cb205b2bbe3589e7989`, are ancestors of 0.8.0.
  Integrating the release into main incorporates the PR without replaying it.
- [PR #6](https://github.com/codeclew/codeclew/pull/6): both token-economics research
  documents are byte-identical to the copies already in the release.
- [PR #7](https://github.com/codeclew/codeclew/pull/7): both syntax-first RFC/plan
  documents are byte-identical to the copies already in the release. Those
  original research commits need not be merged again. Their recorded historical
  commit-metadata limitation is not introduced into main.

## Preserved work

| Work | Durable location | Status |
| --- | --- | --- |
| Pre-consolidation uncommitted checkout | [archive/pre-consolidation-working-copy-20260913](https://github.com/codeclew/codeclew/tree/archive/pre-consolidation-working-copy-20260913), `17ed857` | Exact saved snapshot; early runtime versions are superseded by released implementations. |
| Modular architecture proposal and issue journal | [proposal](../../plans/modular-language-framework-analysis.md), [issues](../../plans/modular-analysis-codeclew-issues.md), adjacent diagram and evidence index | Historical source observations retained in main; not new acceptance requirements. |
| Site design review | [review](site-redesign-20260908.md) | Original review retained with its date and limits. |
| Token/navigation and initial-context experiments | [archive/preloaded-context](https://github.com/codeclew/codeclew/tree/archive/preloaded-context), `55260fc` | Includes all seven java-token-value commits and the later preloading candidate. Runtime changes are optional research, not release code. |
| Token results | [automatic comparison](../../plans/automatic-source-context-results.md), [three criteria](../../plans/token-value-three-criteria.md), [earlier measurements](../../plans/java-kotlin-token-value.md) | Reports retained in main. Automatic preloading increased measured tokens by 24.4% across two questions; no efficiency benefit is claimed. |
| T14 GitLab/documentation CI draft | [archive/deferred-documentation-ci](https://github.com/codeclew/codeclew/tree/archive/deferred-documentation-ci), `cc3ea8e` | Five files committed unchanged. Deferred; not shipped, not wired into CI and not asserted to work on live GitLab. |

Archive branches keep exact source and tests inspectable in a normal Git clone.
Use `git fetch origin` and `git show origin/archive/preloaded-context:path/to/file`
to inspect a candidate without changing the active checkout. To resume a draft,
create a feature branch from current main and selectively port the needed change;
do not merge an entire superseded snapshot over current code.

Local ignored build products and private diagnostics are preserved outside the
active checkout before retiring redundant worktrees. They are not public source
artifacts and are not uploaded. The local preservation manifest records their
original locations and the retired worktree revisions.

## Site and verification

The homepage, service documentation and release-evidence section now describe
0.8.0. Service documentation includes note targets and a link to agent authoring
instructions. Historical navigation and case-study examples retain their original
revision/version labels. Search is regenerated from the updated sections.

Consolidation verification covers site navigation, links, generated documentation,
English content and repository privacy. No runtime rebuild is needed for these
site and research-document changes; the release's completed runtime checks remain
applicable. GitHub's normal main-branch CI and Pages workflow run after publication.

## Next practical work

1. Install 0.8.0 on the work laptop and use the private three-service workbook.
   Bind the local service clones, check freshness, add one service/section note
   and one entity/scenario question, and inspect the preserved originals.
2. Review reservation quantities, idempotency, TTL and failure responses with a
   service owner. Record intended policy separately from observed code, especially
   adjustment/reset behavior and the negative-quantity gap.
3. Complete one useful cross-service flow. Confirm deployed topics and service
   relationships with operator evidence before accepting proposed links; add the
   missing contract and recovery details for that flow.
4. Exercise one small source change locally: observe stale documentation, refresh
   the explanation, and verify that notes and publication history survive.
5. Use the experience to choose the next product slice: easier note authoring or
   more useful entity/scenario coverage. GitLab integration, 40-service capacity
   and renewed token-efficiency experiments remain separate deferred work.
