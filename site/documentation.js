const documentationNodes = {
  admit: {
    step: "01 · Authority",
    authority: "Released contract",
    title: "Admit the exact task authority",
    summary: "Codeclew starts from one installed release, repository, target ref, language and compilation instead of inferring a convenient project boundary.",
    mechanism: "The admission result must report RELEASE runtime authority, the installed launcher and no source fallback.",
    command: "clew context open --repo … --target-ref … --language … --compilation …",
    excerpt: "Require runtimeMode=RELEASE, launcherAuthority=INSTALLED_RELEASE, and sourceFallbackAllowed=false.",
    code: `{
  "runtimeMode": "RELEASE",
  "launcherAuthority": "INSTALLED_RELEASE",
  "sourceFallbackAllowed": false
}`,
    kind: "Public agent contract",
    certainty: "DECLARED · EXACT REVISION",
    source: "https://github.com/codeclew/codeclew/blob/6281138ecbf73bc5de1a9c7eaeb2cdf7009e6ca1/.agents/skills/codeclew/SKILL.md#L20-L43",
    limit: "This proves the released task-admission contract. It does not prove that every repository is supported or that a later analysis is complete."
  },
  navigate: {
    step: "02 · Discover",
    authority: "Released contract",
    title: "Return a bounded decision surface",
    summary: "One navigation request admits the task, opens a managed session and returns at most three fact-bound decision cards.",
    mechanism: "The compact response retains sessionId, contextId and evidenceDigest. Alternatives receive attested previews instead of complete retransmission.",
    command: "clew nav query --term <identifier> --source",
    excerpt: "nav query … returns at most three fact-bound decision cards with exact one-line source previews in one command.",
    code: `{
  "candidateCount": { "returned": 3, "available": 9 },
  "decisionSource": { "authority": "EXACT_SNAPSHOT_TEXT" },
  "declarationIdentity": { "authority": "K2_FIR" },
  "completeness": "PARTIAL"
}`,
    kind: "README public behavior",
    certainty: "DECLARED · BOUNDED",
    source: "https://github.com/codeclew/codeclew/blob/6281138ecbf73bc5de1a9c7eaeb2cdf7009e6ca1/README.md#L72-L95",
    limit: "Search terms retrieve candidates; they do not prove that the first card is the user's intended symbol or that omitted matches are absent."
  },
  select: {
    step: "03 · Decide",
    authority: "Released contract",
    title: "Select one declaration or fail closed",
    summary: "When the identifier and file are known, Codeclew selects one exact declaration and source window instead of returning every textual match.",
    mechanism: "No match and same-file overloads remain typed SYMBOL_NOT_FOUND or AMBIGUOUS_SYMBOL outcomes.",
    command: "clew nav expand --term <exact-id> --file <relative-file> --source",
    excerpt: "This succeeds only for one exact declaration. No match and same-file overloads remain typed results.",
    code: `val row = QUALIFIED_KOTLIN_ENGINE_ROWS.singleOrNull {
    it.projectCompilerVersion == project.projectCompilerVersion &&
        it.engineCompilerVersion == engine.analyzerCompilerVersion
} ?: return KotlinEngineCompatibilityDecision(
    status = "REJECTED",
    kind = "UNQUALIFIED",
    reason = "PROJECT_ENGINE_ROW_NOT_QUALIFIED",
    btaEligible = false,
)`,
    kind: "Kotlin implementation",
    certainty: "TESTED · FAIL-CLOSED",
    source: "https://github.com/codeclew/codeclew/blob/6281138ecbf73bc5de1a9c7eaeb2cdf7009e6ca1/workers/kotlin/src/main/kotlin/dev/semanticthread/worker/Worker.kt#L114-L174",
    limit: "An exact source window proves visible source facts about that declaration; syntax-only selection does not establish runtime reachability."
  },
  document: {
    step: "04 · Explain",
    authority: "Agent contract",
    title: "Keep claims and obligations separate",
    summary: "Every requested item must be supported by evidence or reported as conditional or unproven with the missing evidence named.",
    mechanism: "The agent records predicates, typed outcomes, qualifiers and mutation order before compressing the final explanation.",
    command: "PROVEN → DRAFTED → EMITTED   |   UNPROVEN → obligation",
    excerpt: "Every explicitly requested item must be either supported by cited returned evidence … or reported as conditional/unproven.",
    code: `{
  "fact": "project/engine row is qualified",
  "certainty": "PROVEN",
  "sourceDigest": "sha256:b4a5bdd0…",
  "obligations": []
}`,
    kind: "Public agent contract",
    certainty: "DECLARED · PER CLAIM",
    source: "https://github.com/codeclew/codeclew/blob/6281138ecbf73bc5de1a9c7eaeb2cdf7009e6ca1/.agents/skills/codeclew/SKILL.md#L245-L267",
    limit: "This is a disciplined reporting contract. It reduces silent overclaim but does not make the underlying evidence stronger."
  },
  prepare: {
    step: "05 · Isolate",
    authority: "Mutation contract",
    title: "Prepare one closed change in isolation",
    summary: "A mutation reuses the admitted session and context, validates a closed immutable edit plan and waits for an actionable run state.",
    mechanism: "Freshness is checked immediately before prepare. Status exposes the bounded diff, validations, authority digest and obligations.",
    command: "clew change check-freshness …\nclew change prepare …\nclew change status …",
    excerpt: "Create a closed immutable edit plan, and use change prepare. The high-level prepare call waits for the first actionable run state.",
    code: `clew change check-freshness --session "$SESSION_ID"
clew change prepare \\
  --session "$SESSION_ID" \\
  --context "$CONTEXT_ID" \\
  --plan change-plan.json
clew change status --session "$SESSION_ID" --run "$RUN_ID"`,
    kind: "Public mutation contract",
    certainty: "DECLARED · TRANSACTIONAL",
    source: "https://github.com/codeclew/codeclew/blob/6281138ecbf73bc5de1a9c7eaeb2cdf7009e6ca1/.agents/skills/codeclew/SKILL.md#L286-L299",
    limit: "A prepared candidate is not published work. Native project tests and every returned obligation still require review."
  },
  publish: {
    step: "06 · Mutate",
    authority: "Mutation contract",
    title: "Publish only after fresh approval",
    summary: "The target ref changes only through change publish after another freshness check and explicit user approval.",
    mechanism: "Conditional publication additionally binds the prepared authority digest and requires acknowledgement of every obligation.",
    command: "clew change check-freshness …\nclew change publish --run …",
    excerpt: "Publish only after the user explicitly approves the reviewed candidate.",
    code: `clew change check-freshness --session "$SESSION_ID"
clew change publish \\
  --session "$SESSION_ID" \\
  --run "$RUN_ID"`,
    kind: "Public mutation contract",
    certainty: "DECLARED · EXPLICIT APPROVAL",
    source: "https://github.com/codeclew/codeclew/blob/6281138ecbf73bc5de1a9c7eaeb2cdf7009e6ca1/.agents/skills/codeclew/SKILL.md#L295-L304",
    limit: "The contract constrains publication authority. It does not claim that tests are sufficient or that an approved change is defect-free."
  },
  recover: {
    step: "07 · Exception",
    authority: "Recovery contract",
    title: "Preserve work when authority changes",
    summary: "Freshness outcomes are binding: continue on FRESH, preserve work on DIRTY, rebuild authority on STALE and repair access on UNAVAILABLE.",
    mechanism: "WORKTREE_RECOVERY_REQUIRED uses the bound session and run. A stale target opens a new session instead of replaying an old plan.",
    command: "clew change recover --session … --run …",
    excerpt: "Never clean, reset, rebase, or replay user work to make a result fresh.",
    code: `when (freshness) {
  FRESH       -> continueRun()
  DIRTY       -> preserveDeveloperWork()
  STALE       -> openNewSession()
  UNAVAILABLE -> repairAccessOrStop()
}`,
    kind: "Public recovery contract",
    certainty: "DECLARED · FAIL-CLOSED",
    source: "https://github.com/codeclew/codeclew/blob/6281138ecbf73bc5de1a9c7eaeb2cdf7009e6ca1/.agents/skills/codeclew/SKILL.md#L312-L334",
    limit: "This documents the required response to detected authority changes; it is not evidence that every external Git failure is recoverable."
  }
};

