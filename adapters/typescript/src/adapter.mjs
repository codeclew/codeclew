#!/usr/bin/env node

import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { performance } from "node:perf_hooks";
import { createRequire } from "node:module";
import { spawnSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const ADAPTER_SCHEMA = "codeclew.adapter-output/0.1";
const ADAPTER_ID = "codeclew.typescript-compiler-api";
const ADAPTER_VERSION = "0.1.0";
const LANGUAGE_ID = "typescript";
const OPERATION_VERSION = "1";
const REFERENCE_RELATION = "codeclew.relation/may-reference/1";
const CALL_RELATION = "codeclew.relation/may-call/1";
const THIS_FILE = fileURLToPath(import.meta.url);
const ADAPTER_ROOT = path.resolve(path.dirname(THIS_FILE), "..");
const require = createRequire(import.meta.url);
const TYPESCRIPT_API_FILE = require.resolve("typescript");
const TSC_FILE = require.resolve("typescript/lib/tsc.js");
const TYPESCRIPT_PACKAGE_ROOT = path.dirname(require.resolve("typescript/package.json"));
const SOURCE_BYTES = new WeakMap();

const ProofGrade = Object.freeze({
  NAVIGATION: "NAVIGATION",
  COMPILER_RESOLVED: "COMPILER_RESOLVED",
  COMPILER_CHECKED: "COMPILER_CHECKED",
  STATICALLY_APPROXIMATED: "STATICALLY_APPROXIMATED",
});

const Enumeration = Object.freeze({
  COMPLETE_IN_SCOPE: "COMPLETE_IN_SCOPE",
  PARTIAL: "PARTIAL",
  UNKNOWN: "UNKNOWN",
});

const Consequence = Object.freeze({
  LOCAL_ONLY: "LOCAL_ONLY",
  ENUMERATION_INCOMPLETE: "ENUMERATION_INCOMPLETE",
  PROOF_INVALID: "PROOF_INVALID",
});

function canonicalize(value) {
  if (Array.isArray(value)) {
    return value.map(canonicalize);
  }
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value)
        .filter(([, child]) => child !== undefined)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, child]) => [key, canonicalize(child)]),
    );
  }
  if (typeof value === "number" && !Number.isFinite(value)) {
    throw new Error("canonical JSON cannot contain a non-finite number");
  }
  return value;
}

export function canonicalStringify(value, pretty = false) {
  return JSON.stringify(canonicalize(value), null, pretty ? 2 : undefined);
}

function sha256Bytes(value) {
  return `sha256:${crypto.createHash("sha256").update(value).digest("hex")}`;
}

function sha256Canonical(value) {
  return sha256Bytes(canonicalStringify(value));
}

function digestFile(file) {
  return sha256Bytes(fs.readFileSync(file));
}

function sourceArtifact(sourceFile) {
  const cached = SOURCE_BYTES.get(sourceFile);
  if (cached) return cached;
  const bytes = fs.readFileSync(sourceFile.fileName);
  const utf8Bom = bytes.length >= 3 && bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf;
  const sourceRetainsBom = sourceFile.text.charCodeAt(0) === 0xfeff;
  const value = {
    bytes,
    contentDigest: sha256Bytes(bytes),
    compilerTextByteOffset: utf8Bom && !sourceRetainsBom ? 3 : 0,
  };
  SOURCE_BYTES.set(sourceFile, value);
  return value;
}

function toPosix(value) {
  return value.split(path.sep).join("/");
}

function relativePath(projectDirectory, file) {
  const resolved = path.resolve(file);
  const relative = toPosix(path.relative(projectDirectory, resolved));
  if (relative && !relative.startsWith("../")) {
    return relative;
  }
  const normalized = toPosix(resolved);
  const nodeModulesMarker = "/node_modules/";
  const nodeModulesIndex = normalized.lastIndexOf(nodeModulesMarker);
  if (nodeModulesIndex >= 0) {
    return `external/node_modules/${normalized.slice(nodeModulesIndex + nodeModulesMarker.length)}`;
  }
  let identity = sha256Bytes(path.basename(file));
  try {
    if (fs.statSync(resolved).isFile()) identity = digestFile(resolved);
  } catch {
    // A missing external path remains explicit in dependency/configuration evidence.
  }
  return `external/source/${path.basename(file)}-${identity.slice(7, 19)}`;
}

function normalizeQualifiedName(value, projectDirectory) {
  const normalizedProject = toPosix(path.resolve(projectDirectory));
  return toPosix(value)
    .split(normalizedProject).join(".")
    .replace(/"[^"]*\/node_modules\//g, '"external/node_modules/')
    .replace(/"\/[^\"]+"/g, (match) => `"external/source/${path.posix.basename(match.slice(1, -1))}"`);
}

function stableSort(items, key) {
  return items.sort((left, right) => key(left).localeCompare(key(right)));
}

function parseArguments(argv) {
  const result = {
    output: "-",
    maxDepth: 2,
    maxEntities: 128,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--project") {
      result.project = argv[++index];
    } else if (argument === "--repo") {
      result.project = argv[++index];
    } else if (argument === "--seed-entity") {
      result.seedEntity = argv[++index];
    } else if (argument === "--max-depth") {
      result.maxDepth = Number.parseInt(argv[++index], 10);
    } else if (argument === "--max-entities") {
      result.maxEntities = Number.parseInt(argv[++index], 10);
    } else if (argument === "--output") {
      result.output = argv[++index];
    } else if (argument === "--help" || argument === "-h") {
      result.help = true;
    } else {
      throw new AdapterError("INVALID_ARGUMENT", `unknown argument: ${argument}`, 64);
    }
  }
  if (!result.help && !result.project) {
    throw new AdapterError("INVALID_ARGUMENT", "--repo (or --project) is required", 64);
  }
  if (
    !Number.isSafeInteger(result.maxDepth) || result.maxDepth < 0 ||
    !Number.isSafeInteger(result.maxEntities) || result.maxEntities < 1
  ) {
    throw new AdapterError(
      "INVALID_ARGUMENT",
      "--max-depth must be a non-negative integer and --max-entities a positive integer",
      64,
    );
  }
  return result;
}

function usage() {
  return [
    "Usage: adapter.mjs --repo <directory|tsconfig.json> [--seed-entity <opaque-id>] [--max-depth N] [--max-entities N] [--output <file|->]",
    "",
    "The project must enable TypeScript strict mode without explicitly disabling a strict sub-option.",
  ].join("\n");
}

class AdapterError extends Error {
  constructor(code, message, exitCode = 1, details = {}) {
    super(message);
    this.name = "AdapterError";
    this.code = code;
    this.exitCode = exitCode;
    this.details = details;
  }
}

function resolveProject(projectArgument) {
  const resolved = path.resolve(projectArgument);
  const stat = fs.existsSync(resolved) ? fs.statSync(resolved) : null;
  const configFile = stat?.isDirectory()
    ? ts.findConfigFile(resolved, ts.sys.fileExists, "tsconfig.json")
    : resolved;
  if (!configFile || !fs.existsSync(configFile)) {
    throw new AdapterError(
      "TSCONFIG_NOT_FOUND",
      `no tsconfig.json found for ${resolved}`,
      64,
    );
  }
  return {
    configFile: path.resolve(configFile),
    projectDirectory: path.dirname(path.resolve(configFile)),
  };
}

const STRICT_SUB_OPTIONS = [
  "alwaysStrict",
  "noImplicitAny",
  "noImplicitThis",
  "strictBindCallApply",
  "strictBuiltinIteratorReturn",
  "strictFunctionTypes",
  "strictNullChecks",
  "strictPropertyInitialization",
  "useUnknownInCatchVariables",
];

