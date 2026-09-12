# Site redesign review

final result: passed

The supplied references establish the warm paper background, navy typography,
pastel cards, blue thread and curious pixel cat. Existing documentation and its
pinned evidence determine the content. The mascot and favicon are original SVG
paths; they contain no embedded raster images.

## Visual comparison

- Source visual: `/tmp/codeclew-site-design-qa/reference.png`, the second supplied
  reference, 1122 × 1402 pixels.
- Implementation: `/tmp/codeclew-site-design-qa/nav-query-desktop.jpg` and
  `/tmp/codeclew-site-design-qa/home-desktop.jpg`.
- Desktop CSS viewport: 1122 × 1402. Browser capture returns the visible
  1122 × 1258 region at one pixel per CSS pixel. The upper regions were compared
  at equal width; the reference's lower footer is outside this capture.
- State: navigation walkthrough with `claim-admit` selected; overview. The
  supporting-code disclosure was also inspected open during the comparison.
- Source and rendered screenshots were displayed together. The shared header,
  hero, sidebar, selected pipeline card and evidence panel were compared.
- Mobile capture: `/tmp/codeclew-site-design-qa/home-mobile.jpg`, with a
  390 × 844 CSS viewport. No mobile reference was supplied.

## Findings and fixes

1. P2: The documentation example's wrapping arrows placed cards in narrow grid
   tracks. Replaced that layout with numbered cards in equal-width columns.
   The revised browser capture showed readable cards and working node selection.
2. P2: The saved-edits command and documentation workbench caused document-level
   horizontal overflow at 390 pixels. Bounded code scrolling and zero-minimum
   grid tracks fixed both. Browser measurements confirmed document width equals
   viewport width after the fixes.
3. P2: White labels on the initial primary blue had a contrast ratio of 3.47:1.
   The final blue has 4.84:1 contrast. Mobile search has a 44-pixel target height.

Final captures include these fixes. No unresolved P0, P1 or P2 findings remain
within the requested redesign scope. The vector illustration intentionally uses
fewer color patches than the raster reference. The real source excerpts make
the walkthrough longer than the illustrative reference page.

## Functional verification

- All eight pages opened at desktop and mobile widths. No broken image loads or
  browser console errors were observed in the desktop route sweep.
- Every page is reachable from both the shared primary menu and footer;
  documentation pages also have the complete sidebar.
- Search covers eight pages and 32 section destinations. Search, an empty result,
  Enter navigation, the command-key shortcut, Escape and focus return were checked.
- The mobile Explore menu opened and navigated to the saved-edits example.
- Documentation node selection and detail-level switching updated the inspector.
- The navigation graph's ABSTAIN branch and supporting-code toggle worked.
- The saved-edits report filtered declarations and displayed exact source for
  the selected connected consumer.
- Five static site tests passed: local links and assets, complete menus, unique
  fragment IDs, search destinations, and actual SVG path assets.
- The existing validator passed for all nine pinned navigation claims.
- The embedded saved-edits evidence is unchanged from the original report.
- JavaScript syntax and English-content checks for the changed site files passed.
  The final repository-wide English-content check reports Cyrillic text in the
  concurrently added, unrelated
  `workers/kotlin/src/test/kotlin/dev/semanticthread/worker/KotlinDocumentationFlowTest.kt:23`.
  That file was left untouched.

## Maintenance and scope

`scripts/build_site_navigation.py` owns the shared navigation and search index.
Its `--check` mode and `scripts/test_site.py` run in the GitHub Pages workflow.
Navigation is rendered into static HTML, so it remains available without
JavaScript. Search and interactive inspectors progressively enhance the pages.

The preview is local. Deployment and the unrelated runtime test suite were not
part of this verification. Cross-browser checks beyond the in-app Chromium
browser remain unverified.

## Mascot refinement

The two supplied miniature crops were traced directly into SVG paths, preserving
their silhouettes, markings and expressions. The portrait is used in the brand
and square favicon; the standing cat is used beside the sidebar callout text.
The large mascot now follows the first reference's fuller adult proportions,
with a continuous natural back and two paws resting on the electric-blue yarn.
Its generated artwork was traced to a standalone SVG and inserted as paths in
the existing scene. No raster image is embedded in any shipped mascot SVG.

The revised hero, brand and sidebar callout were inspected together in the
browser. Navigation generation and the five static site checks passed after
the asset update. Layout, page content and navigation destinations are unchanged.