const workbench = document.querySelector(".docs-workbench");
const nodeButtons = [...document.querySelectorAll("[data-doc-node]")];
const levelButtons = [...document.querySelectorAll("[data-doc-level]")];

function renderNode(id) {
  const node = documentationNodes[id];
  if (!node) return;
  document.querySelector("#node-step").textContent = node.step;
  document.querySelector("#node-authority").textContent = node.authority;
  document.querySelector("#node-title").textContent = node.title;
  document.querySelector("#node-summary").textContent = node.summary;
  document.querySelector("#node-mechanism").textContent = node.mechanism;
  document.querySelector("#node-command").textContent = node.command;
  document.querySelector("#node-excerpt").textContent = node.excerpt;
  document.querySelector("#node-code").textContent = node.code;
  document.querySelector("#node-kind").textContent = node.kind;
  document.querySelector("#node-certainty").textContent = node.certainty;
  document.querySelector("#node-source").href = node.source;
  document.querySelector("#node-limit").textContent = node.limit;
  nodeButtons.forEach((button) => {
    const selected = button.dataset.docNode === id;
    button.classList.toggle("is-active", selected);
    button.setAttribute("aria-pressed", String(selected));
  });
}

nodeButtons.forEach((button) => button.addEventListener("click", () => {
  renderNode(button.dataset.docNode);
  if (window.matchMedia("(max-width: 820px)").matches) {
    document.querySelector(".evidence-inspector")?.scrollIntoView({ block: "start" });
  }
}));

levelButtons.forEach((button) => {
  button.addEventListener("click", () => {
    workbench.dataset.docDetail = button.dataset.docLevel;
    levelButtons.forEach((candidate) => candidate.setAttribute("aria-pressed", String(candidate === button)));
  });
});

const dotToggle = document.querySelector("#toggle-dot");
const dotCode = document.querySelector("#dot-code");
dotToggle?.addEventListener("click", () => {
  const expanded = dotToggle.getAttribute("aria-expanded") === "true";
  dotToggle.setAttribute("aria-expanded", String(!expanded));
  dotToggle.textContent = expanded ? "Show" : "Hide";
  dotCode.hidden = expanded;
});
