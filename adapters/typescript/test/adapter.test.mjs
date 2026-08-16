import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import test, { after } from "node:test";
import { fileURLToPath } from "node:url";

const ADAPTER_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const ADAPTER = path.join(ADAPTER_ROOT, "src", "adapter.mjs");
const FIXTURE = path.resolve(ADAPTER_ROOT, "..", "..", "fixtures", "multilang-typescript");
const SHARED_SCHEMA = JSON.parse(
  fs.readFileSync(path.resolve(ADAPTER_ROOT, "..", "..", "schemas", "adapter_output.schema.json"), "utf8"),
);
const TEMP_PROJECTS = new Set();

after(() => {
  for (const directory of TEMP_PROJECTS) {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

function execute(project, extra = []) {
  return spawnSync(process.execPath, [ADAPTER, "--repo", project, ...extra], {
    cwd: ADAPTER_ROOT,
    encoding: "utf8",
  });
}

function canonicalize(value) {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).sort(([left], [right]) => left.localeCompare(right))
        .map(([key, child]) => [key, canonicalize(child)]),
    );
  }
  return value;
}

function canonicalBytes(value) {
  return Buffer.from(JSON.stringify(canonicalize(value)));
}

function digest(value) {
  return `sha256:${crypto.createHash("sha256").update(value).digest("hex")}`;
}

function semanticDigest(bundle) {
  const projection = structuredClone(bundle);
  delete projection.cost;
  delete projection.outputDigest;
  if (projection.impact) delete projection.impact.queryMicros;
  return digest(canonicalBytes(projection));
}

function fileManifest(directory, current = directory, result = {}) {
  for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
    const absolute = path.join(current, entry.name);
    if (entry.isDirectory()) fileManifest(directory, absolute, result);
    else if (entry.isFile()) result[path.relative(directory, absolute)] = digest(fs.readFileSync(absolute));
  }
  return result;
}

function makeProject(tsconfig, source) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "codeclew-ts-adapter-"));
  TEMP_PROJECTS.add(directory);
  fs.writeFileSync(path.join(directory, "tsconfig.json"), `${JSON.stringify(tsconfig, null, 2)}\n`);
  fs.writeFileSync(path.join(directory, "index.ts"), source);
  return directory;
}