function readStrictConfiguration(configFile) {
  const loaded = ts.readConfigFile(configFile, ts.sys.readFile);
  if (loaded.error) {
    throw new AdapterError(
      "TSCONFIG_INVALID",
      formatDiagnostic(loaded.error, path.dirname(configFile)),
      64,
    );
  }
  const parsed = ts.parseJsonConfigFileContent(
    loaded.config,
    ts.sys,
    path.dirname(configFile),
    undefined,
    configFile,
  );
  if (parsed.errors.length > 0) {
    throw new AdapterError(
      "TSCONFIG_INVALID",
      parsed.errors.map((diagnostic) => formatDiagnostic(diagnostic, path.dirname(configFile))).join("\n"),
      64,
    );
  }
  if (parsed.options.strict !== true) {
    throw new AdapterError(
      "STRICT_MODE_REQUIRED",
      "compilerOptions.strict must be true",
      64,
    );
  }
  const compilerStrictOptions = Array.isArray(ts.optionDeclarations)
    ? ts.optionDeclarations.filter((option) => option.strictFlag === true).map((option) => option.name)
    : [];
  const disabled = [...new Set([...STRICT_SUB_OPTIONS, ...compilerStrictOptions])]
    .filter((option) => parsed.options[option] === false)
    .sort();
  if (disabled.length > 0) {
    throw new AdapterError(
      "STRICT_MODE_WEAKENED",
      `strict sub-options explicitly disabled: ${disabled.join(", ")}`,
      64,
      { disabled },
    );
  }
  if (parsed.options.noCheck === true) {
    throw new AdapterError(
      "TYPE_CHECKING_DISABLED",
      "compilerOptions.noCheck must not be true for compiler-checked evidence",
      64,
    );
  }
  if (parsed.fileNames.length === 0) {
    throw new AdapterError("EMPTY_PROJECT", "tsconfig contains no source files", 64);
  }
  return { loaded: loaded.config, parsed };
}

function normalizeCompilerValue(value, projectDirectory) {
  if (Array.isArray(value)) {
    return value.map((child) => normalizeCompilerValue(child, projectDirectory));
  }
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([key, child]) => [
        key,
        normalizeCompilerValue(child, projectDirectory),
      ]),
    );
  }
  if (typeof value === "string" && path.isAbsolute(value)) {
    return relativePath(projectDirectory, value);
  }
  return value;
}

function configuredProgramSources(program, projectDirectory) {
  const roots = new Set(program.getRootFileNames().map((file) => path.resolve(file)));
  return stableSort(
    program
      .getSourceFiles()
      .filter((source) => !program.isSourceFileDefaultLibrary(source))
      .filter((source) => !source.fileName.includes(`${path.sep}node_modules${path.sep}`))
      .filter((source) => {
        const relative = path.relative(projectDirectory, source.fileName);
        return roots.has(path.resolve(source.fileName)) ||
          relative === "" ||
          (!relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative));
      }),
    (source) => relativePath(projectDirectory, source.fileName),
  );
}

function sourceOrigin(file, projectDirectory) {
  const relative = relativePath(projectDirectory, file).toLowerCase();
  if (relative.startsWith("external/")) return "EXTERNAL";
  if (relative.includes("/generated/") || /(?:\.generated|\.g)\.[cm]?tsx?$/.test(relative)) {
    return "GENERATED";
  }
  if (relative.includes("/vendor/") || relative.includes("/vendored/")) return "VENDORED";
  return "USER";
}

const SNAPSHOT_IGNORED_DIRECTORIES = new Set([
  ".git",
  ".gradle",
  ".idea",
  ".semantic-thread",
  ".vscode",
  "build",
  "node_modules",
  "target",
]);

function snapshotRepositorySources(projectDirectory, programSources) {
  const byArtifact = new Map();
  const excludedSymlinks = [];
  function addFile(file, normalizedPath) {
    const bytes = fs.readFileSync(file);
    const source = {
      artifactId: `source:${normalizedPath}`,
      normalizedPath,
      contentDigest: sha256Bytes(bytes),
      sizeBytes: bytes.length,
      origin: sourceOrigin(file, projectDirectory),
    };
    byArtifact.set(source.artifactId, source);
  }
  function visit(directory) {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })
      .sort((left, right) => left.name.localeCompare(right.name))) {
      if (entry.isDirectory() && SNAPSHOT_IGNORED_DIRECTORIES.has(entry.name)) continue;
      const absolute = path.join(directory, entry.name);
      const normalizedPath = relativePath(projectDirectory, absolute);
      if (entry.isSymbolicLink()) {
        excludedSymlinks.push(normalizedPath);
      } else if (entry.isDirectory()) {
        visit(absolute);
      } else if (entry.isFile()) {
        addFile(absolute, normalizedPath);
      }
    }
  }
  visit(projectDirectory);
  for (const sourceFile of programSources) {
    const normalizedPath = relativePath(projectDirectory, sourceFile.fileName);
    if (!byArtifact.has(`source:${normalizedPath}`)) {
      const artifact = sourceArtifact(sourceFile);
      byArtifact.set(`source:${normalizedPath}`, {
        artifactId: `source:${normalizedPath}`,
        normalizedPath,
        contentDigest: artifact.contentDigest,
        sizeBytes: artifact.bytes.length,
        origin: sourceOrigin(sourceFile.fileName, projectDirectory),
      });
    }
  }
  return {
    sources: [...byArtifact.values()].sort((left, right) => left.artifactId.localeCompare(right.artifactId)),
    excludedSymlinks: excludedSymlinks.sort(),
  };
}

function getGitIdentity(projectDirectory) {
  const top = spawnSync("git", ["-C", projectDirectory, "rev-parse", "--show-toplevel"], {
    encoding: "utf8",
  });
  if (top.status !== 0) {
    return { vcsRevision: null, dirty: null, repositoryRoot: null };
  }
  const repositoryRoot = top.stdout.trim();
  const revision = spawnSync("git", ["-C", repositoryRoot, "rev-parse", "HEAD"], {
    encoding: "utf8",
  });
  const relative = path.relative(repositoryRoot, projectDirectory) || ".";
  const status = spawnSync(
    "git",
    ["-C", repositoryRoot, "status", "--porcelain=v1", "--untracked-files=all", "--", relative],
    { encoding: "utf8" },
  );
  return {
    vcsRevision: revision.status === 0 ? revision.stdout.trim() : null,
    dirty: status.status === 0 ? status.stdout.trim().length > 0 : null,
    repositoryRoot,
  };
}

function fileLocation(sourceFile, start, end, projectDirectory) {
  const begin = sourceFile.getLineAndCharacterOfPosition(start);
  const finish = sourceFile.getLineAndCharacterOfPosition(end);
  const artifact = sourceArtifact(sourceFile);
  const startByte = artifact.compilerTextByteOffset + Buffer.byteLength(sourceFile.text.slice(0, start), "utf8");
  const endByte = startByte + Buffer.byteLength(sourceFile.text.slice(start, end), "utf8");
  const normalizedPath = relativePath(projectDirectory, sourceFile.fileName);
  return {
    artifactId: `source:${normalizedPath}`,
    artifactContentDigest: artifact.contentDigest,
    startByte,
    endByte,
    providerCoordinates: {
      schema: "codeclew.typescript-coordinate/0.1",
      normalizedPath,
      coordinateSystem: "UTF16_ZERO_BASED_OFFSETS_AND_ONE_BASED_LINES_COLUMNS",
      startUtf16: start,
      endUtf16: end,
      startLine: begin.line + 1,
      startColumn: begin.character + 1,
      endLine: finish.line + 1,
      endColumn: finish.character + 1,
    },
  };
}

