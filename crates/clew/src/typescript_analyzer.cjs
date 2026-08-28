'use strict'

const crypto = require('crypto')
const fs = require('fs')
const path = require('path')

const FACT_SCHEMA = 'codeclew-typescript-compiler-fact/1.0'
const repository = fs.realpathSync(process.argv[2])
const configRelative = process.argv[3]
const typescriptModule = fs.realpathSync(process.argv[4])
const ts = require(typescriptModule)
const configPath = path.resolve(repository, configRelative)
const repositoryPrefix = repository.endsWith(path.sep) ? repository : repository + path.sep
const typescriptRoot = path.dirname(path.dirname(typescriptModule))
const typescriptPrefix = typescriptRoot.endsWith(path.sep) ? typescriptRoot : typescriptRoot + path.sep

function posix(value) {
  return value.split(path.sep).join('/')
}

function repositoryRelative(file) {
  const resolved = path.resolve(file)
  if (resolved === repository) return '.'
  if (!resolved.startsWith(repositoryPrefix)) return null
  return posix(path.relative(repository, resolved))
}

function isDependencyRelative(relative) {
  return Boolean(relative && relative.split('/').includes('node_modules'))
}

function logicalExternal(file) {
  const resolved = path.resolve(file)
  if (resolved.startsWith(typescriptPrefix)) {
    return `typescript/${posix(path.relative(typescriptRoot, resolved))}`
  }
  const relative = repositoryRelative(resolved)
  if (isDependencyRelative(relative)) return relative
  let content
  try {
    content = fs.readFileSync(resolved)
  } catch (_) {
    content = Buffer.from(resolved)
  }
  return `external/sha256-${crypto.createHash('sha256').update(content).digest('hex')}`
}

function normalizeText(value) {
  return String(value)
    .split(repository).join('<project>')
    .split(posix(repository)).join('<project>')
    .split(typescriptRoot).join('<typescript>')
    .split(posix(typescriptRoot)).join('<typescript>')
    .slice(0, 4096)
}

const configDiagnostics = []
const parsed = ts.getParsedCommandLineOfConfigFile(
  configPath,
  {},
  { ...ts.sys, onUnRecoverableConfigFileDiagnostic: diagnostic => configDiagnostics.push(diagnostic) },
)
if (!parsed) {
  process.stderr.write('TypeScript configuration could not be parsed\n')
  process.exit(2)
}

const options = { ...parsed.options, noEmit: true }
const program = ts.createProgram({
  rootNames: parsed.fileNames,
  options,
  projectReferences: parsed.projectReferences,
})
const checker = program.getTypeChecker()
const facts = []
const externalFiles = []
const sourceFiles = []
const byteMaps = new Map()

function byteMap(source) {
  if (byteMaps.has(source.fileName)) return byteMaps.get(source.fileName)
  const text = source.text
  const offsets = new Uint32Array(text.length + 1)
  let bytes = 0
  let index = 0
  while (index < text.length) {
    offsets[index] = bytes
    const codePoint = text.codePointAt(index)
    const character = String.fromCodePoint(codePoint)
    const units = character.length
    for (let unit = 1; unit < units; unit += 1) offsets[index + unit] = bytes
    bytes += Buffer.byteLength(character, 'utf8')
    index += units
    offsets[index] = bytes
  }
  byteMaps.set(source.fileName, offsets)
  return offsets
}

function range(node) {
  const source = node.getSourceFile()
  const offsets = byteMap(source)
  return { start: offsets[node.getStart(source, false)], end: offsets[node.getEnd()] }
}

function declarationKind(node) {
  if (ts.isClassDeclaration(node)) return 'CLASS'
  if (ts.isInterfaceDeclaration(node)) return 'INTERFACE'
  if (ts.isTypeAliasDeclaration(node)) return 'TYPE_ALIAS'
  if (ts.isEnumDeclaration(node)) return 'ENUM'
  if (ts.isFunctionDeclaration(node)) return 'FUNCTION'
  if (ts.isMethodDeclaration(node) || ts.isMethodSignature(node)) return 'METHOD'
  if (ts.isPropertyDeclaration(node) || ts.isPropertySignature(node)) return 'PROPERTY'
  if (ts.isVariableDeclaration(node)) return 'VARIABLE'
  if (ts.isConstructorDeclaration(node)) return 'CONSTRUCTOR'
  return null
}

function declarationName(node) {
  if (node.name && ts.isIdentifier(node.name)) return node.name.text
  if (node.name && (ts.isStringLiteral(node.name) || ts.isNumericLiteral(node.name))) return node.name.text
  if (ts.isConstructorDeclaration(node)) return 'constructor'
  return 'default'
}