test("real strict fixture emits snapshot-bound compiler evidence and conservative boundaries", () => {
  const filesBefore = fileManifest(FIXTURE);
  const first = execute(FIXTURE);
  assert.equal(first.status, 0, first.stderr);
  assert.deepEqual(fileManifest(FIXTURE), filesBefore, "adapter must not mutate the analyzed project");
  const bundle = JSON.parse(first.stdout);
  assert.deepEqual(Object.keys(bundle).sort(), [...SHARED_SCHEMA.required].sort());
  assert.deepEqual(
    Object.keys(bundle.cost).sort(),
    [...SHARED_SCHEMA.$defs.cost.required].sort(),
  );
  assert.deepEqual(
    Object.keys(bundle.snapshotInput).sort(),
    [...SHARED_SCHEMA.$defs.snapshot.required].sort(),
  );
  assert.equal(bundle.schema, "codeclew.adapter-output/0.1");
  assert.equal(bundle.adapter.languageId, "typescript");
  assert.equal(bundle.compilerReceipt.status, "ACCEPTED");
  assert.equal(bundle.compilerReceipt.grade, "COMPILER_CHECKED");
  assert.match(bundle.snapshotInput.repositoryTreeDigest, /^sha256:[a-f0-9]{64}$/);
  assert.match(bundle.outputDigest, /^sha256:[a-f0-9]{64}$/);
  assert.ok(bundle.snapshotInput.sources.length >= 3);
  assert.ok(bundle.snapshotInput.sources.every((source) => /^sha256:[a-f0-9]{64}$/.test(source.contentDigest)));
  assert.deepEqual(
    bundle.snapshotInput.sources.map((source) => source.artifactId),
    [...bundle.snapshotInput.sources.map((source) => source.artifactId)].sort(),
  );
  assert.ok(bundle.snapshotInput.sources.every((source) =>
    !source.normalizedPath.startsWith("/") && !source.normalizedPath.split("/").includes("..")
  ));
  assert.ok(bundle.entities.length > 0);
  assert.ok(bundle.occurrences.some((occurrence) => occurrence.role === "DEFINITION"));
  assert.ok(bundle.occurrences.some((occurrence) => occurrence.role === "REFERENCE"));
  assert.ok(bundle.occurrences.some((occurrence) => occurrence.role === "CALL"));
  assert.ok(bundle.facts.some((fact) => fact.relation === "codeclew.relation/may-call/1"));
  assert.ok(bundle.occurrences.every((occurrence) => /^sha256:[a-f0-9]{64}$/.test(occurrence.occurrenceId)));
  assert.ok(bundle.facts.every((fact) => /^sha256:[a-f0-9]{64}$/.test(fact.factId)));
  assert.ok(bundle.boundaries.every((boundary) => /^sha256:[a-f0-9]{64}$/.test(boundary.boundaryId)));
  assert.equal(bundle.compilerReceipt.snapshotTreeDigest, bundle.snapshotInput.repositoryTreeDigest);
  const boundaryKinds = new Set(bundle.boundaries.map((boundary) => boundary.details.providerKind));
  assert.ok(boundaryKinds.has("ANY_TYPE"));
  assert.ok(boundaryKinds.has("DYNAMIC_ELEMENT_ACCESS"));
  assert.ok(boundaryKinds.has("DYNAMIC_IMPORT"));
  assert.ok(boundaryKinds.has("AMBIENT_DECLARATION"));
  assert.ok(boundaryKinds.has("SOURCE_MAP_DISABLED"));
  assert.ok(boundaryKinds.has("OPEN_WORLD_DISPATCH"));
  const callCapability = bundle.capabilityDescriptors.find(
    (capability) => capability.operationUri === "codeclew.relation/may-call/1",
  );
  assert.equal(callCapability.guaranteedEnumeration, "PARTIAL");
  assert.equal(callCapability.grade, "STATICALLY_APPROXIMATED");
  assert.equal(callCapability.approximation, "HEURISTIC");
  assert.ok(bundle.cost.totalWallMicros > 0);
  assert.equal(bundle.cost.emittedBytes, canonicalBytes(bundle).length);
  assert.equal(bundle.adapter.binaryDigest, digest(fs.readFileSync(ADAPTER)));
  const unsigned = structuredClone(bundle);
  const declaredDigest = unsigned.outputDigest;
  unsigned.outputDigest = "";
  assert.equal(declaredDigest, digest(canonicalBytes(unsigned)));
  assert.equal(bundle.impact.status, "UNKNOWN");
  assert.ok(bundle.impact.mandatoryObligations.length >= bundle.impact.boundaries.length);

  const second = execute(FIXTURE);
  assert.equal(second.status, 0, second.stderr);
  assert.equal(semanticDigest(JSON.parse(second.stdout)), semanticDigest(bundle));

  const seed = bundle.entities.find((entity) => entity.displayName === "formatValue").opaqueId;
  const impactRun = execute(FIXTURE, ["--seed-entity", seed, "--max-depth", "10", "--max-entities", "128"]);
  assert.equal(impactRun.status, 0, impactRun.stderr);
  const impact = JSON.parse(impactRun.stdout).impact;
  assert.equal(impact.status, "PARTIAL_BOUNDARY");
  assert.ok(impact.affected.some((entity) => entity.entityId === seed));
  assert.ok(impact.paths.length > 0);
  assert.ok(impact.mandatoryObligations.some((obligation) => obligation.status === "UNKNOWN"));
});