function diagnosticLocation(diagnostic, projectDirectory) {
  if (!diagnostic.file || diagnostic.start === undefined) return null;
  return fileLocation(
    diagnostic.file,
    diagnostic.start,
    diagnostic.start + (diagnostic.length ?? 0),
    projectDirectory,
  );
}

function formatDiagnostic(diagnostic, projectDirectory) {
  const message = ts.flattenDiagnosticMessageText(diagnostic.messageText, "\n");
  const location = diagnosticLocation(diagnostic, projectDirectory);
  const coordinates = location?.providerCoordinates;
  return coordinates
    ? `${coordinates.normalizedPath}:${coordinates.startLine}:${coordinates.startColumn}: TS${diagnostic.code}: ${message}`
    : `TS${diagnostic.code}: ${message}`;
}

function categoryName(category) {
  return ts.DiagnosticCategory[category]?.toUpperCase() ?? "UNKNOWN";
}

function buildDependencyEdges(sourceFiles, compilerOptions, projectDirectory, boundary) {
  const edges = [];
  for (const sourceFile of sourceFiles) {
    const moduleSpecifiers = [];
    function visit(node) {
      if (
        (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) &&
        node.moduleSpecifier &&
        ts.isStringLiteralLike(node.moduleSpecifier)
      ) {
        moduleSpecifiers.push(node.moduleSpecifier.text);
      } else if (
        ts.isCallExpression(node) &&
        node.expression.kind === ts.SyntaxKind.ImportKeyword &&
        node.arguments.length === 1
      ) {
        const argument = node.arguments[0];
        if (ts.isStringLiteralLike(argument)) {
          moduleSpecifiers.push(argument.text);
        } else {
          boundary(
            "DYNAMIC_IMPORT",
            sourceFile,
            argument.getStart(sourceFile),
            argument.getEnd(),
            Consequence.ENUMERATION_INCOMPLETE,
            "A non-literal import target cannot be enumerated statically.",
            [REFERENCE_RELATION, CALL_RELATION],
          );
        }
      }
      ts.forEachChild(node, visit);
    }
    visit(sourceFile);
    for (const specifier of [...new Set(moduleSpecifiers)].sort()) {
      const resolution = ts.resolveModuleName(specifier, sourceFile.fileName, compilerOptions, ts.sys);
      const resolved = resolution.resolvedModule?.resolvedFileName;
      edges.push({
        from: relativePath(projectDirectory, sourceFile.fileName),
        specifier,
        resolved: resolved ? relativePath(projectDirectory, resolved) : null,
        resolvedContentDigest: resolved && fs.existsSync(resolved) ? digestFile(resolved) : null,
        externalLibraryImport: resolution.resolvedModule?.isExternalLibraryImport ?? false,
      });
      if (!resolved) {
        boundary(
          "UNRESOLVED_MODULE",
          sourceFile,
          0,
          0,
          Consequence.ENUMERATION_INCOMPLETE,
          `Module '${specifier}' was not resolved by the TypeScript compiler.`,
          [REFERENCE_RELATION, CALL_RELATION],
        );
      }
    }
  }
  return stableSort(edges, (edge) => `${edge.from}\u0000${edge.specifier}`);
}

function symbolKind(symbol) {
  const flags = symbol.flags;
  if (flags & (ts.SymbolFlags.Class | ts.SymbolFlags.Interface | ts.SymbolFlags.TypeAlias)) return "TYPE_LIKE";
  if (flags & (ts.SymbolFlags.Function | ts.SymbolFlags.Method)) return "CALLABLE";
  if (flags & ts.SymbolFlags.Property) return "FIELD_LIKE";
  if (flags & (ts.SymbolFlags.Parameter | ts.SymbolFlags.Variable)) return "VALUE_LIKE";
  if (flags & ts.SymbolFlags.Module) return "MODULE";
  return "VALUE_LIKE";
}

function declarationNameNode(node) {
  return node && "name" in node && node.name ? node.name : null;
}

function isDeclarationName(node) {
  const parent = node.parent;
  if (!parent) return false;
  return declarationNameNode(parent) === node ||
    (ts.isImportSpecifier(parent) && (parent.name === node || parent.propertyName === node)) ||
    (ts.isExportSpecifier(parent) && (parent.name === node || parent.propertyName === node)) ||
    (ts.isBindingElement(parent) && (parent.name === node || parent.propertyName === node));
}

function declarationOccurrenceRole(node, sourceFile) {
  const parent = node.parent;
  if (
    ts.isImportSpecifier(parent) ||
    (ts.isImportClause(parent) && parent.name === node) ||
    ts.isNamespaceImport(parent) ||
    (ts.isImportEqualsDeclaration(parent) && parent.name === node)
  ) {
    return "IMPORT";
  }
  if (ts.isExportSpecifier(parent)) return "EXPORT";
  if (sourceFile.isDeclarationFile) return "DECLARATION";
  const modifiers = ts.canHaveModifiers(parent) ? ts.getModifiers(parent) : undefined;
  return modifiers?.some((modifier) => modifier.kind === ts.SyntaxKind.DeclareKeyword)
    ? "DECLARATION"
    : "DEFINITION";
}

function isNonReferenceIdentifier(node) {
  const parent = node.parent;
  return !parent ||
    (ts.isPropertyAccessExpression(parent) && parent.name === node) === false &&
      ((ts.isPropertyAssignment(parent) && parent.name === node && !parent.questionToken) ||
        (ts.isMethodDeclaration(parent) && parent.name === node) ||
        (ts.isPropertyDeclaration(parent) && parent.name === node) ||
        (ts.isPropertySignature(parent) && parent.name === node) ||
        (ts.isMethodSignature(parent) && parent.name === node) ||
        (ts.isLabeledStatement(parent) && parent.label === node) ||
        (ts.isBreakOrContinueStatement(parent) && parent.label === node));
}

function isWriteExpression(node) {
  const parent = node.parent;
  if (!parent) return false;
  if (ts.isBinaryExpression(parent) && parent.left === node) {
    return parent.operatorToken.kind >= ts.SyntaxKind.FirstAssignment &&
      parent.operatorToken.kind <= ts.SyntaxKind.LastAssignment;
  }
  return (ts.isPrefixUnaryExpression(parent) || ts.isPostfixUnaryExpression(parent)) &&
    (parent.operator === ts.SyntaxKind.PlusPlusToken || parent.operator === ts.SyntaxKind.MinusMinusToken);
}

function isCompoundWrite(node) {
  const parent = node.parent;
  if (ts.isBinaryExpression(parent) && parent.left === node) {
    return parent.operatorToken.kind !== ts.SyntaxKind.EqualsToken;
  }
  return ts.isPrefixUnaryExpression(parent) || ts.isPostfixUnaryExpression(parent);
}

function symbolAt(checker, node) {
  const original = checker.getSymbolAtLocation(node);
  if (!original) return null;
  if (original.flags & ts.SymbolFlags.Alias) {
    try {
      return checker.getAliasedSymbol(original) ?? original;
    } catch {
      return original;
    }
  }
  return original;
}