function sourceIdentity(source) {
  const relative = repositoryRelative(source.fileName)
  return relative ? `module:${relative}` : `module:${logicalExternal(source.fileName)}`
}

function identityForDeclaration(declaration, fallbackName) {
  const source = declaration.getSourceFile()
  if (ts.isSourceFile(declaration)) return sourceIdentity(declaration)
  const relative = repositoryRelative(source.fileName)
  const name = fallbackName || declarationName(declaration)
  if (!relative || isDependencyRelative(relative)) {
    return `ts-external:${logicalExternal(source.fileName)}#${name}`
  }
  const kind = declarationKind(declaration) || 'DECLARATION'
  const offsets = range(declaration)
  return `ts:${relative}#${kind.toLowerCase()}:${name}@${offsets.start}-${offsets.end}`
}

function identityForSymbol(symbol) {
  if (!symbol) return null
  const declaration = symbol.valueDeclaration || (symbol.declarations && symbol.declarations[0])
  if (!declaration) return null
  return identityForDeclaration(declaration, symbol.getName())
}

function ownerIdentity(node) {
  let parent = node.parent
  while (parent && !ts.isSourceFile(parent)) {
    if (declarationKind(parent)) {
      const symbol = parent.name ? checker.getSymbolAtLocation(parent.name) : undefined
      return identityForSymbol(symbol) || identityForDeclaration(parent)
    }
    parent = parent.parent
  }
  return sourceIdentity(node.getSourceFile())
}

function isExported(node) {
  return Boolean(node.modifiers && node.modifiers.some(modifier =>
    modifier.kind === ts.SyntaxKind.ExportKeyword || modifier.kind === ts.SyntaxKind.DefaultKeyword))
}

function addBoundary(code, node, diagnosticCode) {
  if (facts.length >= 262144) return
  const source = node && node.getSourceFile ? node.getSourceFile() : null
  const relative = source ? repositoryRelative(source.fileName) : null
  const offsets = node && relative ? range(node) : null
  facts.push({
    kind: 'BOUNDARY',
    schema: FACT_SCHEMA,
    code,
    diagnosticCode: diagnosticCode == null ? undefined : String(diagnosticCode),
    file: relative || undefined,
    start: offsets ? offsets.start : undefined,
    end: offsets ? offsets.end : undefined,
    requiredChecks: ['FIX_TYPESCRIPT_CONFIGURATION_OR_DEPENDENCY'],
    resolution: 'UNKNOWN',
  })
}

function addRelation(kind, node, sourceIdentityValue, targetSymbol) {
  const targetIdentity = identityForSymbol(targetSymbol)
  const file = repositoryRelative(node.getSourceFile().fileName)
  if (!targetIdentity || !file) return
  const offsets = range(node)
  facts.push({
    kind: 'RELATION',
    schema: FACT_SCHEMA,
    relationKind: kind,
    sourceIdentity: sourceIdentityValue,
    targetIdentity,
    file,
    start: offsets.start,
    end: offsets.end,
    resolution: 'COMPILER_RESOLVED',
  })
}

function visit(node, currentIdentity) {
  const kind = declarationKind(node)
  let identity = currentIdentity
  if (kind) {
    const symbol = node.name ? checker.getSymbolAtLocation(node.name) : undefined
    identity = identityForSymbol(symbol) || identityForDeclaration(node)
    const file = repositoryRelative(node.getSourceFile().fileName)
    if (file) {
      const offsets = range(node)
      const type = checker.getTypeAtLocation(node)
      const signature = node.parameters
        ? checker.getSignatureFromDeclaration(node)
        : undefined
      facts.push({
        kind: 'DECLARATION',
        schema: FACT_SCHEMA,
        declarationKind: kind,
        name: declarationName(node),
        symbolIdentity: identity,
        ownerIdentity: ownerIdentity(node),
        exported: isExported(node),
        typeText: normalizeText(checker.typeToString(type, node, ts.TypeFormatFlags.NoTruncation)),
        signature: signature ? normalizeText(checker.signatureToString(signature, node, ts.TypeFormatFlags.NoTruncation)) : undefined,
        file,
        start: offsets.start,
        end: offsets.end,
        resolution: 'COMPILER_RESOLVED',
      })
      if (kind !== 'CONSTRUCTOR' && (type.flags & ts.TypeFlags.Any) !== 0) {
        addBoundary('TYPESCRIPT_ANY_TYPE', node)
      }
    }
  }

  if (ts.isIdentifier(node) && node.parent && node.parent.name !== node) {
    addRelation('REFERENCES', node, identity || sourceIdentity(node.getSourceFile()), checker.getSymbolAtLocation(node))
  }
  if (ts.isCallExpression(node)) {
    const signature = checker.getResolvedSignature(node)
    if (signature && signature.declaration) {
      const targetSymbol = checker.getSymbolAtLocation(node.expression)
        || (signature.declaration.name
          ? checker.getSymbolAtLocation(signature.declaration.name)
          : undefined)
      addRelation('CALLS', node, identity || sourceIdentity(node.getSourceFile()), targetSymbol)
    } else {
      addBoundary('UNRESOLVED_CALL_SIGNATURE', node)
    }
    if (node.expression.kind === ts.SyntaxKind.ImportKeyword
        && (node.arguments.length !== 1
          || !node.arguments[0]
          || !ts.isStringLiteral(node.arguments[0]))) {
      addBoundary('DYNAMIC_IMPORT_NOT_LITERAL', node)
    }
  }
  if (ts.isImportDeclaration(node) && ts.isStringLiteral(node.moduleSpecifier)) {
    addRelation(
      'IMPORTS',
      node.moduleSpecifier,
      identity || sourceIdentity(node.getSourceFile()),
      checker.getSymbolAtLocation(node.moduleSpecifier),
    )
  }
  if (ts.isTypeReferenceNode(node)) {
    addRelation(
      'TYPE_USES',
      node,
      identity || sourceIdentity(node.getSourceFile()),
      checker.getSymbolAtLocation(node.typeName),
    )
  }
  ts.forEachChild(node, child => visit(child, identity))
}