test("non-strict and selectively weakened strict configurations fail before evidence", () => {
  const nonStrict = makeProject(
    { compilerOptions: { strict: false, noEmit: true }, files: ["index.ts"] },
    "export const value = 1;\n",
  );
  const first = execute(nonStrict);
  assert.equal(first.status, 64);
  assert.equal(first.stdout, "");
  assert.equal(JSON.parse(first.stderr).code, "STRICT_MODE_REQUIRED");

  const weakened = makeProject(
    { compilerOptions: { strict: true, noImplicitAny: false, noEmit: true }, files: ["index.ts"] },
    "export function identity(value) { return value; }\n",
  );
  const second = execute(weakened);
  assert.equal(second.status, 64);
  assert.equal(JSON.parse(second.stderr).code, "STRICT_MODE_WEAKENED");

  const noCheck = makeProject(
    { compilerOptions: { strict: true, noCheck: true, noEmit: true }, files: ["index.ts"] },
    "export const value: string = 1;\n",
  );
  const third = execute(noCheck);
  assert.equal(third.status, 64);
  assert.equal(JSON.parse(third.stderr).code, "TYPE_CHECKING_DISABLED");
});

test("source and compiler configuration mutations change their bound digests", () => {
  const project = makeProject(
    { compilerOptions: { strict: true, noEmit: true, target: "ES2022" }, files: ["index.ts"] },
    "export const value = 1;\n",
  );
  const first = execute(project);
  assert.equal(first.status, 0, first.stderr);
  const before = JSON.parse(first.stdout);

  fs.writeFileSync(path.join(project, "index.ts"), "export const value = 2;\n");
  const sourceChanged = execute(project);
  assert.equal(sourceChanged.status, 0, sourceChanged.stderr);
  const afterSource = JSON.parse(sourceChanged.stdout);
  assert.notEqual(
    afterSource.snapshotInput.repositoryTreeDigest,
    before.snapshotInput.repositoryTreeDigest,
  );

  fs.writeFileSync(
    path.join(project, "tsconfig.json"),
    `${JSON.stringify({
      compilerOptions: { strict: true, noEmit: true, target: "ES2023" },
      files: ["index.ts"],
    }, null, 2)}\n`,
  );
  const configChanged = execute(project);
  assert.equal(configChanged.status, 0, configChanged.stderr);
  const afterConfig = JSON.parse(configChanged.stdout);
  assert.notEqual(
    afterConfig.snapshotInput.buildConfigurationDigest,
    afterSource.snapshotInput.buildConfigurationDigest,
  );
});

test("compiler errors produce a rejected receipt and explicit UNKNOWN boundary", () => {
  const invalid = makeProject(
    { compilerOptions: { strict: true, noEmit: true }, files: ["index.ts"] },
    "export function run(): string { return missingFunction(); }\n",
  );
  const result = execute(invalid);
  assert.equal(result.status, 2, result.stderr);
  const bundle = JSON.parse(result.stdout);
  assert.equal(bundle.compilerReceipt.status, "REJECTED");
  assert.ok(bundle.compilerReceipt.providerPayload.apiDiagnosticCount > 0);
  assert.ok(bundle.compilerReceipt.providerPayload.diagnostics.some((diagnostic) => diagnostic.code === 2304));
  assert.ok(bundle.boundaries.some((boundary) => boundary.details.providerKind === "UNRESOLVED_SYMBOL"));
  assert.ok(bundle.facts.some((fact) => fact.truth === "UNKNOWN"));
  assert.equal(bundle.impact.status, "UNKNOWN");
  assert.ok(bundle.impact.mandatoryObligations.some((obligation) => obligation.status === "VIOLATED"));
  const compilerCapability = bundle.capabilityDescriptors.find(
    (capability) => capability.operationUri === "codeclew.operation/typescript-compiler-check",
  );
  assert.equal(compilerCapability.guaranteedEnumeration, "UNKNOWN");
});