function buildSemanticEvidence(program, sourceFiles, projectDirectory, boundary) {
  const checker = program.getTypeChecker();
  const analyzedSourcePaths = new Set(sourceFiles.map((source) => path.resolve(source.fileName)));
  const entities = new Map();
  const symbolIds = new Map();
  const occurrences = [];
  const facts = new Map();

  function declarationKey(declaration) {
    const source = declaration.getSourceFile();
    return `${relativePath(projectDirectory, source.fileName)}:${declaration.getStart(source)}:${declaration.getEnd()}`;
  }

  function ensureEntity(symbol, fallbackNode = null) {
    if (!symbol) return null;
    if (symbolIds.has(symbol)) return symbolIds.get(symbol);
    const declarations = stableSort([...(symbol.declarations ?? [])], declarationKey);
    const declaration = declarations[0] ?? fallbackNode;
    const declarationKeys = declarations.map(declarationKey);
    const qualifiedName = normalizeQualifiedName((() => {
      try {
        return checker.getFullyQualifiedName(symbol);
      } catch {
        return symbol.getName();
      }
    })(), projectDirectory);
    const identityMaterial = {
      language: LANGUAGE_ID,
      qualifiedName,
      declarations: declarationKeys,
      symbolFlags: symbol.flags,
    };
    const entityId = `ts:${sha256Canonical(identityMaterial)}`;
    symbolIds.set(symbol, entityId);
    let definition = null;
    let origin = "EXTERNAL";
    if (declaration && analyzedSourcePaths.has(path.resolve(declaration.getSourceFile().fileName))) {
      const source = declaration.getSourceFile();
      definition = fileLocation(source, declaration.getStart(source), declaration.getEnd(), projectDirectory);
      origin = sourceOrigin(source.fileName, projectDirectory);
    }
    entities.set(entityId, {
      adapterNamespace: `${ADAPTER_ID}/${ADAPTER_VERSION}`,
      opaqueId: entityId,
      resolution: "RESOLVED",
      coarseKind: symbolKind(symbol),
      displayName: symbol.getName(),
      origin,
      primaryDefinition: definition,
      languagePayload: {
        schema: "codeclew.typescript-symbol/0.1",
        qualifiedName,
        symbolFlags: symbol.flags,
        providerKind: ts.SymbolFlags[symbol.flags] ?? String(symbol.flags),
      },
    });
    return entityId;
  }

  function ensureUnresolvedEntity(sourceFile, node, displayName) {
    const location = fileLocation(sourceFile, node.getStart(sourceFile), node.getEnd(), projectDirectory);
    const entityId = `ts-unresolved:${sha256Canonical({ displayName, location })}`;
    if (!entities.has(entityId)) {
      entities.set(entityId, {
        adapterNamespace: `${ADAPTER_ID}/${ADAPTER_VERSION}`,
        opaqueId: entityId,
        resolution: "UNRESOLVED",
        coarseKind: "VALUE_LIKE",
        displayName,
        origin: sourceOrigin(sourceFile.fileName, projectDirectory),
        primaryDefinition: null,
        languagePayload: {
          schema: "codeclew.typescript-unresolved-symbol/0.1",
          spelling: node.getText(sourceFile),
        },
      });
    }
    return entityId;
  }

  function ensureFileEntity(sourceFile) {
    const key = { language: LANGUAGE_ID, file: relativePath(projectDirectory, sourceFile.fileName) };
    const entityId = `ts-file:${sha256Canonical(key)}`;
    if (!entities.has(entityId)) {
      entities.set(entityId, {
        adapterNamespace: `${ADAPTER_ID}/${ADAPTER_VERSION}`,
        opaqueId: entityId,
        resolution: "SYNTHETIC",
        coarseKind: "MODULE",
        displayName: key.file,
        origin: sourceOrigin(sourceFile.fileName, projectDirectory),
        primaryDefinition: fileLocation(sourceFile, 0, 0, projectDirectory),
        languagePayload: { schema: "codeclew.typescript-file/0.1" },
      });
    }
    return entityId;
  }

  function enclosingEntity(node, sourceFile) {
    for (let current = node.parent; current; current = current.parent) {
      const name = declarationNameNode(current);
      if (name) {
        const symbol = symbolAt(checker, name);
        const entityId = ensureEntity(symbol, current);
        if (entityId) return entityId;
      }
    }
    return ensureFileEntity(sourceFile);
  }

  function addOccurrence(kind, sourceFile, node, entityId, ownerEntityId) {
    const location = fileLocation(sourceFile, node.getStart(sourceFile), node.getEnd(), projectDirectory);
    const material = { kind, entityId, ownerEntityId, location };
    const occurrenceId = sha256Canonical(material);
    occurrences.push({
      occurrenceId,
      role: kind,
      origin: sourceOrigin(sourceFile.fileName, projectDirectory) === "GENERATED" ? "GENERATED" : "SOURCE",
      entityId,
      ownerEntityId,
      range: location,
      grade: entities.get(entityId)?.resolution === "RESOLVED"
        ? ProofGrade.COMPILER_RESOLVED
        : kind === "DEFINITION"
          ? ProofGrade.NAVIGATION
          : ProofGrade.STATICALLY_APPROXIMATED,
    });
    return occurrenceId;
  }

  function addFact(relationUri, subjectEntityId, objectEntityId, truthValue, evidenceOccurrenceId, unknownBoundaryIds = []) {
    const key = `${relationUri}\u0000${subjectEntityId ?? ""}\u0000${objectEntityId ?? ""}\u0000${truthValue}`;
    const previous = facts.get(key);
    if (previous) {
      if (evidenceOccurrenceId) previous.providerPayload.evidenceOccurrenceIds.push(evidenceOccurrenceId);
      previous.providerPayload.unknownReasonBoundaryIds.push(...unknownBoundaryIds);
      return;
    }
    const material = { relationUri, subjectEntityId, objectEntityId, truthValue };
    facts.set(key, {
      factId: sha256Canonical(material),
      relation: `${relationUri}/${OPERATION_VERSION}`,
      owner: subjectEntityId,
      target: objectEntityId,
      truth: truthValue,
      grade: truthValue === "UNKNOWN"
        ? ProofGrade.STATICALLY_APPROXIMATED
        : relationUri.endsWith("may-call")
          ? ProofGrade.STATICALLY_APPROXIMATED
          : ProofGrade.COMPILER_RESOLVED,
      enumeration: Enumeration.PARTIAL,
      providerPayload: {
        approximation: relationUri.endsWith("may-call") ? "HEURISTIC" : "EXACT",
        evidenceOccurrenceIds: evidenceOccurrenceId ? [evidenceOccurrenceId] : [],
        unknownReasonBoundaryIds: [...unknownBoundaryIds],
        scope: "configured TypeScript program source files; dynamic and open-world targets excluded",
      },
      range: occurrences.find((occurrence) => occurrence.occurrenceId === evidenceOccurrenceId)?.range ?? null,
    });
  }

  for (const sourceFile of sourceFiles) {
    ensureFileEntity(sourceFile);
    if (sourceOrigin(sourceFile.fileName, projectDirectory) === "GENERATED") {
      boundary(
        "GENERATED_SOURCE",
        sourceFile,
        0,
        sourceFile.getEnd(),
        Consequence.LOCAL_ONLY,
        "The source is classified as generated and may be replaced by its owning generator.",
        [],
      );
    }
    if (sourceFile.isDeclarationFile) {
      boundary(
        "AMBIENT_DECLARATION",
        sourceFile,
        0,
        sourceFile.getEnd(),
        Consequence.ENUMERATION_INCOMPLETE,
        "Ambient declarations describe types but do not expose an implementation body.",
        [CALL_RELATION],
      );
    }

    function visit(node) {
      if (node.kind === ts.SyntaxKind.AnyKeyword) {
        boundary(
          "ANY_TYPE",
          sourceFile,
          node.getStart(sourceFile),
          node.getEnd(),
          Consequence.PROOF_INVALID,
          "Explicit any suppresses type information needed for semantic closure.",
          [REFERENCE_RELATION, CALL_RELATION],
        );
      }
      if (ts.isElementAccessExpression(node) && !ts.isStringLiteralLike(node.argumentExpression) && !ts.isNumericLiteral(node.argumentExpression)) {
        boundary(
          "DYNAMIC_ELEMENT_ACCESS",
          sourceFile,
          node.argumentExpression.getStart(sourceFile),
          node.argumentExpression.getEnd(),
          Consequence.ENUMERATION_INCOMPLETE,
          "A computed property name can select targets outside the statically enumerated edges.",
          [REFERENCE_RELATION, CALL_RELATION],
        );
      }

      if (ts.isIdentifier(node)) {
        if (isDeclarationName(node)) {
          const symbol = symbolAt(checker, node);
          const entityId = ensureEntity(symbol, node.parent);
          if (entityId) {
            addOccurrence(
              declarationOccurrenceRole(node, sourceFile),
              sourceFile,
              node,
              entityId,
              enclosingEntity(node, sourceFile),
            );
          }
        } else if (!isNonReferenceIdentifier(node)) {
          const symbol = symbolAt(checker, node);
          const ownerEntityId = enclosingEntity(node, sourceFile);
          if (!symbol) {
            const entityId = ensureUnresolvedEntity(sourceFile, node, node.text);
            const occurrenceId = addOccurrence(
              "REFERENCE",
              sourceFile,
              node,
              entityId,
              ownerEntityId,
            );
            const boundaryId = boundary(
              "UNRESOLVED_SYMBOL",
              sourceFile,
              node.getStart(sourceFile),
              node.getEnd(),
              Consequence.ENUMERATION_INCOMPLETE,
              `The TypeScript compiler did not bind identifier '${node.text}' to a symbol.`,
              [REFERENCE_RELATION, CALL_RELATION],
            );
            addFact("codeclew.relation/may-reference", ownerEntityId, entityId, "UNKNOWN", occurrenceId, [boundaryId]);
          } else {
            const entityId = ensureEntity(symbol, node);
            const kinds = isWriteExpression(node)
              ? isCompoundWrite(node) ? ["REFERENCE", "READ", "WRITE"] : ["REFERENCE", "WRITE"]
              : ["REFERENCE", "READ"];
            for (const kind of kinds) {
              const occurrenceId = addOccurrence(kind, sourceFile, node, entityId, ownerEntityId);
              addFact("codeclew.relation/may-reference", ownerEntityId, entityId, "TRUE", occurrenceId);
            }
            try {
              const type = checker.getTypeAtLocation(node);
              if (type.flags & ts.TypeFlags.Any) {
                boundary(
                  "ANY_TYPE",
                  sourceFile,
                  node.getStart(sourceFile),
                  node.getEnd(),
                  Consequence.PROOF_INVALID,
                  `Identifier '${node.text}' has compiler type any.`,
                  [REFERENCE_RELATION, CALL_RELATION],
                );
              }
            } catch {
              // Compiler diagnostics and unresolved-symbol boundaries remain authoritative.
            }
          }
        }
      }

      if (ts.isCallExpression(node)) {
        const ownerEntityId = enclosingEntity(node, sourceFile);
        const signature = checker.getResolvedSignature(node);
        const declaration = signature?.declaration ?? null;
        const targetSymbol = declaration?.symbol ?? symbolAt(checker, node.expression);
        const targetEntityId = ensureEntity(targetSymbol, declaration ?? node.expression) ??
          ensureUnresolvedEntity(sourceFile, node.expression, node.expression.getText(sourceFile));
        const callOccurrenceId = addOccurrence(
          "CALL",
          sourceFile,
          node.expression,
          targetEntityId,
          ownerEntityId,
        );
        if (signature && targetEntityId) {
          addFact("codeclew.relation/may-call", ownerEntityId, targetEntityId, "TRUE", callOccurrenceId);
        } else {
          const boundaryId = boundary(
            "UNRESOLVED_CALL_TARGET",
            sourceFile,
            node.expression.getStart(sourceFile),
            node.expression.getEnd(),
            Consequence.ENUMERATION_INCOMPLETE,
            "The TypeScript compiler did not provide a resolved call signature and target.",
            [CALL_RELATION],
          );
          addFact("codeclew.relation/may-call", ownerEntityId, targetEntityId, "UNKNOWN", callOccurrenceId, [boundaryId]);
        }
      }
      ts.forEachChild(node, visit);
    }
    visit(sourceFile);
  }

  for (const fact of facts.values()) {
    fact.providerPayload.evidenceOccurrenceIds = [...new Set(fact.providerPayload.evidenceOccurrenceIds)].sort();
    fact.providerPayload.unknownReasonBoundaryIds = [...new Set(fact.providerPayload.unknownReasonBoundaryIds)].sort();
  }
  return {
    entities: stableSort([...entities.values()], (entity) => entity.opaqueId),
    occurrences: stableSort(occurrences, (occurrence) => occurrence.occurrenceId),
    facts: stableSort([...facts.values()], (fact) => fact.factId),
  };
}