for (const source of program.getSourceFiles()) {
  const relative = repositoryRelative(source.fileName)
  if (relative && !isDependencyRelative(relative)) {
    const lower = relative.toLowerCase()
    if (/\.(ts|tsx|mts|cts)$/.test(lower)) {
      sourceFiles.push(relative)
      visit(source, sourceIdentity(source))
    } else if (/\.(js|jsx|mjs|cjs)$/.test(lower)) {
      addBoundary('JAVASCRIPT_SOURCE_DEFERRED_TO_JAVASCRIPT_PROFILE', source)
    }
  } else if (source.isDeclarationFile || isDependencyRelative(relative)) {
    externalFiles.push({ logicalName: logicalExternal(source.fileName), physicalPath: source.fileName })
  }
}

const diagnostics = [
  ...configDiagnostics,
  ...ts.getPreEmitDiagnostics(program),
]
for (const diagnostic of diagnostics) {
  const node = diagnostic.file && diagnostic.start != null
    ? findNodeAt(diagnostic.file, diagnostic.start)
    : null
  addBoundary('TYPESCRIPT_DIAGNOSTIC', node, diagnostic.code)
}

function findNodeAt(source, position) {
  let selected = source
  function descend(node) {
    if (position < node.getFullStart() || position > node.getEnd()) return
    selected = node
    ts.forEachChild(node, descend)
  }
  descend(source)
  return selected
}

function relativeOption(value) {
  if (typeof value !== 'string') return value
  const relative = repositoryRelative(value)
  return relative || normalizeText(value)
}

const canonicalOptions = {
  strict: Boolean(options.strict),
  target: options.target == null ? null : options.target,
  module: options.module == null ? null : options.module,
  moduleResolution: options.moduleResolution == null ? null : options.moduleResolution,
  jsx: options.jsx == null ? null : options.jsx,
  allowJs: Boolean(options.allowJs),
  checkJs: Boolean(options.checkJs),
  resolveJsonModule: Boolean(options.resolveJsonModule),
  skipLibCheck: Boolean(options.skipLibCheck),
  baseUrl: options.baseUrl == null ? null : relativeOption(options.baseUrl),
  paths: options.paths || {},
  types: options.types || [],
  lib: options.lib || [],
  noEmit: true,
}

const projectReferences = (parsed.projectReferences || []).map(reference => {
  const config = path.join(reference.path, 'tsconfig.json')
  return repositoryRelative(config) || normalizeText(config)
}).sort()

sourceFiles.sort()
externalFiles.sort((left, right) => left.logicalName.localeCompare(right.logicalName))
facts.push(...projectReferences.map(reference => ({
  kind: 'BOUNDARY',
  schema: FACT_SCHEMA,
  code: 'PROJECT_REFERENCE_DECLARED_NOT_MERGED',
  file: reference,
  requiredChecks: ['ANALYZE_REFERENCED_TSCONFIG_SEPARATELY'],
  resolution: 'DECLARED',
})))

process.stdout.write(JSON.stringify({
  schema: 'codeclew-typescript-analyzer-output/1.0',
  compilerVersion: ts.version,
  nodeVersion: process.version,
  configPath: configRelative,
  sourceFiles,
  externalFiles,
  canonicalOptions,
  projectReferences,
  facts,
}))