function runCompilerReceipt(configFile, projectDirectory, snapshotTreeDigest, analysisInputDigest, program) {
  const apiDiagnostics = ts.getPreEmitDiagnostics(program);
  const compilerScratch = fs.mkdtempSync(path.join(os.tmpdir(), "codeclew-typescript-check-"));
  const commandArgs = [
    TSC_FILE,
    "--project",
    configFile,
    "--noEmit",
    "--pretty",
    "false",
    "--incremental",
    "true",
    "--tsBuildInfoFile",
    path.join(compilerScratch, "state.tsbuildinfo"),
  ];
  const cliStart = performance.now();
  let cli;
  try {
    cli = spawnSync(process.execPath, commandArgs, {
      cwd: projectDirectory,
      encoding: "utf8",
      env: process.env,
    });
  } finally {
    fs.rmSync(compilerScratch, { recursive: true, force: true });
  }
  const cliMs = performance.now() - cliStart;
  const diagnostics = apiDiagnostics.map((diagnostic) => ({
    code: diagnostic.code,
    category: categoryName(diagnostic.category),
    message: ts.flattenDiagnosticMessageText(diagnostic.messageText, "\n"),
    location: diagnosticLocation(diagnostic, projectDirectory),
  }));
  stableSort(diagnostics, (diagnostic) =>
    `${diagnostic.location?.artifactId ?? ""}:${diagnostic.location?.startByte ?? -1}:${diagnostic.code}`
  );
  const accepted = apiDiagnostics.length === 0 && cli.status === 0 && !cli.error;
  const receiptBody = {
    schema: "codeclew.compiler-receipt/0.1",
    method: "TYPESCRIPT_COMPILER_API_AND_TSC_NO_EMIT",
    status: accepted ? "ACCEPTED" : "REJECTED",
    grade: ProofGrade.COMPILER_CHECKED,
    claim: "The exact configured TypeScript program is accepted by the pinned compiler with --noEmit.",
    snapshotTreeDigest,
    providerPayload: {
      schema: "codeclew.typescript-compiler-receipt/0.1",
      analysisInputDigest,
      accepted,
      exitCode: cli.status,
      signal: cli.signal,
      command: [
        "node",
        "typescript/lib/tsc.js",
        "--project",
        relativePath(projectDirectory, configFile),
        "--noEmit",
        "--pretty",
        "false",
        "--incremental",
        "true",
        "--tsBuildInfoFile",
        "<isolated-temporary-directory>/state.tsbuildinfo",
      ],
      workingDirectory: ".",
      compilerVersion: ts.version,
      compilerBinaryDigest: digestFile(TSC_FILE),
      apiDiagnosticCount: diagnostics.length,
      diagnostics,
      stdoutDigest: sha256Bytes(cli.stdout ?? ""),
      stderrDigest: sha256Bytes(cli.stderr ?? ""),
      spawnError: cli.error ? String(cli.error.message) : null,
    },
  };
  return {
    receipt: receiptBody,
    cliMs,
  };
}

function capabilityDescriptors(toolchainDigest, buildConfigurationDigest, targetDigest, boundaries, compilerAccepted) {
  const relevant = (operationUri) => boundaries.filter((item) => item.details.affectsOperations.includes(operationUri));
  function descriptor(operationUri, proofGrade, approximation, forcedEnumeration = null) {
    const operationBoundaries = relevant(operationUri);
    const enumeration = forcedEnumeration ?? (operationBoundaries.some((item) =>
      item.consequence === Consequence.ENUMERATION_INCOMPLETE || item.consequence === Consequence.PROOF_INVALID
    ) ? Enumeration.PARTIAL : Enumeration.COMPLETE_IN_SCOPE);
    const body = {
      languageId: LANGUAGE_ID,
      adapterId: ADAPTER_ID,
      adapterVersion: ADAPTER_VERSION,
      toolchainDigest,
      buildConfigurationDigest,
      targetDigest,
      operationUri,
      operationVersion: OPERATION_VERSION,
      operationSpecificationDigest: sha256Canonical({
        schema: "codeclew.typescript-operation-specification/0.1",
        operationUri,
        operationVersion: OPERATION_VERSION,
        semantics: operationSemantics(operationUri),
      }),
      grade: proofGrade,
      support: "SUPPORTED",
      approximation,
      guaranteedEnumeration: enumeration,
      knownBoundaryKinds: [...new Set(operationBoundaries.map((item) => item.kindUri))].sort(),
      costClass: operationUri === "codeclew.operation/typescript-compiler-check"
        ? "COMPILER_FRONTEND"
        : "COMPILER_API",
    };
    return body;
  }
  return stableSort([
    descriptor("codeclew.operation/typescript-snapshot", ProofGrade.NAVIGATION, "EXACT", Enumeration.COMPLETE_IN_SCOPE),
    descriptor("codeclew.operation/typescript-entities", ProofGrade.NAVIGATION, "EXACT", Enumeration.PARTIAL),
    descriptor(REFERENCE_RELATION, ProofGrade.COMPILER_RESOLVED, "EXACT"),
    descriptor(CALL_RELATION, ProofGrade.STATICALLY_APPROXIMATED, "HEURISTIC", Enumeration.PARTIAL),
    descriptor(
      "codeclew.operation/typescript-compiler-check",
      ProofGrade.COMPILER_CHECKED,
      "NOT_APPLICABLE",
      compilerAccepted ? Enumeration.COMPLETE_IN_SCOPE : Enumeration.UNKNOWN,
    ),
  ], (item) => item.operationUri);
}

function operationSemantics(operationUri) {
  switch (operationUri) {
    case REFERENCE_RELATION:
      return "Compiler-bound source occurrences contribute may-reference TRUE facts; absence is never a negative fact.";
    case CALL_RELATION:
      return "Resolved signatures contribute heuristic may-call candidates; structural/open-world dispatch remains partial.";
    case "codeclew.operation/typescript-compiler-check":
      return "TypeScript Compiler API diagnostics and a separate pinned tsc --noEmit process must both accept.";
    case "codeclew.operation/typescript-snapshot":
      return "Repository/configuration/compiler inputs are content-addressed with normalized identities.";
    case "codeclew.operation/typescript-entities":
      return "Compiler symbols are mapped to adapter-owned opaque entity identities; this is navigation evidence.";
    default:
      return "Adapter-owned operation with no cross-language semantic interpretation.";
  }
}

const IMPACT_CLOSURE_SPEC = "codeclew.impact.reverse-resolved-relations/0.1";

function boundedReverseImpact(seed, entities, facts, boundaries, maxDepth, maxEntities, compilerAccepted) {
  const started = performance.now();
  const obligations = boundaries.map((item, index) => {
    const mandatory = item.consequence !== Consequence.LOCAL_ONLY;
    return {
      id: `validate-boundary-${index}`,
      kind: "codeclew.obligation/validate-boundary/1",
      mandatory,
      status: mandatory ? "UNKNOWN" : "SATISFIED",
      boundaryDigest: item.boundaryId,
    };
  });
  obligations.unshift({
    id: "typescript-compiler-check",
    kind: "codeclew.obligation/compiler-check/1",
    mandatory: true,
    status: compilerAccepted ? "SATISFIED" : "VIOLATED",
  });

  const base = {
    schema: "codeclew.impact-result/0.1",
    closureSpecification: IMPACT_CLOSURE_SPEC,
    affected: [],
    paths: [],
    mandatoryObligations: obligations,
    boundaries,
  };
  if (!seed) {
    obligations.unshift({
      id: "resolve-seed",
      kind: "codeclew.obligation/resolve-entity/1",
      mandatory: true,
      status: "UNKNOWN",
    });
    return {
      ...base,
      status: "UNKNOWN",
      reason: "NO_SEED_ENTITY",
      queryMicros: Math.round((performance.now() - started) * 1000),
    };
  }
  if (!entities.some((entity) => entity.opaqueId === seed && entity.resolution !== "UNRESOLVED")) {
    obligations.unshift({
      id: "resolve-seed",
      kind: "codeclew.obligation/resolve-entity/1",
      mandatory: true,
      status: "UNKNOWN",
    });
    return {
      ...base,
      status: "UNKNOWN",
      reason: "UNRESOLVED_SEED_ENTITY",
      seedEntity: seed,
      queryMicros: Math.round((performance.now() - started) * 1000),
    };
  }

  const queue = [{ entityId: seed, depth: 0 }];
  const visited = new Set([seed]);
  const affected = [{ entityId: seed, impactClass: "DEFINITE", depth: 0 }];
  const paths = [];
  let budgetBoundary = false;
  for (let cursor = 0; cursor < queue.length; cursor += 1) {
    const { entityId, depth } = queue[cursor];
    const incoming = facts.filter((fact) => fact.truth === "TRUE" && fact.target === entityId);
    if (depth >= maxDepth) {
      if (incoming.some((fact) => !visited.has(fact.owner))) budgetBoundary = true;
      continue;
    }
    for (const fact of incoming) {
      paths.push({
        from: entityId,
        to: fact.owner,
        factId: fact.factId,
        relation: fact.relation,
      });
      if (!visited.has(fact.owner)) {
        if (affected.length >= maxEntities) {
          budgetBoundary = true;
          break;
        }
        visited.add(fact.owner);
        affected.push({ entityId: fact.owner, impactClass: "POSSIBLE", depth: depth + 1 });
        queue.push({ entityId: fact.owner, depth: depth + 1 });
      }
    }
  }

  const impactBoundaries = [...boundaries];
  if (budgetBoundary) {
    const details = { maxDepth, maxEntities };
    const budget = {
      boundaryId: sha256Canonical({ kind: "projection-budget", ...details }),
      kindUri: "codeclew.boundary/projection-budget/1",
      consequence: Consequence.ENUMERATION_INCOMPLETE,
      origin: null,
      provider: ADAPTER_ID,
      details,
    };
    impactBoundaries.push(budget);
    obligations.push({
      id: "validate-projection-budget",
      kind: "codeclew.obligation/increase-impact-budget/1",
      mandatory: true,
      status: "UNKNOWN",
      boundaryDigest: budget.boundaryId,
    });
  }
  const semanticBoundary = impactBoundaries.some((item) =>
    item.consequence === Consequence.ENUMERATION_INCOMPLETE || item.consequence === Consequence.PROOF_INVALID
  );
  return {
    schema: "codeclew.impact-result/0.1",
    status: !compilerAccepted
      ? "UNKNOWN"
      : budgetBoundary
        ? "PARTIAL_BUDGET"
        : semanticBoundary
          ? "PARTIAL_BOUNDARY"
          : "COMPLETE_IN_SCOPE",
    closureSpecification: IMPACT_CLOSURE_SPEC,
    seedEntity: seed,
    maxDepth,
    maxEntities,
    affected,
    paths,
    mandatoryObligations: obligations,
    boundaries: impactBoundaries,
    queryMicros: Math.round((performance.now() - started) * 1000),
  };
}

export function runAdapter(projectArgument, options = {}) {
  const started = performance.now();
  const phase = {};
  const discoveryStart = performance.now();
  const { configFile, projectDirectory } = resolveProject(projectArgument);
  const { loaded, parsed } = readStrictConfiguration(configFile);
  phase.discoveryMs = performance.now() - discoveryStart;

  const programStart = performance.now();
  const program = ts.createProgram({
    rootNames: parsed.fileNames,
    options: { ...parsed.options, noEmit: true },
    projectReferences: parsed.projectReferences,
  });
  const sourceFiles = configuredProgramSources(program, projectDirectory);
  phase.compilerProgramMs = performance.now() - programStart;

  const boundaries = new Map();
  function boundary(kind, sourceFile, start, end, consequence, explanation, affectsOperations) {
    const location = sourceFile
      ? fileLocation(sourceFile, start, end, projectDirectory)
      : {
          artifactId: `source:${relativePath(projectDirectory, configFile)}`,
          artifactContentDigest: digestFile(configFile),
          startByte: 0,
          endByte: fs.statSync(configFile).size,
        };
    const material = { kind, consequence, location, affectsOperations: [...affectsOperations].sort() };
    const boundaryId = sha256Canonical(material);
    if (!boundaries.has(boundaryId)) {
      boundaries.set(boundaryId, {
        boundaryId,
        kindUri: `codeclew.boundary/typescript/${kind.toLowerCase().replaceAll("_", "-")}/1`,
        consequence,
        origin: location,
        provider: "TYPESCRIPT_COMPILER_API",
        details: {
          providerKind: kind,
          explanation,
          affectsOperations: [...affectsOperations].sort(),
        },
      });
    }
    return boundaryId;
  }

  boundary(
    "OPEN_WORLD_DISPATCH",
    null,
    0,
    0,
    Consequence.ENUMERATION_INCOMPLETE,
    "TypeScript structural typing, callbacks, and runtime mutation prevent a closed-world runtime call-target enumeration.",
    [CALL_RELATION],
  );
  if (!parsed.options.sourceMap && !parsed.options.inlineSourceMap) {
    boundary(
      "SOURCE_MAP_DISABLED",
      null,
      0,
      0,
      Consequence.LOCAL_ONLY,
      "No emitted JavaScript source map is configured; evidence locations refer directly to TypeScript source.",
      [],
    );
  } else {
    boundary(
      "SOURCE_MAP_NOT_CONSUMED",
      null,
      0,
      0,
      Consequence.LOCAL_ONLY,
      "Source-map generation is configured, but this source-only adapter does not map emitted JavaScript locations.",
      [],
    );
  }
  if (parsed.options.skipLibCheck === true) {
    boundary(
      "SKIPPED_DECLARATION_CHECK",
      null,
      0,
      0,
      Consequence.PROOF_INVALID,
      "skipLibCheck excludes declaration-file type checking from compiler acceptance.",
      [REFERENCE_RELATION, CALL_RELATION],
    );
  }
  if (parsed.options.allowJs === true && parsed.options.checkJs !== true) {
    boundary(
      "UNCHECKED_JAVASCRIPT",
      null,
      0,
      0,
      Consequence.PROOF_INVALID,
      "JavaScript sources are admitted without checkJs and cannot close compiler-resolved obligations.",
      [REFERENCE_RELATION, CALL_RELATION],
    );
  }
  if ((parsed.projectReferences ?? []).length > 0) {
    boundary(
      "PROJECT_REFERENCE_BOUNDARY",
      null,
      0,
      0,
      Consequence.ENUMERATION_INCOMPLETE,
      "Referenced projects are captured in build/dependency digests but are not recursively indexed by this adapter invocation.",
      [REFERENCE_RELATION, CALL_RELATION],
    );
  }
  const externalAmbientDependencies = program.getSourceFiles()
    .filter((source) => source.isDeclarationFile)
    .filter((source) => !sourceFiles.includes(source))
    .filter((source) => !program.isSourceFileDefaultLibrary(source));
  if (externalAmbientDependencies.length > 0) {
    boundary(
      "EXTERNAL_AMBIENT_DEPENDENCY",
      null,
      0,
      0,
      Consequence.ENUMERATION_INCOMPLETE,
      `${externalAmbientDependencies.length} external ambient declaration source(s) affect resolution but are outside the configured source contour.`,
      [REFERENCE_RELATION, CALL_RELATION],
    );
  }

  const snapshotStart = performance.now();
  const snapshot = snapshotRepositorySources(projectDirectory, sourceFiles);
  const sources = snapshot.sources;
  if (snapshot.excludedSymlinks.length > 0) {
    boundary(
      "SYMLINKS_NOT_TRAVERSED",
      null,
      0,
      0,
      Consequence.LOCAL_ONLY,
      `${snapshot.excludedSymlinks.length} repository symlink(s) were not traversed; configured compiler inputs remain content-addressed separately.`,
      [],
    );
  }
  const dependencyEdges = buildDependencyEdges(sourceFiles, parsed.options, projectDirectory, boundary);
  const dependencyArtifacts = program.getSourceFiles()
    .filter((source) => !sourceFiles.includes(source))
    .filter((source) => !program.isSourceFileDefaultLibrary(source))
    .map((source) => ({
      normalizedPath: relativePath(projectDirectory, source.fileName),
      contentDigest: sourceArtifact(source).contentDigest,
    }))
    .sort((left, right) => left.normalizedPath.localeCompare(right.normalizedPath));
  const effectiveCompilerOptions = normalizeCompilerValue(parsed.options, projectDirectory);
  delete effectiveCompilerOptions.configFilePath;
  const buildModel = {
    configPath: relativePath(projectDirectory, configFile),
    configDigest: digestFile(configFile),
    rawConfiguration: loaded,
    effectiveCompilerOptions,
    projectReferences: (parsed.projectReferences ?? []).map((reference) => relativePath(projectDirectory, reference.path)).sort(),
    rootFiles: parsed.fileNames.map((file) => relativePath(projectDirectory, file)).sort(),
  };
  const buildModelDigest = sha256Canonical(buildModel);
  const buildConfigurationDigest = sha256Canonical({
    effectiveCompilerOptions,
    projectReferences: buildModel.projectReferences,
  });
  const dependencyGraphDigest = sha256Canonical({ dependencyEdges, dependencyArtifacts });
  const compilerDistribution = program.getSourceFiles()
    .filter((source) => program.isSourceFileDefaultLibrary(source))
    .map((source) => ({
      path: toPosix(path.relative(TYPESCRIPT_PACKAGE_ROOT, source.fileName)),
      contentDigest: sourceArtifact(source).contentDigest,
    }));
  compilerDistribution.push({
    path: toPosix(path.relative(TYPESCRIPT_PACKAGE_ROOT, TYPESCRIPT_API_FILE)),
    contentDigest: digestFile(TYPESCRIPT_API_FILE),
  });
  compilerDistribution.push({
    path: toPosix(path.relative(TYPESCRIPT_PACKAGE_ROOT, TSC_FILE)),
    contentDigest: digestFile(TSC_FILE),
  });
  compilerDistribution.sort((left, right) => left.path.localeCompare(right.path));
  const toolchain = {
    toolUri: "codeclew.toolchain/typescript/1",
    version: ts.version,
    distributionDigest: sha256Canonical(compilerDistribution),
    providerPayload: {
      languageId: LANGUAGE_ID,
      compiler: "typescript",
      compilerApiDigest: digestFile(TYPESCRIPT_API_FILE),
      compilerBinaryDigest: digestFile(TSC_FILE),
      nodeVersion: process.version,
      nodeExecutableDigest: digestFile(process.execPath),
    },
  };
  const toolchainDigest = sha256Canonical(toolchain);
  const git = getGitIdentity(projectDirectory);
  const generated = sources.filter((source) => source.origin === "GENERATED");
  const repositoryTreeDigest = sha256Canonical({
    schema: "codeclew.repository-tree/0.1",
    members: sources,
  });
  const snapshotInput = {
    repositoryTreeDigest,
    vcsRevision: git.vcsRevision,
    dirty: git.dirty ?? true,
    sources,
    buildSystemUri: "https://www.typescriptlang.org/tsconfig",
    buildModelDigest,
    buildConfigurationDigest,
    dependencyGraphDigest,
    toolchain,
    targets: [{
      targetId: "typescript-configured-program",
      configurationDigest: buildConfigurationDigest,
      enabledFeatures: ["strict"],
      platform: ts.ScriptTarget[parsed.options.target ?? ts.ScriptTarget.ES5],
      compilerFlags: ["--strict", "--noEmit"],
      providerPayload: {
        module: ts.ModuleKind[parsed.options.module ?? ts.ModuleKind.None],
        moduleResolution: ts.ModuleResolutionKind[parsed.options.moduleResolution ?? ts.ModuleResolutionKind.Classic],
      },
    }],
    relevantEnvironment:
      ["LANG", "LC_ALL", "NODE_OPTIONS", "TSC_NONPOLLING_WATCHER", "TSC_WATCHDIRECTORY", "TSC_WATCHFILE", "TZ"]
        .map((key) => ({ key, value: process.env[key] ?? "<unset>" })),
    generatedSourcesManifestDigest: sha256Canonical(generated.map((source) => ({
      artifactId: source.artifactId,
      contentDigest: source.contentDigest,
    }))),
  };
  const targetDigest = buildConfigurationDigest;
  const analysisInputDigest = sha256Canonical(snapshotInput);
  phase.snapshotMs = performance.now() - snapshotStart;

  const extractionStart = performance.now();
  const semantic = buildSemanticEvidence(program, sourceFiles, projectDirectory, boundary);
  phase.extractionMs = performance.now() - extractionStart;

  const receiptResult = runCompilerReceipt(
    configFile,
    projectDirectory,
    repositoryTreeDigest,
    analysisInputDigest,
    program,
  );
  phase.compilerReceiptMs = receiptResult.cliMs;

  const boundaryValues = stableSort([...boundaries.values()], (item) => item.boundaryId);
  const adapterBinaryDigest = digestFile(THIS_FILE);
  const impact = boundedReverseImpact(
    options.seedEntity,
    semantic.entities,
    semantic.facts,
    boundaryValues,
    options.maxDepth ?? 2,
    options.maxEntities ?? 128,
    receiptResult.receipt.accepted,
  );
  const stablePayload = {
    schema: ADAPTER_SCHEMA,
    adapter: {
      adapterId: ADAPTER_ID,
      version: ADAPTER_VERSION,
      binaryDigest: adapterBinaryDigest,
      languageId: LANGUAGE_ID,
    },
    snapshotInput,
    capabilityDescriptors: capabilityDescriptors(
      toolchainDigest,
      buildConfigurationDigest,
      targetDigest,
      boundaryValues,
      receiptResult.receipt.accepted,
    ),
    entities: semantic.entities,
    occurrences: semantic.occurrences,
    facts: semantic.facts,
    boundaries: boundaryValues,
    compilerReceipt: receiptResult.receipt,
    impact,
  };
  const totalWithoutSerialization = performance.now() - started;
  const cost = {
    totalWallMicros: Math.round(totalWithoutSerialization * 1000),
    repositorySnapshotMicros: Math.round(phase.snapshotMs * 1000),
    buildDiscoveryMicros: Math.round(phase.discoveryMs * 1000),
    coldIndexMicros: Math.round(phase.compilerProgramMs * 1000),
    warmIndexMicros: 0,
    adapterMicros: Math.round((phase.compilerProgramMs + phase.extractionMs + phase.compilerReceiptMs) * 1000),
    queryMicros: impact.queryMicros,
    sourceBytesRead: sources.reduce((sum, source) => sum + source.sizeBytes, 0),
    emittedBytes: 0,
    storedFactBytes: Buffer.byteLength(canonicalStringify(semantic.facts)),
    modelVisibleSourceBytes: 0,
    cacheRequests: 0,
    cacheHits: 0,
    providerProcessingMicros: Math.round(phase.extractionMs * 1000),
  };
  return {
    bundle: { ...stablePayload, cost, outputDigest: "" },
    accepted: receiptResult.receipt.accepted,
  };
}

function sealAndSerialize(bundle) {
  bundle.outputDigest = `sha256:${"0".repeat(64)}`;
  for (let attempt = 0; attempt < 8; attempt += 1) {
    const serialized = canonicalStringify(bundle);
    const bytes = Buffer.byteLength(serialized);
    if (bundle.cost.emittedBytes === bytes) break;
    bundle.cost.emittedBytes = bytes;
  }
  bundle.outputDigest = "";
  bundle.outputDigest = sha256Canonical(bundle);
  return `${canonicalStringify(bundle)}\n`;
}

function writeOutput(output, contents) {
  if (output === "-") {
    process.stdout.write(contents);
    return;
  }
  const resolved = path.resolve(output);
  fs.mkdirSync(path.dirname(resolved), { recursive: true });
  fs.writeFileSync(resolved, contents);
}

function main() {
  try {
    const options = parseArguments(process.argv.slice(2));
    if (options.help) {
      process.stdout.write(`${usage()}\n`);
      return;
    }
    const result = runAdapter(options.project, options);
    writeOutput(options.output, sealAndSerialize(result.bundle));
    if (!result.accepted) process.exitCode = 2;
  } catch (error) {
    if (error instanceof AdapterError) {
      process.stderr.write(`${canonicalStringify({
        schema: "codeclew.adapter-error/0.1",
        code: error.code,
        message: error.message,
        details: error.details,
      })}\n`);
      process.exitCode = error.exitCode;
      return;
    }
    process.stderr.write(`${canonicalStringify({
      schema: "codeclew.adapter-error/0.1",
      code: "INTERNAL_ERROR",
      message: error instanceof Error ? error.message : String(error),
    })}\n`);
    process.exitCode = 1;
  }
}

if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  main();
}
