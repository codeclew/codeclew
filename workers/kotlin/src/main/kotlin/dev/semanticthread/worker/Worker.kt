package dev.semanticthread.worker

import dev.semanticthread.worker.IncrementalK2Runtime
import dev.semanticthread.worker.PersistentProjectModelCache
import dev.semanticthread.worker.syntaxOnlyIndexSourceFiles
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.PrintStream
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.security.MessageDigest
import java.util.zip.ZipFile
import kotlin.io.path.*
import kotlinx.serialization.json.*
import org.jetbrains.kotlin.cli.jvm.K2JVMCompiler
import org.jetbrains.kotlin.cli.jvm.compiler.EnvironmentConfigFiles
import org.jetbrains.kotlin.cli.jvm.compiler.KotlinCoreEnvironment
import org.jetbrains.kotlin.com.intellij.openapi.application.ApplicationManager
import org.jetbrains.kotlin.com.intellij.openapi.extensions.ExtensionPoint
import org.jetbrains.kotlin.com.intellij.openapi.util.Disposer
import org.jetbrains.kotlin.com.intellij.psi.PsiElement
import org.jetbrains.kotlin.com.intellij.psi.PsiErrorElement
import org.jetbrains.kotlin.com.intellij.psi.impl.source.tree.TreeCopyHandler
import org.jetbrains.kotlin.com.intellij.psi.util.PsiTreeUtil
import org.jetbrains.kotlin.config.CompilerConfiguration
import org.jetbrains.kotlin.lexer.KtTokens
import org.jetbrains.kotlin.psi.*

internal const val FIR_FACTS_EXTRACTOR_SCHEMA = "fir-facts-extractor/0.6"
internal const val SEMANTIC_K2_CACHE_SCHEMA = "semantic-k2-cache/0.4"
internal const val SEMANTIC_K2_DISK_CACHE_AUTHORITY = "NON_AUTHORITATIVE"

internal val SUPPORTED_DESCRIPTOR_KINDS = setOf(
    "FUNCTION", "CONSTRUCTOR", "PROPERTY", "MUTABLE_PROPERTY", "CLASS",
)
internal val SUPPORTED_VISIBILITIES = setOf("public", "internal", "private", "protected")
internal val SUPPORTED_EFFECTIVE_VISIBILITIES = setOf(
    "public", "internal", "private-in-class", "private-in-file", "protected",
)
internal val SUPPORTED_MODALITIES = setOf("FINAL", "OPEN", "ABSTRACT", "SEALED")
internal val SUPPORTED_RELATION_KINDS = setOf(
    "OVERRIDES", "CALLS", "REFERENCES", "CONSTRUCTS", "READS", "WRITES",
    "INITIALIZES", "NULL_COALESCES", "RETURNS_VALUE_FROM",
)

private val COMPILER_PLUGIN_SERVICE_FILES = setOf(
    "META-INF/services/org.jetbrains.kotlin.compiler.plugin.CompilerPluginRegistrar",
    "META-INF/services/org.jetbrains.kotlin.compiler.plugin.ComponentRegistrar",
)

internal data class EffectiveCompilerPluginPlan(
    val plugins: List<Path>,
    val boundaries: List<String>,
)

private fun isCompilerPluginJar(path: Path): Boolean =
    path.isRegularFile() && runCatching {
        ZipFile(path.toFile()).use { archive ->
            COMPILER_PLUGIN_SERVICE_FILES.any { archive.getEntry(it) != null }
        }
    }.getOrDefault(false)

/**
 * Run project compiler plugins only when their ABI is compatible with this
 * exact analyzer. Gradle exposes a whole plugin classpath (including support
 * libraries), while K2's -Xplugin expects registrar jars. Kotlin-owned
 * serialization is replaced by the analyzer-patch artifact bundled with the
 * worker; scripting is omitted for ordinary .kt compilations, which this
 * worker supports. Unknown registrars fail closed across patch versions.
 */
internal fun effectiveCompilerPluginPlan(
    requested: List<Path>,
    declaredCompilerVersion: String,
    analyzerClasspath: List<Path> = System.getProperty("java.class.path")
        .split(File.pathSeparator)
        .filter(String::isNotBlank)
        .map(Path::of),
): EffectiveCompilerPluginPlan {
    val requestedRegistrars = requested
        .map(Path::toAbsolutePath)
        .map(Path::normalize)
        .filter(::isCompilerPluginJar)
        .distinct()
    val effective = mutableListOf<Path>()
    val boundaries = mutableSetOf<String>()
    requestedRegistrars.forEach { plugin ->
        val name = plugin.fileName.toString()
        when {
            name.startsWith("kotlin-scripting-compiler-embeddable-") -> {
                boundaries += "KOTLIN_SCRIPTING_PLUGIN_OMITTED_FOR_KT_ANALYSIS"
            }
            name.startsWith("kotlin-serialization-compiler-plugin-embeddable-") -> {
                val expectedName = "kotlin-serialization-compiler-plugin-embeddable-$WORKER_COMPILER_VERSION.jar"
                if (name == expectedName) {
                    effective.add(plugin)
                } else {
                    val compatible = analyzerClasspath
                        .map(Path::toAbsolutePath)
                        .map(Path::normalize)
                        .filter(Path::isRegularFile)
                        .singleOrNull { it.fileName.toString() == expectedName }
                        ?: throw WorkerFailure(
                            "UNSUPPORTED_COMPILER_PLUGIN_ABI",
                            "analyzer-compatible Kotlin serialization compiler plugin is unavailable",
                        )
                    effective.add(compatible)
                    boundaries += "KOTLIN_SERIALIZATION_PLUGIN_REBOUND_TO_ANALYZER_PATCH"
                }
            }
            declaredCompilerVersion != WORKER_COMPILER_VERSION -> throw WorkerFailure(
                "UNSUPPORTED_COMPILER_PLUGIN_ABI",
                "compiler plugin ${plugin.fileName} targets Kotlin $declaredCompilerVersion but analyzer is $WORKER_COMPILER_VERSION",
            )
            else -> effective.add(plugin)
        }
    }
    return EffectiveCompilerPluginPlan(
        effective.distinct().sortedBy { it.toString() },
        boundaries.sorted(),
    )
}

private fun JsonElement?.safeString(): String? =
    (this as? JsonPrimitive)?.takeIf(JsonPrimitive::isString)?.contentOrNull

private fun JsonElement?.safeInt(): Int? = (this as? JsonPrimitive)?.intOrNull

internal fun compilerRangeToUtf8Bytes(source: String, start: Int, end: Int): IntRange? {
    if (start < 0 || end < start || end > source.length) return null
    if (start > 0 && start < source.length && Character.isLowSurrogate(source[start]) && Character.isHighSurrogate(source[start - 1])) return null
    if (end > 0 && end < source.length && Character.isLowSurrogate(source[end]) && Character.isHighSurrogate(source[end - 1])) return null
    val byteStart = source.substring(0, start).toByteArray(Charsets.UTF_8).size
    val byteEnd = byteStart + source.substring(start, end).toByteArray(Charsets.UTF_8).size
    return byteStart until byteEnd
}

internal fun stableBoundaryDigest(value: JsonElement): String = sha(canonicalJson(value).toByteArray())

internal fun repositoryRelativeCompilerPath(repo: Path, raw: String): String? {
    if (raw.isBlank()) return null
    val parsed = runCatching { Path.of(raw) }.getOrNull() ?: return null
    val candidate = (if (parsed.isAbsolute) parsed else repo.resolve(parsed)).normalize()
    val canonicalRepo = repo.toAbsolutePath().normalize()
    if (!candidate.startsWith(canonicalRepo)) return null
    return runCatching { canonicalRepo.relativize(candidate).invariantSeparatorsPathString }
        .getOrNull()
        ?.takeIf(String::isNotEmpty)
        ?.takeUnless { it == ".." || it.startsWith("../") || it.startsWith('/') }
}

internal fun repositorySourceFile(repo: Path, raw: String): Path {
    val canonicalRepo = repo.toRealPath()
    val parsed = runCatching { Path.of(raw) }.getOrElse {
        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "Kotlin source path is invalid")
    }
    val candidate = (if (parsed.isAbsolute) parsed else canonicalRepo.resolve(parsed))
        .toAbsolutePath()
        .normalize()
    if (!candidate.startsWith(canonicalRepo) || candidate == canonicalRepo) {
        throw WorkerFailure(
            "UNSUPPORTED_PROJECT_CONFIGURATION",
            "Kotlin source path is outside the repository",
        )
    }
    var current = canonicalRepo
    val components = canonicalRepo.relativize(candidate).toList()
    components.forEachIndexed { index, component ->
        current = current.resolve(component)
        if (!Files.exists(current, java.nio.file.LinkOption.NOFOLLOW_LINKS) || Files.isSymbolicLink(current)) {
            throw WorkerFailure(
                "UNSUPPORTED_PROJECT_CONFIGURATION",
                "Kotlin source path is absent or symbolic",
            )
        }
        val final = index == components.lastIndex
        if (final && !Files.isRegularFile(current, java.nio.file.LinkOption.NOFOLLOW_LINKS) ||
            !final && !Files.isDirectory(current, java.nio.file.LinkOption.NOFOLLOW_LINKS)
        ) {
            throw WorkerFailure(
                "UNSUPPORTED_PROJECT_CONFIGURATION",
                "Kotlin source path contains a non-regular component",
            )
        }
    }
    val canonical = current.toRealPath()
    if (canonical != candidate || !canonical.startsWith(canonicalRepo)) {
        throw WorkerFailure(
            "UNSUPPORTED_PROJECT_CONFIGURATION",
            "Kotlin source path does not have a contained canonical identity",
        )
    }
    return canonical
}

internal fun validateProjectModelSourceFiles(repo: Path, model: JsonObject): JsonObject = buildJsonObject {
    model.forEach { (key, value) ->
        if (key !in setOf("sourceFiles", "analysisSourceFiles")) {
            put(key, value)
            return@forEach
        }
        val sources = value as? JsonArray ?: throw WorkerFailure(
            "UNSUPPORTED_PROJECT_CONFIGURATION",
            "Kotlin build model $key is not an array",
        )
        putJsonArray(key) {
            sources.forEach { source ->
                val raw = source.safeString() ?: throw WorkerFailure(
                    "UNSUPPORTED_PROJECT_CONFIGURATION",
                    "Kotlin build model source path is not a string",
                )
                add(JsonPrimitive(repositorySourceFile(repo, raw).toString()))
            }
        }
    }
    if ("sourceFiles" !in model) {
        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "Kotlin build model has no sourceFiles")
    }
}

internal fun descriptorUnsupportedReason(raw: JsonObject, file: String?, source: String?): String? {
    val identity = raw["symbolIdentity"].safeString().orEmpty()
    val owner = raw["ownerIdentity"].safeString().orEmpty()
    val kind = raw["declarationKind"].safeString().orEmpty()
    val start = raw["start"].safeInt() ?: -1
    val end = raw["end"].safeInt() ?: -1
    return when {
        file.isNullOrEmpty() -> "INVALID_DESCRIPTOR_SOURCE_PATH"
        identity.isEmpty() || owner.isEmpty() -> "INVALID_DESCRIPTOR_IDENTITY"
        kind !in SUPPORTED_DESCRIPTOR_KINDS -> "UNKNOWN_DECLARATION_KIND"
        source == null -> "DESCRIPTOR_SOURCE_NOT_IN_COMPILATION"
        compilerRangeToUtf8Bytes(source, start, end) == null -> "INVALID_DESCRIPTOR_SOURCE_RANGE"
        raw["visibility"].safeString() !in SUPPORTED_VISIBILITIES -> "UNKNOWN_VISIBILITY"
        raw["effectiveVisibility"].safeString() !in SUPPORTED_EFFECTIVE_VISIBILITIES -> "UNKNOWN_EFFECTIVE_VISIBILITY"
        raw["modality"].safeString() !in SUPPORTED_MODALITIES -> "UNKNOWN_MODALITY"
        rawContainsUnresolvedCompilerType(raw) -> "UNRESOLVED_DESCRIPTOR_TYPE"
        else -> null
    }
}

private fun rawContainsUnresolvedCompilerType(raw: JsonObject): Boolean {
    fun unresolved(value: JsonElement, typeContext: Boolean = false): Boolean = when (value) {
        is JsonObject -> {
            val hasNestedType = value.keys.any { key ->
                key.contains("type", ignoreCase = true) || key == "bounds"
            }
            typeContext && value.isNotEmpty() && !hasNestedType || value.any { (key, child) ->
                unresolved(child, key.contains("type", ignoreCase = true) || key == "bounds")
            }
        }
        is JsonArray -> value.any { unresolved(it, typeContext) }
        is JsonPrimitive -> typeContext && (!value.isString || value.content.let { rendered ->
            rendered.isBlank() || rendered.contains("<unresolved>", ignoreCase = true) ||
                rendered.contains("<ERROR", ignoreCase = true) || rendered.contains("<unknown>", ignoreCase = true) ||
                rendered.contains("..") || rendered.contains('!')
        })
        else -> false
    }
    return unresolved(raw)
}

internal fun parseCompilerFactLines(lines: List<String>): List<JsonObject> = lines
    .filter(String::isNotBlank)
    .flatMap { line ->
        runCatching {
            Json.parseToJsonElement(line).jsonObject.also { parsed ->
                require(parsed["recordType"].safeString() != null) { "compiler fact has no string recordType" }
            }
        }.fold(
            onSuccess = ::listOf,
            onFailure = {
                val digest = sha(line.toByteArray())
                listOf(
                    buildJsonObject {
                        put("recordType", "DECLARATION_DESCRIPTOR_BOUNDARY")
                        put("schema", "declaration-descriptor-boundary/0.1")
                        put("stage", "NORMALIZE")
                        put("code", "MALFORMED_COMPILER_FACT_ROW")
                        put("resolution", "UNKNOWN")
                        put("provider", "COMPILER_DESCRIPTOR_NORMALIZER")
                        put("rawRowHash", digest)
                    },
                    buildJsonObject {
                        put("recordType", "DECLARATION_RELATION_BOUNDARY")
                        put("schema", "declaration-relation-boundary/0.1")
                        put("stage", "NORMALIZE")
                        put("code", "MALFORMED_COMPILER_FACT_ROW")
                        put("resolution", "UNKNOWN")
                        put("provider", "COMPILER_RELATION_NORMALIZER")
                        put("rawRowHash", digest)
                    },
                )
            },
        )
    }

internal const val K1_BUILD_STATE_ROOT_ENV = "CODECLEW_K1_BUILD_STATE_ROOT"
internal const val K1_BUILD_STATE_SEED_FILE = "CODECLEW_K1_BUILD_STATE_SEED"
internal const val K1_BUILD_STATE_MANIFEST_FILE = "CODECLEW_K1_BUILD_STATE_MANIFEST.json"
internal const val K1_BUILD_STATE_MANIFEST_SCHEMA = "codeclew.kotlin-k1-build-state-manifest/0.1"

internal data class BuildStateLayout(
    val mode: String,
    val gradleUserHome: Path,
    val mavenLocalRepository: Path,
    val authorityRoot: Path?,
    val sealedFiles: List<JsonObject>,
    val seedDigest: String?,
    val manifestDigest: String?,
    val markerBytesDigest: String?,
    val namespaceDigest: String,
) {
    fun semanticIdentity(): JsonObject = buildJsonObject {
        put("mode", mode)
        put("runtimeIsolation", if (mode == "EXTERNAL") "PRIVATE_DISPOSABLE_COPY" else "REPOSITORY_OWNED_LEGACY")
        seedDigest?.let { put("seedDigest", it) }
        manifestDigest?.let { put("manifestDigest", it) }
        markerBytesDigest?.let { put("markerBytesDigest", it) }
        put("namespaceDigest", namespaceDigest)
        put("gradleUserHome", "gradle-user-home")
        put("mavenLocalRepository", "maven-repository")
        put("homeCredentials", if (mode == "EXTERNAL") "ISOLATED" else "INHERITED_LEGACY")
    }
}

private fun validStateComponent(component: String) =
    component.isNotBlank() && component != "." && component != ".." && '/' !in component && '\\' !in component

private fun realStateSubdirectory(root: Path, vararg components: String): Path {
    var current = root.toRealPath()
    for (component in components) {
        require(validStateComponent(component)) { "invalid build-state component" }
        val next = current.resolve(component)
        if (Files.exists(next, java.nio.file.LinkOption.NOFOLLOW_LINKS)) {
            if (Files.isSymbolicLink(next) || !Files.isDirectory(next, java.nio.file.LinkOption.NOFOLLOW_LINKS)) {
                throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "build-state path is not a real directory")
            }
        } else {
            Files.createDirectory(next)
        }
        current = next.toRealPath()
    }
    return current
}

private fun existingRealStateSubdirectory(root: Path, component: String): Path {
    require(validStateComponent(component)) { "invalid build-state component" }
    val child = root.resolve(component)
    if (!Files.exists(child, java.nio.file.LinkOption.NOFOLLOW_LINKS) || Files.isSymbolicLink(child) ||
        !Files.isDirectory(child, java.nio.file.LinkOption.NOFOLLOW_LINKS)
    ) {
        throw WorkerFailure(
            "UNSUPPORTED_PROJECT_CONFIGURATION",
            "external K1 build-state PREPARE is incomplete: $component is absent or not a real directory",
        )
    }
    val canonical = child.toRealPath()
    if (canonical != child || !canonical.startsWith(root)) {
        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state directory escapes its root")
    }
    return canonical
}

private fun isSha256(value: String?): Boolean =
    value?.matches(Regex("sha256:[0-9a-f]{64}")) == true

private fun verifiedBuildStateFiles(rootName: String, root: Path): List<JsonObject> =
    Files.walk(root).use { paths ->
        paths.map { path ->
            if (Files.isSymbolicLink(path)) {
                throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state tree contains a symbolic link")
            }
            if (path == root || Files.isDirectory(path, java.nio.file.LinkOption.NOFOLLOW_LINKS)) return@map null
            if (!Files.isRegularFile(path, java.nio.file.LinkOption.NOFOLLOW_LINKS)) {
                throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state tree contains a special file")
            }
            val relative = root.relativize(path).invariantSeparatorsPathString
            buildJsonObject {
                put("root", rootName)
                put("path", relative)
                put("size", Files.size(path))
                put("sha256", sha(path.readBytes()))
            }
        }.filter { it != null }.map { it!! }
            .sorted(compareBy({ it["root"]!!.jsonPrimitive.content }, { it["path"]!!.jsonPrimitive.content }))
            .toList()
    }

private fun buildStateTreeDigest(files: List<JsonObject>, rootName: String): String = sha(buildString {
    files.filter { it["root"]?.jsonPrimitive?.content == rootName }.forEach { row ->
        append(row["path"]!!.jsonPrimitive.content).append('\u0000')
        append(row["size"]!!.jsonPrimitive.long).append('\u0000')
        append(row["sha256"]!!.jsonPrimitive.content).append('\u0000')
    }
}.toByteArray())

private data class VerifiedBuildStateManifest(
    val seedDigest: String,
    val manifestDigest: String,
    val markerBytesDigest: String,
    val sealedFiles: List<JsonObject>,
)

private fun verifiedManifestFile(root: Path, relative: String): Path {
    var current = root
    relative.split('/').forEach { component ->
        if (!validStateComponent(component)) {
            throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state manifest path is invalid")
        }
        current = current.resolve(component)
        if (!Files.exists(current, java.nio.file.LinkOption.NOFOLLOW_LINKS) || Files.isSymbolicLink(current)) {
            throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state manifest member is absent or symbolic")
        }
    }
    if (!Files.isRegularFile(current, java.nio.file.LinkOption.NOFOLLOW_LINKS) ||
        !current.toRealPath().startsWith(root)
    ) {
        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state manifest member is not a real file")
    }
    return current
}

private fun verifyExternalBuildStateManifest(
    root: Path,
    gradleUserHome: Path,
    mavenLocalRepository: Path,
): VerifiedBuildStateManifest {
    val manifestPath = root.resolve(K1_BUILD_STATE_MANIFEST_FILE)
    val markerPath = root.resolve(K1_BUILD_STATE_SEED_FILE)
    for ((path, label) in listOf(manifestPath to "manifest", markerPath to "seed marker")) {
        if (!Files.exists(path, java.nio.file.LinkOption.NOFOLLOW_LINKS) || Files.isSymbolicLink(path) ||
            !Files.isRegularFile(path, java.nio.file.LinkOption.NOFOLLOW_LINKS)
        ) {
            throw WorkerFailure(
                "UNSUPPORTED_PROJECT_CONFIGURATION",
                "external K1 build-state $label is absent or not a real file",
            )
        }
    }
    if (Files.size(manifestPath) !in 1..64L * 1024 * 1024 || Files.size(markerPath) != 72L) {
        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state authority file size is invalid")
    }
    val manifestBytes = manifestPath.readBytes()
    val manifest = runCatching { Json.parseToJsonElement(manifestBytes.decodeToString()).jsonObject }
        .getOrElse {
            throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state manifest is invalid JSON")
        }
    val expectedKeys = setOf(
        "schema", "seriesId", "cohort", "toolchain", "repositories",
        "gradleUserHomeTreeDigest", "mavenLocalRepositoryTreeDigest", "files", "seedDigest",
    )
    if (manifest.keys != expectedKeys || manifest["schema"]?.jsonPrimitive?.contentOrNull != K1_BUILD_STATE_MANIFEST_SCHEMA ||
        manifest["seriesId"]?.jsonPrimitive?.contentOrNull.isNullOrBlank() ||
        manifest["cohort"]?.jsonPrimitive?.contentOrNull !in setOf("QUALIFICATION", "BLIND_HOLDOUT", "FIXTURE")
    ) {
        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state manifest envelope is invalid")
    }
    val canonicalManifestBytes = (canonicalJson(manifest) + "\n").toByteArray()
    if (!manifestBytes.contentEquals(canonicalManifestBytes)) {
        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state manifest is not canonical JSON plus newline")
    }
    val manifestDigest = sha(manifestBytes)
    val markerBytes = markerPath.readBytes()
    if (!markerBytes.contentEquals("$manifestDigest\n".toByteArray())) {
        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state marker does not seal the exact manifest")
    }
    val markerBytesDigest = sha(markerBytes)
    val seedDigest = manifest["seedDigest"]?.jsonPrimitive?.contentOrNull
    val seedBody = buildJsonObject {
        manifest.forEach { (key, value) -> put(key, if (key == "seedDigest") JsonPrimitive("") else value) }
    }
    if (!isSha256(seedDigest) || seedDigest != sha((canonicalJson(seedBody) + "\n").toByteArray())) {
        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state seed digest is invalid")
    }
    val toolchain = manifest["toolchain"] as? JsonObject
    if (toolchain.isNullOrEmpty() || toolchain.values.any { !isSha256(it.jsonPrimitive.contentOrNull) }) {
        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state toolchain identity is invalid")
    }
    val repositories = manifest["repositories"] as? JsonArray
    if (repositories.isNullOrEmpty() || repositories.any { item ->
            val row = item as? JsonObject ?: return@any true
            row.keys != setOf(
                "entry", "commit", "gitTree", "selectedCompilation", "buildDsl", "prepareArgvSha256", "exitCode",
            ) || row["entry"]?.jsonPrimitive?.contentOrNull.isNullOrBlank() ||
                row["selectedCompilation"]?.jsonPrimitive?.contentOrNull.isNullOrBlank() ||
                row["buildDsl"]?.jsonPrimitive?.contentOrNull.isNullOrBlank() ||
                row["commit"]?.jsonPrimitive?.contentOrNull?.matches(Regex("[0-9a-f]{40}")) != true ||
                row["gitTree"]?.jsonPrimitive?.contentOrNull?.matches(Regex("[0-9a-f]{40}")) != true ||
                !isSha256(row["prepareArgvSha256"]?.jsonPrimitive?.contentOrNull) ||
                row["exitCode"]?.jsonPrimitive?.intOrNull != 0
        }
    ) {
        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state repository authority is invalid")
    }
    val declaredFiles = manifest["files"] as? JsonArray
    if (declaredFiles == null || declaredFiles.any { item ->
            val row = item as? JsonObject ?: return@any true
            row.keys != setOf("root", "path", "size", "sha256") ||
                row["root"]?.jsonPrimitive?.contentOrNull !in setOf("gradle-user-home", "maven-repository") ||
                row["path"]?.jsonPrimitive?.contentOrNull.let { path ->
                    path.isNullOrEmpty() || path.startsWith('/') || path.split('/').any { it.isEmpty() || it == "." || it == ".." }
                } || row["size"]?.jsonPrimitive?.longOrNull?.let { it < 0 } != false ||
                !isSha256(row["sha256"]?.jsonPrimitive?.contentOrNull)
        }
    ) {
        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state file manifest is invalid")
    }
    val fileRows = declaredFiles.map(JsonElement::jsonObject)
    val sortedRows = fileRows.sortedWith(compareBy(
        { it["root"]!!.jsonPrimitive.content },
        { it["path"]!!.jsonPrimitive.content },
    ))
    if (fileRows != sortedRows || fileRows.map { it["root"]!!.jsonPrimitive.content to it["path"]!!.jsonPrimitive.content }.toSet().size != fileRows.size) {
        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state file manifest is not uniquely ordered")
    }
    val actualFiles = verifiedBuildStateFiles("gradle-user-home", gradleUserHome) +
        verifiedBuildStateFiles("maven-repository", mavenLocalRepository)
    if (fileRows != actualFiles) {
        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state live seed tree differs from its manifest")
    }
    fileRows.forEach { row ->
        val stateRoot = when (row["root"]!!.jsonPrimitive.content) {
            "gradle-user-home" -> gradleUserHome
            "maven-repository" -> mavenLocalRepository
            else -> error("validated build-state root")
        }
        val path = verifiedManifestFile(stateRoot, row["path"]!!.jsonPrimitive.content)
        if (Files.size(path) != row["size"]!!.jsonPrimitive.long || sha(path.readBytes()) != row["sha256"]!!.jsonPrimitive.content) {
            throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state seeded file differs from its manifest")
        }
    }
    val gradleTreeDigest = buildStateTreeDigest(fileRows, "gradle-user-home")
    val mavenTreeDigest = buildStateTreeDigest(fileRows, "maven-repository")
    if (manifest["gradleUserHomeTreeDigest"]?.jsonPrimitive?.contentOrNull != gradleTreeDigest ||
        manifest["mavenLocalRepositoryTreeDigest"]?.jsonPrimitive?.contentOrNull != mavenTreeDigest
    ) {
        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state tree digest is invalid")
    }
    return VerifiedBuildStateManifest(seedDigest, manifestDigest, markerBytesDigest, fileRows)
}

internal fun prepareFixtureBuildState(root: Path) {
    val canonicalRoot = root.toRealPath()
    realStateSubdirectory(canonicalRoot, "gradle-user-home")
    realStateSubdirectory(canonicalRoot, "maven-repository")
    val emptyTreeDigest = buildStateTreeDigest(emptyList(), "gradle-user-home")
    val body = buildJsonObject {
        put("schema", K1_BUILD_STATE_MANIFEST_SCHEMA)
        put("seriesId", "K1_WORKER_FIXTURE")
        put("cohort", "FIXTURE")
        putJsonObject("toolchain") { put("fixture", sha("fixture-toolchain".toByteArray())) }
        putJsonArray("repositories") {
            add(buildJsonObject {
                put("entry", "fixture")
                put("commit", "a".repeat(40))
                put("gitTree", "b".repeat(40))
                put("selectedCompilation", ":/main")
                put("buildDsl", "FIXTURE")
                put("prepareArgvSha256", sha("fixture-prepare".toByteArray()))
                put("exitCode", 0)
            })
        }
        put("gradleUserHomeTreeDigest", emptyTreeDigest)
        put("mavenLocalRepositoryTreeDigest", emptyTreeDigest)
        putJsonArray("files") {}
        put("seedDigest", "")
    }
    val seedDigest = sha((canonicalJson(body) + "\n").toByteArray())
    val manifest = buildJsonObject {
        body.forEach { (key, value) -> put(key, if (key == "seedDigest") JsonPrimitive(seedDigest) else value) }
    }
    val bytes = (canonicalJson(manifest) + "\n").toByteArray()
    canonicalRoot.resolve(K1_BUILD_STATE_MANIFEST_FILE).writeBytes(bytes)
    canonicalRoot.resolve(K1_BUILD_STATE_SEED_FILE).writeText("${sha(bytes)}\n")
}

private fun buildStateNamespaceDigest(repo: Path, seedDigest: String?): String {
    val canonicalRepo = repo.toRealPath()
    val inputs = Files.walk(canonicalRepo).use { paths ->
        paths.filter { Files.isRegularFile(it, java.nio.file.LinkOption.NOFOLLOW_LINKS) }
            .map { file -> canonicalRepo.relativize(file).invariantSeparatorsPathString to file }
            .filter { (relative, _) ->
                val components = relative.split('/')
                if (components.any { it in setOf(".git", ".gradle", ".kotlin", ".semantic-thread", "build", "target") }) {
                    false
                } else {
                    val name = relative.substringAfterLast('/')
                    name in setOf(
                        "settings.gradle", "settings.gradle.kts", "build.gradle", "build.gradle.kts",
                        "gradle.properties", "libs.versions.toml", "gradle-wrapper.properties", "gradle-wrapper.jar",
                        "gradlew", "gradlew.bat", "pom.xml", "mvnw", "mvnw.cmd",
                    ) || relative.startsWith(".mvn/") || relative.startsWith("buildSrc/") ||
                        relative.startsWith("build-logic/") || relative.startsWith("gradle/")
                }
            }
            .sorted(compareBy { it.first })
            .map { (relative, file) -> "$relative:${sha(file.readBytes())}" }
            .toList()
    }
    return sha(buildString {
        append("k1-build-state-namespace/0.1\u0000")
        append(seedDigest ?: "LEGACY_REPOSITORY_OWNED").append('\u0000')
        inputs.forEach { append(it).append('\u0000') }
    }.toByteArray())
}

internal fun externalBuildStateLayout(repo: Path, configuredRoot: Path): BuildStateLayout {
    val canonicalRepo = repo.toRealPath()
    if (!configuredRoot.isAbsolute || configuredRoot.normalize() != configuredRoot) {
        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state root must be absolute and normalized")
    }
    if (!Files.exists(configuredRoot, java.nio.file.LinkOption.NOFOLLOW_LINKS) ||
        Files.isSymbolicLink(configuredRoot) ||
        !Files.isDirectory(configuredRoot, java.nio.file.LinkOption.NOFOLLOW_LINKS)
    ) {
        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state root must be an existing real directory")
    }
    val root = configuredRoot.toRealPath()
    if (root != configuredRoot || root.startsWith(canonicalRepo) || canonicalRepo.startsWith(root)) {
        throw WorkerFailure(
            "UNSUPPORTED_PROJECT_CONFIGURATION",
            "external K1 build-state root must be non-symlinked and outside/not containing the repository",
        )
    }
    val gradleUserHome = existingRealStateSubdirectory(root, "gradle-user-home")
    val mavenLocalRepository = existingRealStateSubdirectory(root, "maven-repository")
    val verifiedManifest = verifyExternalBuildStateManifest(root, gradleUserHome, mavenLocalRepository)
    val namespaceDigest = buildStateNamespaceDigest(canonicalRepo, verifiedManifest.seedDigest)
    return BuildStateLayout(
        mode = "EXTERNAL",
        gradleUserHome = gradleUserHome,
        mavenLocalRepository = mavenLocalRepository,
        authorityRoot = root,
        sealedFiles = verifiedManifest.sealedFiles,
        seedDigest = verifiedManifest.seedDigest,
        manifestDigest = verifiedManifest.manifestDigest,
        markerBytesDigest = verifiedManifest.markerBytesDigest,
        namespaceDigest = namespaceDigest,
    )
}

internal fun buildStateLayout(repo: Path): BuildStateLayout {
    val configured = System.getenv(K1_BUILD_STATE_ROOT_ENV)?.takeIf(String::isNotBlank)
    if (configured != null) return externalBuildStateLayout(repo, Path.of(configured))
    val namespaceDigest = buildStateNamespaceDigest(repo, null)
    return BuildStateLayout(
        mode = "LEGACY_REPOSITORY_OWNED",
        gradleUserHome = repoOwnedStateDirectory(repo, ".gradle"),
        mavenLocalRepository = repoOwnedStateDirectory(repo, ".semantic-thread", "maven-repository"),
        authorityRoot = null,
        sealedFiles = emptyList(),
        seedDigest = null,
        manifestDigest = null,
        markerBytesDigest = null,
        namespaceDigest = namespaceDigest,
    )
}

private fun recheckExternalBuildStateSeal(state: BuildStateLayout) {
    if (state.mode != "EXTERNAL") return
    val root = state.authorityRoot
        ?: throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state has no authority root")
    val manifest = root.resolve(K1_BUILD_STATE_MANIFEST_FILE)
    val marker = root.resolve(K1_BUILD_STATE_SEED_FILE)
    if (Files.isSymbolicLink(manifest) || Files.isSymbolicLink(marker) ||
        !manifest.isRegularFile() || !marker.isRegularFile() ||
        sha(manifest.readBytes()) != state.manifestDigest || sha(marker.readBytes()) != state.markerBytesDigest
    ) {
        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "external K1 build-state authority changed during worker lifetime")
    }
}

private fun materializeBuildStateRuntime(authority: BuildStateLayout): BuildStateLayout {
    require(authority.mode == "EXTERNAL" && authority.authorityRoot != null) {
        "only a verified external build state can be materialized"
    }
    val runtimeRoot = Files.createTempDirectory("semantic-thread-k1-build-state-runtime").toRealPath()
    try {
        val gradleRuntime = runtimeRoot.resolve("gradle-user-home").also { Files.createDirectory(it) }
        val mavenRuntime = runtimeRoot.resolve("maven-repository").also { Files.createDirectory(it) }
        authority.sealedFiles.forEach { row ->
            val rootName = row["root"]!!.jsonPrimitive.content
            val sourceRoot = when (rootName) {
                "gradle-user-home" -> authority.authorityRoot.resolve(rootName)
                "maven-repository" -> authority.authorityRoot.resolve(rootName)
                else -> error("verified build-state root")
            }
            val destinationRoot = if (rootName == "gradle-user-home") gradleRuntime else mavenRuntime
            val relative = row["path"]!!.jsonPrimitive.content
            val source = verifiedManifestFile(sourceRoot, relative)
            val bytes = source.readBytes()
            if (bytes.size.toLong() != row["size"]!!.jsonPrimitive.long ||
                sha(bytes) != row["sha256"]!!.jsonPrimitive.content
            ) {
                throw WorkerFailure(
                    "UNSUPPORTED_PROJECT_CONFIGURATION",
                    "external K1 build-state seed changed while creating its runtime copy",
                )
            }
            var destinationDirectory = destinationRoot
            relative.substringBeforeLast('/', "").split('/').filter(String::isNotEmpty).forEach { component ->
                val next = destinationDirectory.resolve(component)
                if (Files.exists(next, java.nio.file.LinkOption.NOFOLLOW_LINKS)) {
                    if (Files.isSymbolicLink(next) || !Files.isDirectory(next, java.nio.file.LinkOption.NOFOLLOW_LINKS)) {
                        throw WorkerFailure(
                            "UNSUPPORTED_PROJECT_CONFIGURATION",
                            "external K1 build-state runtime parent is not a real directory",
                        )
                    }
                } else {
                    Files.createDirectory(next)
                }
                destinationDirectory = next
            }
            val destination = destinationRoot.resolve(relative)
            if (Files.exists(destination, java.nio.file.LinkOption.NOFOLLOW_LINKS)) {
                throw WorkerFailure(
                    "UNSUPPORTED_PROJECT_CONFIGURATION",
                    "external K1 build-state runtime file is duplicated",
                )
            }
            Files.createFile(destination).writeBytes(bytes)
        }
        val copiedFiles = verifiedBuildStateFiles("gradle-user-home", gradleRuntime) +
            verifiedBuildStateFiles("maven-repository", mavenRuntime)
        if (copiedFiles != authority.sealedFiles) {
            throw WorkerFailure(
                "UNSUPPORTED_PROJECT_CONFIGURATION",
                "external K1 build-state runtime copy differs from its sealed seed",
            )
        }
        return authority.copy(
            gradleUserHome = gradleRuntime,
            mavenLocalRepository = mavenRuntime,
        )
    } catch (failure: Throwable) {
        runtimeRoot.toFile().deleteRecursively()
        throw failure
    }
}

internal fun repoOwnedStateDirectory(repo: Path, vararg components: String): Path {
    val canonicalRepo = repo.toRealPath()
    var current = canonicalRepo
    for (component in components) {
        require(validStateComponent(component)) {
            "invalid repository-owned state component"
        }
        val next = current.resolve(component)
        if (Files.exists(next, java.nio.file.LinkOption.NOFOLLOW_LINKS)) {
            if (Files.isSymbolicLink(next) || !Files.isDirectory(next, java.nio.file.LinkOption.NOFOLLOW_LINKS)) {
                throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "repository-owned state path is not a real directory")
            }
        } else {
            Files.createDirectory(next)
        }
        current = next
    }
    val resolved = current.toRealPath()
    if (resolved == canonicalRepo || !resolved.startsWith(canonicalRepo)) {
        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "repository-owned state path escapes repository")
    }
    return resolved
}

internal fun gradleModelCommand(
    wrapper: Path,
    repo: Path,
    gradleUserHome: Path,
    projectCacheDirectory: Path,
    initScript: Path,
    compileTask: String,
    modelTask: String,
    reuseDaemon: Boolean = false,
): List<String> = listOf(
    wrapper.toString(),
    "-p", repo.toString(),
    "--offline",
    "--gradle-user-home", gradleUserHome.toString(),
    "--project-cache-dir", projectCacheDirectory.toString(),
    *(if (reuseDaemon) emptyArray() else arrayOf("--no-daemon")),
    "--quiet",
    "-Duser.home=$gradleUserHome",
    "-Pkotlin.project.persistent.dir=${projectCacheDirectory.resolve("kotlin")}",
    "-I", initScript.toString(),
    "-Dsemantic.thread.compileTask=$compileTask",
    modelTask,
)

internal enum class ProjectModelBuildTool {
    GRADLE,
    MAVEN,
}

internal fun sanitizedProjectModelProcess(
    command: List<String>,
    repo: Path,
    isolatedHome: Path? = null,
    buildTool: ProjectModelBuildTool? = null,
    seededEnvironment: Map<String, String>? = null,
): ProcessBuilder =
    ProcessBuilder(command).also { builder ->
        if (seededEnvironment != null) {
            builder.environment().clear()
            builder.environment().putAll(seededEnvironment)
        }
        require((isolatedHome == null) == (buildTool == null)) {
            "isolated project-model home and build tool must be supplied together"
        }
        for (key in listOf(
            "CODECLEW_K1_BUILD_STATE_ROOT",
            "CODECLEW_K2_INDEX_ROOT",
            "GRADLE_OPTS",
            "GRADLE_USER_HOME",
            "MAVEN_OPTS",
            "MAVEN_ARGS",
            "MAVEN_CONFIG",
            "MAVEN_USER_HOME",
            "JAVA_OPTS",
            "JAVA_TOOL_OPTIONS",
            "JDK_JAVA_OPTIONS",
            "_JAVA_OPTIONS",
        )) {
            builder.environment().remove(key)
        }
        if (isolatedHome != null) {
            builder.environment()["HOME"] = isolatedHome.toString()
            builder.environment()["USERPROFILE"] = isolatedHome.toString()
            when (buildTool) {
                ProjectModelBuildTool.GRADLE ->
                    builder.environment()["GRADLE_USER_HOME"] = isolatedHome.toString()
                ProjectModelBuildTool.MAVEN ->
                    builder.environment()["MAVEN_USER_HOME"] = isolatedHome.toString()
                null -> error("unreachable project-model build tool")
            }
        }
        builder.directory(repo.toFile()).redirectErrorStream(true)
    }

internal class Worker(
    private val configuredBuildStateRoot: Path? =
        System.getenv(K1_BUILD_STATE_ROOT_ENV)?.takeIf(String::isNotBlank)?.let(Path::of),
) : AutoCloseable {
    private val disposable = Disposer.newDisposable("semantic-thread-worker")
    private val environment = KotlinCoreEnvironment.createForProduction(
        disposable, CompilerConfiguration(), EnvironmentConfigFiles.JVM_CONFIG_FILES
    )
    init {
        val area = ApplicationManager.getApplication().extensionArea
        if (!area.hasExtensionPoint(TreeCopyHandler.EP_NAME)) {
            area.registerExtensionPoint(TreeCopyHandler.EP_NAME.name, TreeCopyHandler::class.java.name, ExtensionPoint.Kind.INTERFACE)
        }
    }
    private val factory = KtPsiFactory(environment.project, markGenerated = false)
    private val json = Json { ignoreUnknownKeys = false; explicitNulls = false }
    private val analysisCache = mutableMapOf<String, K2Analysis>()
    private val projectModelCache = mutableMapOf<String, JsonObject>()
    private val externalBuildStateCache = mutableMapOf<Path, BuildStateLayout>()
    private val externalBuildStateRuntimeRoots = mutableSetOf<Path>()
    private var requestCacheRequests = 0L
    private var requestCacheHits = 0L
    private var requestPsiParseMicros = 0L
    private var requestK2AnalysisMicros = 0L
    private var requestFirExtractionMicros = 0L

    private fun stateFor(repo: Path): BuildStateLayout {
        val canonicalRepo = repo.toRealPath()
        val configured = configuredBuildStateRoot ?: return buildStateLayout(canonicalRepo)
        return externalBuildStateCache.getOrPut(canonicalRepo) {
            materializeBuildStateRuntime(externalBuildStateLayout(canonicalRepo, configured)).also { state ->
                state.gradleUserHome.parent?.let(externalBuildStateRuntimeRoots::add)
            }
        }.also(::recheckExternalBuildStateSeal)
    }

    fun handle(kind: Int, payload: ByteArray): String {
        IncrementalK2Runtime.reset()
        requestCacheRequests = 0
        requestCacheHits = 0
        requestPsiParseMicros = 0
        requestK2AnalysisMicros = 0
        requestFirExtractionMicros = 0
        val processingStarted = System.nanoTime()
        val request = if (payload.isEmpty()) buildJsonObject {} else json.parseToJsonElement(payload.decodeToString()).jsonObject
        val result = when (kind) {
            2 -> inspect(Path.of(request.requiredString("repo")).toRealPath(), request["compilation"]?.takeUnless { it is JsonNull }?.jsonPrimitive?.content)
            3 -> index(Path.of(request.requiredString("repo")).toRealPath(), request["compilation"]?.takeUnless { it is JsonNull }?.jsonPrimitive?.content, request["syntaxOnly"]?.jsonPrimitive?.booleanOrNull == true, request["files"]?.jsonArray?.map { it.jsonPrimitive.content }.orEmpty())
            4 -> resolveSymbol(Path.of(request.requiredString("repo")).toRealPath(), request.requiredString("symbol"), request["compilation"]?.jsonPrimitive?.content ?: ":/main")
            5 -> resolveExpression(Path.of(request.requiredString("repo")).toRealPath(), request.requiredString("file"), request.requiredInt("offset"), request["compilation"]?.jsonPrimitive?.content ?: ":/main")
            6 -> localGraph(Path.of(request.requiredString("repo")).toRealPath(), request.requiredString("symbol"), request["compilation"]?.jsonPrimitive?.content ?: ":/main")
            7 -> applyEdit(request)
            8 -> validateCandidate(request)
            else -> error("unsupported request kind $kind")
        }
        val processingMicros = (System.nanoTime() - processingStarted) / 1_000
        val response = buildJsonObject {
            result.forEach { (key, value) -> put(key, value) }
            putJsonObject("profiling") {
                put("workerProcessingMicros", processingMicros)
                put("cacheRequests", requestCacheRequests)
                put("cacheHits", requestCacheHits)
                put("psiParseMicros", requestPsiParseMicros)
                put("k2AnalysisMicros", requestK2AnalysisMicros)
                put("firExtractionMicros", requestFirExtractionMicros)
            }
        }
        val incremental = IncrementalK2Runtime.takeProfiling()
        return IncrementalK2Runtime.mergeProfiling(response, incremental).toString()
    }

    private fun inspect(requestedRepo: Path, compilation: String?): JsonObject {
        require(requestedRepo.isDirectory()) { "repository does not exist: $requestedRepo" }
        val repo = requestedRepo.toRealPath()
        val buildModel = cachedProjectModel(repo, compilation)
        val modelFiles = projectModelFiles(repo)
        val sourceFiles = buildModel["sourceFiles"]?.jsonArray?.map { Path.of(it.jsonPrimitive.content) }.orEmpty()
        val sourceRoots = sourceFiles.mapNotNull { sourceRoot(repo, it) }.distinct().sorted()
        val generatedRoots = sourceFiles.filter {
            val normalized = it.normalize()
            normalized.startsWith(repo.resolve("build/generated").normalize()) ||
                normalized.startsWith(repo.resolve("target/generated-sources").normalize())
        }.map { repo.relativize(it.parent).invariantSeparatorsPathString }.distinct().sorted()
        val classpath = buildModel["classpath"]?.jsonArray?.map { normalizeArtifact(repo, Path.of(it.jsonPrimitive.content)) }.orEmpty()
        val requestedPlugins = buildModel["requestedCompilerPlugins"]?.jsonArray
            ?.map { normalizeArtifact(repo, Path.of(it.jsonPrimitive.content)) }
            .orEmpty()
        val plugins = buildModel["compilerPlugins"]?.jsonArray
            ?.map { normalizeArtifact(repo, Path.of(it.jsonPrimitive.content)) }
            .orEmpty()
        val declaredCompilerVersion = buildModel["compilerVersion"]?.jsonPrimitive?.contentOrNull
            ?: throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "declared Kotlin compiler version is unavailable")
        val compilerLine = declaredCompilerVersion.split('.').take(2).joinToString(".")
        val module = buildModel["projectPath"]?.jsonPrimitive?.contentOrNull ?: ":"
        val sourceSet = compilation?.substringAfterLast('/') ?: "main"
        val canonicalCompilation = "$module/$sourceSet"
        if (compilation != null && compilation != canonicalCompilation) {
            throw WorkerFailure(
                "UNSUPPORTED_PROJECT_CONFIGURATION",
                "requested compilation differs from authoritative module/source set",
            )
        }
        val normalized = buildJsonObject {
            put("buildSystem", buildModel["buildSystem"] ?: JsonPrimitive("GRADLE"))
            put("buildLauncher", buildModel["buildLauncher"] ?: JsonPrimitive("./gradlew"))
            put("buildRoot", ".")
            put("projectDirectory", buildModel["projectDir"]?.jsonPrimitive?.contentOrNull?.let { raw ->
                runCatching { repo.relativize(Path.of(raw).toAbsolutePath().normalize()).invariantSeparatorsPathString }.getOrNull()
                    ?.takeUnless { it == ".." || it.startsWith("../") }
            } ?: ".")
            put("platform", buildModel["platform"] ?: JsonPrimitive("JVM"))
            put("module", module); put("sourceSet", sourceSet); put("compilation", canonicalCompilation)
            putJsonArray("sourceRoots") { sourceRoots.forEach(::add) }; putJsonArray("generatedSourceRoots") { generatedRoots.forEach(::add) }
            putJsonArray("compileClasspath") { classpath.forEach(::add) }; putJsonArray("friendPaths") { buildModel["friendPaths"]?.jsonArray?.map { normalizeArtifact(repo, Path.of(it.jsonPrimitive.content)) }?.forEach(::add) }
            put("compilerVersion", WORKER_COMPILER_VERSION)
            put("declaredCompilerVersion", declaredCompilerVersion)
            put("languageVersion", buildModel["languageVersion"]?.takeUnless { it is JsonNull } ?: JsonPrimitive(compilerLine)); put("apiVersion", buildModel["apiVersion"]?.takeUnless { it is JsonNull } ?: JsonPrimitive(compilerLine)); put("jvmTarget", buildModel["jvmTarget"]?.takeUnless { it is JsonNull }?.jsonPrimitive?.content?.removePrefix("JVM_") ?: "21")
            putJsonArray("freeCompilerArguments") { buildModel["freeCompilerArguments"]?.jsonArray?.forEach(::add) }
            putJsonArray("optIns") { buildModel["optIns"]?.jsonArray?.forEach(::add) }
            putJsonArray("requestedCompilerPlugins") { requestedPlugins.forEach(::add) }
            putJsonArray("compilerPlugins") { plugins.forEach(::add) }
            putJsonArray("compilerPluginOptions") { buildModel["compilerPluginOptions"]?.jsonArray?.forEach(::add) }
            put("compileTask", buildModel["compileTask"] ?: JsonPrimitive(":compileKotlin")); putJsonArray("testTasks") { buildModel["tasks"]?.jsonArray?.map { it.jsonPrimitive.content }?.filter { it == "test" || it.endsWith("Test") }?.sorted()?.forEach(::add) }
            put("fieldBoundaries", buildModel["fieldBoundaries"] ?: buildJsonObject {
                listOf("libraries", "friendPaths", "compilerPlugins", "freeCompilerArguments", "optIns", "languageVersion", "apiVersion", "jvmTarget", "compilerVersion").forEach { field -> put(field, "UNAVAILABLE_PROVIDER") }
            })
            put("buildModelBoundaries", buildModel["buildModelBoundaries"] ?: JsonArray(emptyList()))
            put("dependencyCoordinates", buildModel["dependencyCoordinates"] ?: JsonArray(emptyList()))
            put("repositories", buildModel["repositories"] ?: JsonArray(emptyList()))
            put("reactorPoms", buildModel["reactorPoms"] ?: JsonArray(emptyList()))
            put("buildPlugins", buildModel["buildPlugins"] ?: JsonArray(emptyList()))
            put("classpathAuthority", buildModel["classpathAuthority"] ?: buildJsonObject {
                put("chosen", "UNAVAILABLE_PROVIDER")
            })
            put("buildState", stateFor(repo).semanticIdentity())
            put("generatedSourceConfiguration", buildModel["generatedSourceConfiguration"] ?: buildJsonObject {
                putJsonArray("roots") { generatedRoots.forEach(::add) }
                putJsonArray("producers") {}
                put("status", if (generatedRoots.isEmpty()) "NONE_DISCOVERED" else "ROOTS_ONLY")
            })
            buildModel["mavenTestLifecycle"]?.let { put("mavenTestLifecycle", it) }
            put("gradleVersion", buildModel["gradleVersion"] ?: JsonPrimitive("unknown")); put("mavenVersion", buildModel["mavenVersion"] ?: JsonPrimitive("unknown"))
            put("jdkHomeFingerprint", jdkFingerprint(Path.of(buildModel["jdkHome"]?.jsonPrimitive?.content ?: System.getProperty("java.home"))))
            putJsonArray("modelInputs") { modelFiles.map { buildJsonObject { put("path", repo.relativize(it).invariantSeparatorsPathString); put("hash", sha(it.readBytes())) } }.sortedBy { it.toString() }.forEach(::add) }
        }
        val modelHash = sha(normalized.toString().toByteArray())
        val semanticInputManifest = buildJsonObject {
            put("schema", "kotlin-semantic-input-manifest/0.1")
            put("compilation", canonicalCompilation)
            put("declaredCompilerVersion", declaredCompilerVersion)
            put("analyzerCompilerVersion", WORKER_COMPILER_VERSION)
            putJsonArray("orderedCompileClasspath") { classpath.forEach(::add) }
            putJsonArray("orderedFriendPaths") {
                buildModel["friendPaths"]?.jsonArray
                    ?.map { normalizeArtifact(repo, Path.of(it.jsonPrimitive.content)) }
                    ?.forEach(::add)
            }
            putJsonArray("requestedCompilerPlugins") { requestedPlugins.forEach(::add) }
            putJsonArray("orderedCompilerPlugins") { plugins.forEach(::add) }
            putJsonArray("orderedFreeCompilerArguments") { buildModel["freeCompilerArguments"]?.jsonArray?.forEach(::add) }
            putJsonArray("orderedOptIns") { buildModel["optIns"]?.jsonArray?.forEach(::add) }
            putJsonArray("orderedCompilerPluginOptions") { buildModel["compilerPluginOptions"]?.jsonArray?.forEach(::add) }
            put("target", normalized["jvmTarget"] ?: JsonNull)
            put("languageVersion", normalized["languageVersion"] ?: JsonNull)
            put("apiVersion", normalized["apiVersion"] ?: JsonNull)
            put("jdkHomeFingerprint", normalized["jdkHomeFingerprint"] ?: JsonNull)
            put("fieldBoundaries", normalized["fieldBoundaries"] ?: JsonNull)
            put("buildRoot", normalized["buildRoot"] ?: JsonNull)
            put("projectDirectory", normalized["projectDirectory"] ?: JsonNull)
            put("module", normalized["module"] ?: JsonNull)
            put("sourceSet", normalized["sourceSet"] ?: JsonNull)
            put("platform", normalized["platform"] ?: JsonNull)
            put("generatedSourceConfiguration", normalized["generatedSourceConfiguration"] ?: JsonNull)
            put("buildModelBoundaries", normalized["buildModelBoundaries"] ?: JsonNull)
            put("dependencyCoordinates", normalized["dependencyCoordinates"] ?: JsonNull)
            put("repositories", normalized["repositories"] ?: JsonNull)
            put("reactorPoms", normalized["reactorPoms"] ?: JsonNull)
            put("buildPlugins", normalized["buildPlugins"] ?: JsonNull)
            put("classpathAuthority", normalized["classpathAuthority"] ?: JsonNull)
            put("buildState", normalized["buildState"] ?: JsonNull)
            put("modelInputs", normalized["modelInputs"] ?: JsonArray(emptyList()))
        }
        return buildJsonObject {
            put("schema", "semantic-project/0.1"); put("projectPath", ".")
            normalized.forEach { (key, value) -> put(key, value) }
            put("compilation", canonicalCompilation)
            put("workerCompilerVersion", WORKER_COMPILER_VERSION)
            put("jdk", 21)
            put("projectModelHash", modelHash)
            put("semanticInputManifest", semanticInputManifest)
            put("semanticInputManifestHash", sha(canonicalJson(semanticInputManifest).toByteArray()))
        }
    }

    private fun gradleModel(repo: Path, compilation: String?): JsonObject {
        val wrapper = repo.resolve("gradlew"); if (!wrapper.isRegularFile()) throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "Gradle Wrapper is required")
        val state = stateFor(repo)
        val gradleUserHome = state.gradleUserHome
        val projectCacheDirectory = if (state.mode == "EXTERNAL") {
            realStateSubdirectory(gradleUserHome, "project-cache")
        } else {
            gradleUserHome
        }
        val script = Files.createTempFile("semantic-thread-model", ".init.gradle")
        try {
            val resource = Worker::class.java.getResourceAsStream("/semantic-thread-model.init.gradle") ?: error("project model init script missing")
            script.writeBytes(resource.use { it.readBytes() })
            val selected = compilation ?: ":/main"
            val projectPath = if ('/' in selected) selected.substringBeforeLast('/').ifBlank { ":" } else selected.substringBeforeLast(':', ":").ifBlank { ":" }
            val sourceSet = if ('/' in selected) selected.substringAfterLast('/') else if (selected.contains("compileTest", true)) "test" else "main"
            val compileTask = if ('/' in selected) if (sourceSet == "main") "compileKotlin" else "compile${sourceSet.replaceFirstChar(Char::uppercase)}Kotlin" else selected.substringAfterLast(':').ifBlank { "compileKotlin" }
            val modelTask = if (projectPath == ":") ":semanticThreadModel" else "$projectPath:semanticThreadModel"
            val process = sanitizedProjectModelProcess(
                gradleModelCommand(
                    wrapper,
                    repo,
                    gradleUserHome,
                    projectCacheDirectory,
                    script,
                    compileTask,
                    modelTask,
                    reuseDaemon = !System.getenv("CODECLEW_K2_INDEX_ROOT").isNullOrBlank(),
                ),
                repo,
                state.gradleUserHome.takeIf { state.mode == "EXTERNAL" },
                ProjectModelBuildTool.GRADLE.takeIf { state.mode == "EXTERNAL" },
            ).start()
            val output = process.inputStream.bufferedReader().readText(); val status = process.waitFor()
            if (status != 0) throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "Gradle model extraction failed: ${output.takeLast(2000)}")
            val line = output.lineSequence().lastOrNull { it.startsWith("__SEMANTIC_THREAD_MODEL__") }
                ?: throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "Gradle model marker missing")
            return json.parseToJsonElement(line.removePrefix("__SEMANTIC_THREAD_MODEL__")).jsonObject
        } finally { Files.deleteIfExists(script) }
    }

    private fun applyCompilerPluginCompatibility(model: JsonObject): JsonObject {
        val requested = model["compilerPlugins"]?.jsonArray
            ?.map { Path.of(it.jsonPrimitive.content) }
            .orEmpty()
        val declared = model["compilerVersion"]?.jsonPrimitive?.contentOrNull
            ?: throw WorkerFailure(
                "UNSUPPORTED_PROJECT_CONFIGURATION",
                "declared Kotlin compiler version is unavailable",
            )
        val plan = effectiveCompilerPluginPlan(requested, declared)
        val existingBoundaries = model["buildModelBoundaries"]?.jsonArray
            ?.mapNotNull { it.jsonPrimitive.contentOrNull }
            .orEmpty()
        return buildJsonObject {
            model.forEach { (key, value) ->
                if (key !in setOf("compilerPlugins", "requestedCompilerPlugins", "buildModelBoundaries")) {
                    put(key, value)
                }
            }
            putJsonArray("requestedCompilerPlugins") {
                requested.map(Path::toAbsolutePath).map(Path::normalize).distinct().sortedBy(Path::toString).forEach {
                    add(JsonPrimitive(it.toString()))
                }
            }
            putJsonArray("compilerPlugins") {
                plan.plugins.forEach { add(JsonPrimitive(it.toString())) }
            }
            putJsonArray("buildModelBoundaries") {
                (existingBoundaries + plan.boundaries).distinct().sorted().forEach { add(JsonPrimitive(it)) }
            }
        }
    }

    private fun projectModel(repo: Path, compilation: String?): JsonObject {
        val hasGradle = Files.isRegularFile(repo.resolve("gradlew"), java.nio.file.LinkOption.NOFOLLOW_LINKS)
        val hasMaven = Files.isRegularFile(repo.resolve("pom.xml"), java.nio.file.LinkOption.NOFOLLOW_LINKS)
        if (hasGradle && hasMaven) {
            throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "mixed Gradle and Maven repositories are not supported")
        }
        val model = when {
            hasGradle -> gradleModel(repo, compilation).let { model ->
                buildJsonObject {
                    model.forEach(::put)
                    put("buildSystem", "GRADLE")
                }
            }
            hasMaven -> MavenProjectModelExtractor(stateFor(repo)).extract(repo, compilation)
            else -> throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "Gradle Wrapper or Maven pom.xml is required")
        }
        return applyCompilerPluginCompatibility(model)
    }

    private fun cachedProjectModel(repo: Path, compilation: String?): JsonObject {
        val projectModelStarted = System.nanoTime()
        fun elapsedMicros(started: Long): Long = (System.nanoTime() - started).coerceAtLeast(0L) / 1_000L
        requestCacheRequests++
        val keyStarted = System.nanoTime()
        val canonicalRepo = repo.toRealPath()
        val inputHash = sha((listOf("projectModelSchema=6") + projectModelFiles(canonicalRepo).map { file -> canonicalRepo.relativize(file).invariantSeparatorsPathString + ":" + sha(file.readBytes()) } +
            Files.walk(canonicalRepo).use { paths -> paths.filter {
                Files.isRegularFile(it, java.nio.file.LinkOption.NOFOLLOW_LINKS) && it.extension == "kt" &&
                    !it.invariantSeparatorsPathString.contains("/build/") &&
                    !it.invariantSeparatorsPathString.contains("/target/")
            }.map { canonicalRepo.relativize(it).invariantSeparatorsPathString }.sorted().toList() }).joinToString("\n").toByteArray())
        val key = "$canonicalRepo|${compilation ?: ":/main"}|$inputHash"
        val keyMicros = elapsedMicros(keyStarted)
        val persistentRoot = System.getenv("CODECLEW_K2_INDEX_ROOT")
        val persistentConfigured = !persistentRoot.isNullOrBlank()
        projectModelCache[key]?.let { model ->
            requestCacheHits++
            IncrementalK2Runtime.recordProjectModel(
                status = "MEMORY_HIT",
                totalMicros = elapsedMicros(projectModelStarted),
                keyMicros = keyMicros,
                loadMicros = 0L,
                extractionMicros = 0L,
                publishMicros = 0L,
                persistentConfigured = persistentConfigured,
                published = false,
            )
            return model
        }
        val loadStarted = System.nanoTime()
        val persistentModel = PersistentProjectModelCache.load(persistentRoot, canonicalRepo, key)
        val loadMicros = elapsedMicros(loadStarted)
        if (persistentModel != null) {
            requestCacheHits++
            projectModelCache[key] = persistentModel
            IncrementalK2Runtime.recordProjectModel(
                status = "PERSISTENT_HIT",
                totalMicros = elapsedMicros(projectModelStarted),
                keyMicros = keyMicros,
                loadMicros = loadMicros,
                extractionMicros = 0L,
                publishMicros = 0L,
                persistentConfigured = persistentConfigured,
                published = false,
            )
            return persistentModel
        }
        val extractionStarted = System.nanoTime()
        val model = withSemanticInputManifestHash(
            validateProjectModelSourceFiles(
                canonicalRepo,
                projectModel(canonicalRepo, compilation),
            ),
        )
        val extractionMicros = elapsedMicros(extractionStarted)
        // The optional persistent copy lives only under the explicit private index root.
        // Publication failure never changes the already verified semantic result.
        val publishStarted = System.nanoTime()
        val published = PersistentProjectModelCache.publish(persistentRoot, canonicalRepo, key, model)
        val publishMicros = elapsedMicros(publishStarted)
        projectModelCache[key] = model
        IncrementalK2Runtime.recordProjectModel(
            status = if (published) "EXTRACTED_PUBLISHED" else "EXTRACTED_NOT_PUBLISHED",
            totalMicros = elapsedMicros(projectModelStarted),
            keyMicros = keyMicros,
            loadMicros = loadMicros,
            extractionMicros = extractionMicros,
            publishMicros = publishMicros,
            persistentConfigured = persistentConfigured,
            published = published,
        )
        return model
    }

    private fun sourceRoot(repo: Path, file: Path): String? {
        val normalized = file.normalize(); val markers = listOf("/src/main/kotlin/", "/src/test/kotlin/")
        val text = normalized.invariantSeparatorsPathString
        val marker = markers.firstOrNull(text::contains) ?: return normalized.parent?.let { repo.relativize(it).invariantSeparatorsPathString }
        val root = Path.of(text.substringBefore(marker) + marker.removeSuffix("/")); return if (root.startsWith(repo)) repo.relativize(root).invariantSeparatorsPathString else root.invariantSeparatorsPathString
    }

    private fun normalizeArtifact(repo: Path, path: Path): String {
        val normalized = path.toAbsolutePath().normalize()
        return if (normalized.startsWith(repo.toAbsolutePath().normalize())) "repo:${repo.relativize(normalized).invariantSeparatorsPathString}:${artifactFingerprint(normalized)}"
        else "artifact:${normalized.fileName}:${artifactFingerprint(normalized)}"
    }

    private fun artifactFingerprint(path: Path): String {
        if (Files.isSymbolicLink(path)) {
            throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "semantic input artifact is a symbolic link")
        }
        return when {
            path.isRegularFile() -> sha(path.readBytes())
            path.isDirectory() -> sha(Files.walk(path).use { entries ->
                entries.sorted().filter { entry ->
                    if (Files.isSymbolicLink(entry)) {
                        throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "semantic input artifact tree contains a symbolic link")
                    }
                    entry.isRegularFile()
                }.map { entry ->
                    path.relativize(entry).invariantSeparatorsPathString + ":" + sha(entry.readBytes())
                }.toList().joinToString("\n").toByteArray()
            })
            else -> throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "semantic input artifact is missing")
        }
    }

    private fun jdkFingerprint(home: Path): String {
        val canonical = home.toRealPath()
        val release = canonical.resolve("release")
        val java = canonical.resolve("bin/java")
        if (!release.isRegularFile() || !java.isRegularFile()) {
            throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "JDK identity files are missing")
        }
        return sha(buildString {
            append("release:").append(sha(release.readBytes())).append('\u0000')
            append("java:").append(sha(java.readBytes())).append('\u0000')
        }.toByteArray())
    }

    private fun extractorAuthority(): JsonObject {
        val pluginArtifact = Path.of(Worker::class.java.protectionDomain.codeSource.location.toURI())
            .toAbsolutePath()
            .normalize()
        return buildJsonObject {
            put("extractorSchema", FIR_FACTS_EXTRACTOR_SCHEMA)
            put("pluginArtifactFingerprint", artifactFingerprint(pluginArtifact))
            put("workerCompilerVersion", WORKER_COMPILER_VERSION)
            put("workerVersion", WORKER_VERSION)
            put("workerProtocolVersion", "$PROTOCOL_MAJOR.$PROTOCOL_MINOR")
        }
    }

    private fun projectModelFiles(repo: Path): List<Path> = Files.walk(repo).use { paths ->
        paths.filter { Files.isRegularFile(it, java.nio.file.LinkOption.NOFOLLOW_LINKS) }.filter {
            val relative = repo.relativize(it).invariantSeparatorsPathString
            if (relative.split('/').any { part -> part in setOf("build", "target", ".gradle", ".kotlin", ".semantic-thread", ".git") }) return@filter false
            val n = it.fileName.toString()
                n == "settings.gradle" || n == "settings.gradle.kts" || n == "build.gradle" || n == "build.gradle.kts" ||
                n == "gradle.properties" || n == "libs.versions.toml" || n == "gradle-wrapper.properties" || n == "gradle-wrapper.jar" || n == "gradlew" || n == "gradlew.bat" ||
                n == "pom.xml" || n == "mvnw" || n == "mvnw.cmd" || relative.startsWith(".mvn/") ||
                relative.startsWith("buildSrc/") || relative.startsWith("build-logic/") || relative.startsWith("gradle/")
        }.sorted().toList()
    }

    private fun sourceFiles(repo: Path): List<Path> = syntaxOnlyIndexSourceFiles(repo)

    private fun compilationSourceFiles(repo: Path, compilation: String): List<Path> =
        cachedProjectModel(repo, compilation)["sourceFiles"]?.jsonArray
            ?.map { Path.of(it.jsonPrimitive.content) }
            ?.filter { Files.isRegularFile(it, java.nio.file.LinkOption.NOFOLLOW_LINKS) }
            ?.sorted()
            .orEmpty()

    private fun analyzeWithK2(repo: Path, overrides: Map<String, String> = emptyMap(), compilation: String = ":/main"): K2Analysis {
        requestCacheRequests++
        val analysisRepo = repo.toRealPath()
        val model = cachedProjectModel(analysisRepo, compilation)
        val sources = (model["analysisSourceFiles"] ?: model["sourceFiles"])
            ?.jsonArray
            ?.map { Path.of(it.jsonPrimitive.content) }
            ?.filter { Files.isRegularFile(it, java.nio.file.LinkOption.NOFOLLOW_LINKS) }
            ?.sorted()
            .orEmpty()
        val pluginArtifact = Path.of(Worker::class.java.protectionDomain.codeSource.location.toURI())
            .toAbsolutePath()
            .normalize()
        val semanticConfigurationDigest = model["semanticInputManifestHash"]?.jsonPrimitive?.contentOrNull
            ?: throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "semantic input manifest hash is unavailable")
        val cacheKey = semanticK2CacheKey(
            extractorAuthority(),
            buildString {
                val version = model["declaredCompilerVersion"]?.jsonPrimitive?.contentOrNull ?: WORKER_COMPILER_VERSION
                append("declaredCompilerVersion=").append(version).append('\u0000')
                append("semanticConfigurationDigest=").append(semanticConfigurationDigest).append('\u0000')
                append("factsPlugin=").append(artifactFingerprint(pluginArtifact)).append('\u0000')
            },
        )
        val memoryKey = "$analysisRepo|$compilation|$cacheKey"
        analysisCache[memoryKey]?.let { requestCacheHits++; return it }

        val pluginPlan = runCatching {
            IncrementalK2Runtime.backendOrNull()?.let { backend ->
                val modelVersion = model["declaredCompilerVersion"]?.jsonPrimitive?.contentOrNull
                    ?: WORKER_COMPILER_VERSION
                val indexRoot = runCatching {
                    System.getenv(K2_INDEX_ROOT_ENV)?.takeIf(String::isNotBlank)?.let(Path::of)?.toRealPath()
                }.getOrNull()
                val useBackend = overrides.isEmpty() &&
                    backend != null &&
                    indexRoot != null &&
                    modelVersion == WORKER_COMPILER_VERSION &&
                    modelVersion in setOf("2.1.21", WORKER_COMPILER_VERSION)
                if (!useBackend) return@runCatching null

                val request = IncrementalK2Request(
                    indexRoot = indexRoot,
                    repo = analysisRepo,
                    compilation = compilation,
                    semanticConfigurationDigest = cacheKey,
                    expectedCompilerVersion = WORKER_COMPILER_VERSION,
                    moduleName = model["projectPath"]?.jsonPrimitive?.contentOrNull?.substringAfterLast(':')
                        ?.ifBlank { "main" } ?: "main",
                    sources = sources,
                    classpath = model["classpath"]?.jsonArray?.map { Path.of(it.jsonPrimitive.content) }?.toList().orEmpty().sortedBy { it.toString() },
                    friendPaths = model["friendPaths"]?.jsonArray?.map { Path.of(it.jsonPrimitive.content) }?.toList().orEmpty().sortedBy { it.toString() },
                    compilerPlugins = model["compilerPlugins"]?.jsonArray?.map { Path.of(it.jsonPrimitive.content) }?.toList().orEmpty().sortedBy { it.toString() },
                    compilerPluginOptions = model["compilerPluginOptions"]?.jsonArray?.map { it.jsonPrimitive.content }?.toList().orEmpty().sorted(),
                    freeCompilerArguments = model["freeCompilerArguments"]?.jsonArray?.map { it.jsonPrimitive.content }?.toList().orEmpty().sorted(),
                    optIns = model["optIns"]?.jsonArray?.map { it.jsonPrimitive.content }?.toList().orEmpty().sorted(),
                    jdkHome = Path.of(model["jdkHome"]?.jsonPrimitive?.content ?: System.getProperty("java.home")),
                    jvmTarget = model["jvmTarget"]?.jsonPrimitive?.content ?: "21",
                    languageVersion = model["languageVersion"]?.jsonPrimitive?.contentOrNull,
                    apiVersion = model["apiVersion"]?.jsonPrimitive?.contentOrNull,
                    factsPlugin = pluginArtifact,
                )
                val result = backend.analyze(request)
                requestK2AnalysisMicros += result.totalMicros
                requestFirExtractionMicros += result.firExtractionMicros
                when (result.status) {
                    IncrementalK2Status.UNCHANGED_HIT,
                    IncrementalK2Status.COLD_FULL,
                    IncrementalK2Status.INCREMENTAL,
                    IncrementalK2Status.RECOVERED_FULL -> {
                        val direct = K2Analysis(result.valid, result.facts, result.diagnostics)
                        analysisCache[memoryKey] = direct
                        IncrementalK2Runtime.record(result, fallbackUsed = false)
                        return direct
                    }
                    IncrementalK2Status.BUSY,
                    IncrementalK2Status.FAILED_RECOVERABLE -> {
                        IncrementalK2Runtime.record(result, fallbackUsed = true)
                        null
                    }
                }
            }
        }.getOrNull()

        if (pluginPlan != null) {
            return pluginPlan
        }

        // The live in-process cache is sufficient for repeated requests in one
        // analysis. A repository-owned disk copy was never authoritative and
        // made a read-only agent query dirty the user's worktree.
        val temp = Files.createTempDirectory("semantic-thread-k2")
        try {
            val sourceArgs = sources.map { original ->
                val relative = analysisRepo.relativize(original.toRealPath()).invariantSeparatorsPathString
                val replacement = overrides[relative]
                if (replacement == null) original else temp.resolve("sources").resolve(relative).also { it.parent.createDirectories(); it.writeText(replacement) }
            }
            val factsFile = temp.resolve("facts.jsonl"); val outputDir = temp.resolve("classes").also(Path::createDirectories)
            val classpath = model["classpath"]?.jsonArray?.joinToString(File.pathSeparator) { it.jsonPrimitive.content }.orEmpty()
            val command = mutableListOf("-d", outputDir.toString(), "-classpath", classpath, "-no-stdlib", "-no-reflect", "-jdk-home", model["jdkHome"]!!.jsonPrimitive.content, "-jvm-target", model["jvmTarget"]?.jsonPrimitive?.content?.removePrefix("JVM_") ?: "21")
            model["languageVersion"]?.jsonPrimitive?.contentOrNull?.let { command += listOf("-language-version", it) }
            model["apiVersion"]?.jsonPrimitive?.contentOrNull?.let { command += listOf("-api-version", it) }
            val friendPaths = model["friendPaths"]?.jsonArray?.map { it.jsonPrimitive.content }.orEmpty()
            if (friendPaths.isNotEmpty()) command += "-Xfriend-paths=${friendPaths.joinToString(File.pathSeparator)}"
            model["freeCompilerArguments"]?.jsonArray?.map { it.jsonPrimitive.content }?.let(command::addAll)
            model["optIns"]?.jsonArray?.map { "-opt-in=${it.jsonPrimitive.content}" }?.let(command::addAll)
            model["compilerPlugins"]?.jsonArray?.map { "-Xplugin=${it.jsonPrimitive.content}" }?.let(command::addAll)
            model["compilerPluginOptions"]?.jsonArray?.map { option ->
                listOf("-P", option.jsonPrimitive.content)
            }?.flatten()?.let(command::addAll)
            command += listOf("-Xplugin=$pluginArtifact", "-P", "plugin:$FACTS_PLUGIN_ID:output=$factsFile")
            command += sourceArgs.map(Path::toString)
            val compilerOutput = ByteArrayOutputStream()
            val k2Started = System.nanoTime()
            val status = PrintStream(compilerOutput, true, Charsets.UTF_8).use { output ->
                synchronized(K2JVMCompiler::class.java) { K2JVMCompiler().exec(output, *command.toTypedArray()).code }
            }
            requestK2AnalysisMicros += (System.nanoTime() - k2Started) / 1_000
            val output = compilerOutput.toString(Charsets.UTF_8)
            val diagnostics = output.lineSequence().filter { it.isNotBlank() }.map { line ->
                buildJsonObject { put("severity", when { "error:" in line.lowercase() || line.startsWith("e:") -> "ERROR"; "warning:" in line.lowercase() || line.startsWith("w:") -> "WARNING"; else -> "INFO" }); put("message", line) }
            }.toList()
            val facts = if (factsFile.isRegularFile()) parseCompilerFactLines(factsFile.readLines()) else emptyList()
            requestFirExtractionMicros += facts
                .filter { it["recordType"]?.jsonPrimitive?.content == "FIR_CFG" }
                .sumOf { it["firExtractionMicros"]?.jsonPrimitive?.longOrNull ?: 0 }
            val result = K2Analysis(status == 0, facts.sortedBy { it.toString() }, diagnostics)
            analysisCache[memoryKey] = result
            return result
        } finally { temp.toFile().deleteRecursively() }
    }

    private fun writeCacheAtomically(path: Path, content: String) {
        path.parent.createDirectories()
        val temporary = Files.createTempFile(path.parent, path.fileName.toString(), ".tmp")
        try {
            temporary.writeText(content)
            try { Files.move(temporary, path, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING) }
            catch (_: Throwable) { Files.move(temporary, path, StandardCopyOption.REPLACE_EXISTING) }
        } finally { Files.deleteIfExists(temporary) }
    }

    private fun parse(path: Path, capturedBytes: ByteArray? = null): KtFile {
        val started = System.nanoTime()
        val text = capturedBytes?.toString(Charsets.UTF_8) ?: path.readText()
        return factory.createFile(path.fileName.toString(), text).also {
            requestPsiParseMicros += (System.nanoTime() - started) / 1_000
        }
    }

    private fun index(requestedRepo: Path, compilation: String?, syntaxOnly: Boolean = false, requestedFiles: List<String> = emptyList()): JsonObject {
        val repo = requestedRepo.toRealPath()
        val selected = compilation ?: ":/main"
        val model = if (syntaxOnly) null else cachedProjectModel(repo, selected)
                val project = if (syntaxOnly) null else inspect(repo, selected)
                val module = project?.get("module")?.jsonPrimitive?.content ?: "."
                val sourceSet = project?.get("sourceSet")?.jsonPrimitive?.content ?: selected.substringAfterLast('/')
        val selectedFiles = if (syntaxOnly) {
            syntaxOnlyIndexSourceFiles(repo, requestedFiles)
        } else {
            val allFiles = model?.get("sourceFiles")?.jsonArray
                ?.map { Path.of(it.jsonPrimitive.content) }
                ?.filter { Files.isRegularFile(it, java.nio.file.LinkOption.NOFOLLOW_LINKS) }
                ?.sorted()
                .orEmpty()
            val requested = requestedFiles.toSet()
            val selectedFromModel = if (requested.isEmpty()) allFiles else allFiles.filter { repo.relativize(it).invariantSeparatorsPathString in requested }
            if (requested.isNotEmpty() && selectedFromModel.size != requested.size) throw WorkerFailure("INVALID_INPUT", "requested index file is outside selected compilation")
            selectedFromModel
        }
        val attemptedAnalysis = if (syntaxOnly) K2Analysis(false, emptyList(), emptyList()) else analyzeWithK2(repo, compilation = selected)
                val analysis = if (attemptedAnalysis.valid) attemptedAnalysis else K2Analysis(false, emptyList(), attemptedAnalysis.diagnostics)
                val semanticAvailable = analysis.valid
                val declarationRelations = if (semanticAvailable) declarationRelationGraph(repo, selected, false, analysis, project!!) else JsonArray(emptyList())
                val declarationDescriptors = if (semanticAvailable) declarationDescriptorGraph(repo, selected, false, analysis, project!!, module, sourceSet) else JsonArray(emptyList())
        val files = selectedFiles.map { path ->
            val bytes = path.readBytes(); val kt = parse(path, if (syntaxOnly) bytes else null); val pkg = kt.packageFqName.asString()
            val declarations = PsiTreeUtil.collectElementsOfType(kt, KtNamedDeclaration::class.java)
                .filter { it is KtNamedFunction || it is KtClassOrObject || it is KtProperty }
                .sortedBy { it.textOffset }.map { declarationJson(repo, path, pkg, it, analysis, module, sourceSet) }
            val relative = repo.relativize(path).invariantSeparatorsPathString
            val inheritance = PsiTreeUtil.collectElementsOfType(kt, KtClassOrObject::class.java).map { declaration ->
                buildJsonObject { put("symbol", symbolId(pkg, declaration, module, sourceSet)); putJsonArray("supertypes") { declaration.superTypeListEntries.map { it.typeReference?.text.orEmpty() }.sorted().forEach(::add) } }
            }.sortedBy { it.toString() }
            val overrides = PsiTreeUtil.collectElementsOfType(kt, KtCallableDeclaration::class.java).filter { it.hasModifier(KtTokens.OVERRIDE_KEYWORD) }
                .map { symbolId(pkg, it, module, sourceSet) }.sorted()
            buildJsonObject {
                put("fileId", "file:" + sha("$module|$sourceSet|$relative".toByteArray()).removePrefix("sha256:")); put("module", module); put("sourceSet", sourceSet)
                put("path", relative); put("normalizedRelativePath", relative); put("contentHash", sha(bytes)); put("package", pkg)
                put("lineEnding", if (bytes.decodeToString().contains("\r\n")) "CRLF" else "LF"); put("bom", bytes.take(3) == listOf(0xef.toByte(), 0xbb.toByte(), 0xbf.toByte()))
                putJsonArray("imports") { kt.importDirectives.map { it.importPath?.pathStr.orEmpty() }.sorted().forEach(::add) }
                putJsonArray("declarations") { declarations.forEach(::add) }
                putJsonArray("declarationIds") { declarations.map { it["declarationId"]!! }.sortedBy { it.toString() }.forEach(::add) }
                putJsonArray("semanticFacts") { if (semanticAvailable) fileFacts(repo, path, analysis).forEach(::add) }
                                putJsonArray("inheritance") { if (semanticAvailable) inheritance.forEach(::add) }
                                putJsonArray("overrides") { if (semanticAvailable) overrides.forEach(::add) }
                                putJsonArray("functionSummaries") { if (semanticAvailable) declarations.filter { it["kind"]?.jsonPrimitive?.content?.contains("Function") == true }.map { buildJsonObject { put("symbolId", it["symbolId"]!!); put("semanticSummaryHash", it["semanticSummaryHash"]!!) } }.forEach(::add) }
                putJsonArray("diagnostics") { analysis.diagnostics.filter { diagnostic -> diagnostic["message"]?.jsonPrimitive?.content?.replace('\\', '/')?.contains(relative) == true }.forEach(::add) }
            }
        }
        val canonical = JsonArray(files)
        return buildJsonObject {
            put("schema", "semantic-index/0.1"); put("compilation", selected); put("partial", requestedFiles.isNotEmpty()); put("analysisMode", if (semanticAvailable) "K2_SEMANTIC" else "SYNTAX_DECLARATIONS"); put("files", canonical); put("indexHash", sha(canonical.toString().toByteArray()))
            put("declarationRelations", declarationRelations)
            put("declarationRelationHash", sha(canonicalJson(declarationRelations).toByteArray()))
            put("declarationDescriptors", declarationDescriptors)
            put("declarationDescriptorHash", sha(canonicalJson(declarationDescriptors).toByteArray()))
            if (project == null) {
                            val syntaxManifest = buildJsonObject {
                                put("schema", "kotlin-syntax-input-manifest/0.1")
                                put("compilation", selected)
                            }
                            val syntaxManifestHash = sha(canonicalJson(syntaxManifest).toByteArray())
                            put("projectModelHash", syntaxManifestHash)
                            put("classpathHash", sha("[]".toByteArray()))
                            put("semanticInputManifest", syntaxManifest)
                            put("semanticInputManifestHash", syntaxManifestHash)
                            put("buildModelBoundaries", JsonArray(emptyList()))
                            put("compilerVersion", WORKER_COMPILER_VERSION)
                            put("compilerOptionsHash", sha("{}".toByteArray()))
                        } else {
                            put("projectModelHash", project["projectModelHash"]!!); put("classpathHash", sha(project["compileClasspath"]!!.toString().toByteArray()))
                            put("semanticInputManifest", project["semanticInputManifest"]!!)
                            put("semanticInputManifestHash", project["semanticInputManifestHash"]!!)
                            put("buildModelBoundaries", project["buildModelBoundaries"]!!)
                            put("compilerVersion", project["compilerVersion"]!!)
                            put("compilerOptionsHash", sha(buildJsonObject { put("languageVersion", project["languageVersion"]!!); put("apiVersion", project["apiVersion"]!!); put("jvmTarget", project["jvmTarget"]!!); put("freeCompilerArguments", project["freeCompilerArguments"]!!); put("compilerPlugins", project["compilerPlugins"]!!); put("compilerPluginOptions", project["compilerPluginOptions"]!!) }.toString().toByteArray()))
                        }
            put("k2Validated", analysis.valid); putJsonArray("diagnostics") { analysis.diagnostics.forEach(::add) }
        }
    }

    private fun declarationRelationGraph(
        repo: Path,
        compilation: String,
        syntaxOnly: Boolean,
        analysis: K2Analysis,
        project: JsonObject,
    ): JsonObject {
        fun relativeFile(raw: String): String? {
            return repositoryRelativeCompilerPath(repo, raw)
        }
        val sourceTextByFile = projectSourceTextByRelativePath(repo, compilation)
        val quarantinedDescriptorKeys = analysis.facts
            .filter { it["recordType"].safeString() == "DECLARATION_DESCRIPTOR" }
            .filter { raw ->
                val file = raw["file"].safeString()?.let(::relativeFile)
                val reason = descriptorUnsupportedReason(raw, file, file?.let(sourceTextByFile::get))
                reason != null && !isOptionalDescriptorAttributeBoundary(reason)
            }
            .flatMap { raw ->
                listOf("symbolIdentity", "compilerCallableId", "compilerClassId")
                    .mapNotNull { field -> raw[field].safeString() }
            }
            .filter(String::isNotEmpty)
            .toSet()
        fun boundary(
            code: String,
            raw: JsonObject,
            stage: String = "NORMALIZE",
            file: String? = null,
            owner: String? = raw["owner"].safeString(),
            target: String? = null,
            relationKind: String? = null,
            start: Int? = null,
            end: Int? = null,
            retainedRelationHash: String? = null,
        ) = buildJsonObject {
            put("schema", "declaration-relation-boundary/0.1")
            file?.let { put("file", it) }
            owner?.takeIf(String::isNotEmpty)?.let { put("owner", it) }
            target?.takeIf(String::isNotEmpty)?.let { put("target", it) }
            relationKind?.takeIf(String::isNotEmpty)?.let { put("relationKind", it) }
            start?.let { put("start", it) }
            end?.let { put("end", it) }
            retainedRelationHash?.let { put("retainedRelationHash", it) }
            put("stage", stage)
            put("code", code)
            put("resolution", "UNKNOWN")
            put("provider", "COMPILER_RELATION_NORMALIZER")
            put("rawRowHash", canonicalCompilerRowDigest(raw, file))
        }
        val cfgByOwner = analysis.facts
            .filter { it["recordType"].safeString() == "FIR_CFG" }
            .groupBy { fact ->
                val file = fact["file"].safeString()?.let(::relativeFile).orEmpty()
                "$file\u0000${fact["symbol"].safeString().orEmpty()}"
            }
        val generatedBoundaries = mutableListOf<JsonObject>()
        val relations = analysis.facts
            .filter { it["recordType"].safeString() == "DECLARATION_RELATION" }
            .mapNotNull { raw ->
                val rawFile = raw["file"].safeString()
                val file = rawFile?.let(::relativeFile)
                if (file == null) {
                    generatedBoundaries += boundary("INVALID_RELATION_SOURCE_PATH", raw)
                    return@mapNotNull null
                }
                val owner = raw["owner"].safeString().orEmpty()
                val target = raw["target"].safeString().orEmpty()
                val kind = raw["kind"].safeString().orEmpty()
                val start = raw["start"].safeInt() ?: -1
                val end = raw["end"].safeInt() ?: -1
                val source = sourceTextByFile[file]
                val byteRange = source?.let { compilerRangeToUtf8Bytes(it, start, end) }
                val unsupported = when {
                    owner.isEmpty() || target.isEmpty() -> "INVALID_RELATION_IDENTITY"
                    kind !in SUPPORTED_RELATION_KINDS -> "UNKNOWN_RELATION_KIND"
                    owner in quarantinedDescriptorKeys || target in quarantinedDescriptorKeys -> "REFERENCE_TO_QUARANTINED_DESCRIPTOR"
                    source == null -> "RELATION_SOURCE_NOT_IN_COMPILATION"
                    byteRange == null -> "INVALID_RELATION_SOURCE_RANGE"
                    else -> null
                }
                if (unsupported != null) {
                    val exactCore = unsupported == "REFERENCE_TO_QUARANTINED_DESCRIPTOR" && byteRange != null
                    generatedBoundaries += boundary(
                        unsupported,
                        raw,
                        file = file,
                        target = target.takeIf { exactCore },
                        relationKind = kind.takeIf { exactCore },
                        start = byteRange?.first?.takeIf { exactCore },
                        end = byteRange?.let { it.last + 1 }?.takeIf { exactCore },
                    )
                    return@mapNotNull null
                }
                val unresolvedTypeEvidence = rawContainsUnresolvedCompilerType(raw)
                val provenByteRange = byteRange!!
                if (unresolvedTypeEvidence && !isRetainedCallTopologyKind(kind)) {
                    generatedBoundaries += boundary(
                        "UNRESOLVED_RELATION_TYPE",
                        raw,
                        file = file,
                        target = target,
                        relationKind = kind,
                        start = provenByteRange.first,
                        end = provenByteRange.last + 1,
                    )
                    return@mapNotNull null
                }
                val retainCallTopology = unresolvedTypeEvidence
                val cfgNodes = cfgByOwner["$file\u0000$owner"].orEmpty()
                    .flatMap { cfg -> (cfg["nodes"] as? JsonArray).orEmpty() }
                    .mapNotNull { it as? JsonObject }
                    .filter { node ->
                        val nodeStart = node["start"].safeInt() ?: return@filter false
                        val nodeEnd = node["end"].safeInt() ?: return@filter false
                        nodeStart <= start && nodeEnd >= end || start <= nodeStart && end >= nodeEnd
                    }
                    .mapNotNull { it["id"].safeInt() }
                    .distinct()
                    .sorted()
                if (kind in setOf("CALLS", "CONSTRUCTS", "READS", "WRITES", "NULL_COALESCES", "RETURNS_VALUE_FROM") && cfgNodes.isEmpty()) {
                    generatedBoundaries += buildJsonObject {
                        put("schema", "declaration-relation-boundary/0.1")
                        put("file", file)
                        put("owner", owner)
                        put("stage", "ORDER_PROVENANCE")
                        put("code", "NO_CFG_NODE_FOR_RELATION")
                        put("start", provenByteRange.first)
                        put("end", provenByteRange.last + 1)
                        put("resolution", "UNKNOWN")
                        put("provider", "K2_FIR_CFG")
                    }
                }
                val sourceRowHash = canonicalCompilerRowDigest(raw, file)
                val relationPayload = if (retainCallTopology) relationCorePayload(raw, sourceRowHash) else raw
                val normalized = buildJsonObject {
                    relationPayload.entries.sortedBy { it.key }.forEach { (key, value) ->
                        if (key !in setOf("recordType", "file", "start", "end")) put(key, value)
                    }
                    put("file", file)
                    put("start", provenByteRange.first)
                    put("end", provenByteRange.last + 1)
                    putJsonArray("cfgNodeIds") { cfgNodes.forEach(::add) }
                    put("sourceProvenance", "COMPILER_UTF16_RANGE_TO_UTF8_BYTES")
                    put(
                        "orderProvenance",
                        if (cfgNodes.isEmpty()) {
                            JsonPrimitive(raw["orderProvenance"].safeString().takeIf {
                                it in setOf("K2_FIR_CFG", "FIR_SOURCE_RANGE", "UNKNOWN")
                            } ?: "UNKNOWN")
                        } else {
                            JsonPrimitive("K2_FIR_CFG")
                        },
                    )
                }
                if (retainCallTopology) {
                    generatedBoundaries += boundary(
                        "UNRESOLVED_RELATION_TYPE",
                        raw,
                        file = file,
                        target = target,
                        relationKind = kind,
                        start = provenByteRange.first,
                        end = provenByteRange.last + 1,
                        retainedRelationHash = stableBoundaryDigest(normalized),
                    )
                }
                normalized
            }
            .distinctBy(::canonicalJson)
            .sortedBy(::canonicalJson)
        val compilerBoundaries = analysis.facts
            .filter { it["recordType"].safeString() == "DECLARATION_RELATION_BOUNDARY" }
            .map { raw ->
                val rawFile = raw["file"].safeString()
                val normalizedFile = rawFile?.let(::relativeFile)
                if (rawFile != null && normalizedFile == null) {
                    return@map boundary("INVALID_RELATION_SOURCE_PATH", raw)
                }
                val start = raw["start"].safeInt()
                val end = raw["end"].safeInt()
                val byteRange = if (start != null && end != null && normalizedFile != null) {
                    sourceTextByFile[normalizedFile]?.let { compilerRangeToUtf8Bytes(it, start, end) }
                } else null
                if (start != null && end != null && byteRange == null) {
                    return@map boundary("INVALID_RELATION_SOURCE_RANGE", raw, file = normalizedFile)
                }
                buildJsonObject {
                    raw.entries.sortedBy { it.key }.forEach { (key, value) ->
                        if (key !in setOf("recordType", "file", "start", "end")) put(key, value)
                    }
                    normalizedFile?.let { put("file", it) }
                    byteRange?.let { range ->
                        put("start", range.first)
                        put("end", range.last + 1)
                        put("sourceProvenance", "COMPILER_UTF16_RANGE_TO_UTF8_BYTES")
                    }
                    put("rawRowHash", canonicalCompilerRowDigest(raw, normalizedFile))
                }
            }
        val boundaries = (compilerBoundaries + generatedBoundaries)
            .distinctBy(::canonicalJson)
            .sortedBy(::canonicalJson)
        return canonicalJsonValue(buildJsonObject {
            put("schema", "declaration-relation-graph/0.1")
            put("compilation", compilation)
            put("coverage", if (syntaxOnly || boundaries.isNotEmpty()) "PARTIAL" else "COMPLETE_SUPPORTED_SUBSET")
            putJsonArray("relations") { relations.forEach(::add) }
            putJsonArray("boundaries") {
                if (syntaxOnly) add(buildJsonObject {
                    put("schema", "declaration-relation-boundary/0.1")
                    put("stage", "ANALYSIS")
                    put("code", "SYNTAX_ONLY")
                    put("resolution", "UNKNOWN")
                    put("provider", "WORKER")
                })
                boundaries.forEach(::add)
            }
            putJsonObject("provenance") {
                put("provider", "COMPILER_SEMANTIC_FACTS")
                extractorAuthority().forEach { (key, value) -> put(key, value) }
                put("compilerVersion", project["compilerVersion"] ?: JsonPrimitive("<unknown>"))
                put("projectModelHash", project["projectModelHash"] ?: JsonPrimitive("<unknown>"))
                put("classpathHash", sha(project["compileClasspath"]!!.toString().toByteArray()))
                put("compilerOptionsHash", sha(buildJsonObject {
                    put("languageVersion", project["languageVersion"]!!)
                    put("apiVersion", project["apiVersion"]!!)
                    put("jvmTarget", project["jvmTarget"]!!)
                    put("freeCompilerArguments", project["freeCompilerArguments"]!!)
                    put("compilerPlugins", project["compilerPlugins"]!!)
                    put("compilerPluginOptions", project["compilerPluginOptions"]!!)
                }.toString().toByteArray()))
            }
        }).jsonObject
    }

    private fun declarationDescriptorGraph(
        repo: Path,
        compilation: String,
        syntaxOnly: Boolean,
        analysis: K2Analysis,
        project: JsonObject,
        module: String,
        sourceSet: String,
    ): JsonObject {
        fun relativeFile(raw: String): String? {
            return repositoryRelativeCompilerPath(repo, raw)
        }
        val sourceTextByFile = projectSourceTextByRelativePath(repo, compilation)
        fun boundary(
            code: String,
            raw: JsonObject,
            file: String? = null,
            retainedDescriptorHash: String? = null,
        ) = buildJsonObject {
            put("schema", "declaration-descriptor-boundary/0.1")
            file?.let { put("file", it) }
            raw["symbolIdentity"].safeString()?.takeIf(String::isNotEmpty)?.let { put("symbolIdentity", it) }
            retainedDescriptorHash?.let { put("retainedDescriptorHash", it) }
            put("stage", "NORMALIZE")
            put("code", code)
            put("resolution", "UNKNOWN")
            put("provider", "COMPILER_DESCRIPTOR_NORMALIZER")
            put("module", module)
            put("sourceSet", sourceSet)
            put("compilerAuthority", FIR_FACTS_EXTRACTOR_SCHEMA)
            put("rawRowHash", canonicalCompilerRowDigest(raw, file))
        }
        val generatedBoundaries = mutableListOf<JsonObject>()
        val descriptors = analysis.facts
            .filter { it["recordType"].safeString() == "DECLARATION_DESCRIPTOR" }
            .mapNotNull { raw ->
                val file = raw["file"].safeString()?.let(::relativeFile)
                val start = raw["start"].safeInt() ?: -1
                val end = raw["end"].safeInt() ?: -1
                val source = file?.let(sourceTextByFile::get)
                val byteRange = source?.let { compilerRangeToUtf8Bytes(it, start, end) }
                val unsupported = descriptorUnsupportedReason(raw, file, source)
                val attributeBoundary = unsupported.takeIf(::isOptionalDescriptorAttributeBoundary)
                if (unsupported != null && attributeBoundary == null) {
                    generatedBoundaries += boundary(unsupported, raw, file)
                    return@mapNotNull null
                }
                val provenByteRange = byteRange!!
                val sourceRowHash = canonicalCompilerRowDigest(raw, file)
                val descriptorPayload = if (attributeBoundary != null) {
                    descriptorCorePayload(raw, sourceRowHash)
                } else {
                    raw
                }
                val normalized = buildJsonObject {
                    descriptorPayload.entries.sortedBy { it.key }.forEach { (key, value) ->
                        if (key !in setOf("recordType", "file", "start", "end")) put(key, value)
                    }
                    put("file", file)
                    put("start", provenByteRange.first)
                    put("end", provenByteRange.last + 1)
                    put("module", module)
                    put("sourceSet", sourceSet)
                    put("sourceProvenance", "COMPILER_UTF16_RANGE_TO_UTF8_BYTES")
                    put("compilerAuthority", FIR_FACTS_EXTRACTOR_SCHEMA)
                }
                if (attributeBoundary != null) {
                    generatedBoundaries += boundary(
                        attributeBoundary,
                        raw,
                        file,
                        retainedDescriptorHash = stableBoundaryDigest(normalized),
                    )
                }
                normalized
            }
            .distinctBy(::canonicalJson)
            .sortedBy(::canonicalJson)
        val compilerBoundaries = analysis.facts
            .filter { it["recordType"].safeString() == "DECLARATION_DESCRIPTOR_BOUNDARY" }
            .map { raw ->
                val rawFile = raw["file"].safeString()
                val normalizedFile = rawFile?.let(::relativeFile)
                if (rawFile != null && normalizedFile == null) {
                    return@map boundary("INVALID_DESCRIPTOR_SOURCE_PATH", raw)
                }
                val start = raw["start"].safeInt()
                val end = raw["end"].safeInt()
                val byteRange = if (start != null && end != null && normalizedFile != null) {
                    sourceTextByFile[normalizedFile]?.let { compilerRangeToUtf8Bytes(it, start, end) }
                } else null
                if (start != null && end != null && byteRange == null) {
                    return@map boundary("INVALID_DESCRIPTOR_SOURCE_RANGE", raw, normalizedFile)
                }
                buildJsonObject {
                    raw.entries.sortedBy { it.key }.forEach { (key, value) ->
                        if (key !in setOf("recordType", "file", "start", "end")) put(key, value)
                    }
                    normalizedFile?.let { put("file", it) }
                    byteRange?.let { range ->
                        put("start", range.first)
                        put("end", range.last + 1)
                        put("sourceProvenance", "COMPILER_UTF16_RANGE_TO_UTF8_BYTES")
                    }
                    put("module", module)
                    put("sourceSet", sourceSet)
                    put("compilerAuthority", FIR_FACTS_EXTRACTOR_SCHEMA)
                    put("rawRowHash", canonicalCompilerRowDigest(raw, normalizedFile))
                }
            }
        val boundaries = (compilerBoundaries + generatedBoundaries)
            .distinctBy(::canonicalJson)
            .sortedBy(::canonicalJson)
        return canonicalJsonValue(buildJsonObject {
            put("schema", "declaration-descriptor-graph/0.1")
            put("compilation", compilation)
            put("coverage", if (syntaxOnly || boundaries.isNotEmpty()) "PARTIAL" else "COMPLETE_SUPPORTED_SUBSET")
            putJsonArray("descriptors") { descriptors.forEach(::add) }
            putJsonArray("boundaries") {
                if (syntaxOnly) add(buildJsonObject {
                    put("schema", "declaration-descriptor-boundary/0.1")
                    put("stage", "ANALYSIS")
                    put("code", "SYNTAX_ONLY")
                    put("resolution", "UNKNOWN")
                    put("provider", "WORKER")
                    put("module", module)
                    put("sourceSet", sourceSet)
                    put("compilerAuthority", FIR_FACTS_EXTRACTOR_SCHEMA)
                })
                boundaries.forEach(::add)
            }
            putJsonObject("provenance") {
                put("provider", "COMPILER_SEMANTIC_FACTS")
                extractorAuthority().forEach { (key, value) -> put(key, value) }
                put("compilerVersion", project["compilerVersion"] ?: JsonPrimitive("<unknown>"))
                put("projectModelHash", project["projectModelHash"] ?: JsonPrimitive("<unknown>"))
                put("classpathHash", sha(project["compileClasspath"]!!.toString().toByteArray()))
                put("compilerOptionsHash", sha(buildJsonObject {
                    put("languageVersion", project["languageVersion"]!!)
                    put("apiVersion", project["apiVersion"]!!)
                    put("jvmTarget", project["jvmTarget"]!!)
                    put("freeCompilerArguments", project["freeCompilerArguments"]!!)
                    put("compilerPlugins", project["compilerPlugins"]!!)
                    put("compilerPluginOptions", project["compilerPluginOptions"]!!)
                }.toString().toByteArray()))
            }
        }).jsonObject
    }

    private fun projectSourceTextByRelativePath(repo: Path, compilation: String): Map<String, String> =
        (cachedProjectModel(repo, compilation)["sourceFiles"]?.jsonArray.orEmpty())
            .mapNotNull { entry ->
                val path = runCatching { Path.of(entry.jsonPrimitive.content).toRealPath() }.getOrNull()
                    ?: return@mapNotNull null
                val relative = runCatching { repo.relativize(path).invariantSeparatorsPathString }.getOrNull()
                    ?.takeUnless { it == ".." || it.startsWith("../") }
                    ?: return@mapNotNull null
                relative to runCatching { path.readText() }.getOrNull().orEmpty()
            }
            .toMap()

    private fun declarationJson(repo: Path, path: Path, pkg: String, declaration: KtNamedDeclaration, analysis: K2Analysis? = null, module: String = "", sourceSet: String = "main"): JsonObject {
        val resolvedTypes = resolvedIdentityTypes(repo, path, declaration, analysis)
        val identity = symbolIdentity(pkg, declaration, module.ifBlank { ":" }, sourceSet, resolvedTypes)
        val symbol = identity.toString(); val signature = sourceSignature(declaration)
        val containing = generateSequence(declaration.parent) { it.parent }.filterIsInstance<KtNamedDeclaration>().firstOrNull()?.let { symbolId(pkg, it, module.ifBlank { ":" }, sourceSet) }
        val relative = repo.relativize(path).invariantSeparatorsPathString
        val body = (declaration as? KtDeclarationWithBody)?.bodyExpression?.text.orEmpty()
        val declarationFacts = analysis?.let { fileFacts(repo, path, it) }?.filter { fact ->
            val start = fact["start"]?.jsonPrimitive?.intOrNull ?: -1; val end = fact["end"]?.jsonPrimitive?.intOrNull ?: -1
            start >= declaration.textRange.startOffset && end <= declaration.textRange.endOffset
        }.orEmpty()
        val signatureHash = sha(normalizeTokens(signature).toByteArray())
        val bodyHash = sha(normalizeTokens(body).toByteArray())
        val abiHash = sha(buildJsonObject { put("symbol", symbol); put("kind", declaration::class.simpleName.orEmpty()); put("signatureHash", signatureHash) }.toString().toByteArray())
        val summaryHash = sha(buildJsonObject { put("bodyHash", bodyHash); putJsonArray("facts") { declarationFacts.map(::semanticSignature).forEach(::add) } }.toString().toByteArray())
        val declarationId = "declaration:" + sha("$module|$sourceSet|$relative|$symbol|${declaration::class.simpleName}|$signatureHash".toByteArray()).removePrefix("sha256:")
        return buildJsonObject {
            put("declarationId", declarationId); put("symbolId", symbol); put("symbolIdentity", identity); put("legacySymbolId", legacySymbolId(pkg, declaration)); put("name", declaration.name.orEmpty()); put("kind", declaration::class.simpleName ?: "KtDeclaration")
            resolvedTypes?.get("symbol")?.let { put("compilerSymbol", it) }
            if (containing != null) put("containingDeclaration", containing)
            putJsonObject("sourceOrigin") { put("file", relative); put("rangeStart", declaration.textRange.startOffset); put("rangeEnd", declaration.textRange.endOffset) }
            put("file", relative); put("rangeStart", declaration.textRange.startOffset); put("rangeEnd", declaration.textRange.endOffset)
            put("sourceSignatureHash", signatureHash); put("signatureHash", signatureHash); put("bodyHash", bodyHash); put("abiHash", abiHash); put("semanticSummaryHash", summaryHash)
        }
    }

    private fun legacySymbolId(pkg: String, declaration: KtNamedDeclaration): String {
        val owners = generateSequence(declaration.parent) { it.parent }.filterIsInstance<KtNamedDeclaration>().toList().asReversed().mapNotNull { it.name }
        val base = (listOf(pkg).filter { it.isNotBlank() } + owners + listOf(declaration.name ?: "<anonymous>")).joinToString(".")
        if (declaration !is KtNamedFunction) return base
        val receiver = declaration.receiverTypeReference?.text?.let { "$it." }.orEmpty()
        val params = declaration.valueParameters.joinToString(",") { it.typeReference?.text ?: "?" }
        return "$receiver$base($params)"
    }

    private fun resolvedIdentityTypes(repo: Path, path: Path, declaration: KtNamedDeclaration, analysis: K2Analysis?): JsonObject? {
        if (analysis == null) return null
        if (declaration is KtNamedFunction) {
            return cfgRecords(repo, path, analysis).firstOrNull { fact ->
                fact["start"]?.jsonPrimitive?.intOrNull == declaration.textRange.startOffset &&
                    fact["end"]?.jsonPrimitive?.intOrNull == declaration.textRange.endOffset
            }
        }
        if (declaration is KtProperty) {
            val initializer = declaration.initializer ?: return null
            val fact = fileFacts(repo, path, analysis).filter { candidate ->
                val start = candidate["start"]?.jsonPrimitive?.intOrNull ?: -1
                val end = candidate["end"]?.jsonPrimitive?.intOrNull ?: -1
                start >= initializer.textRange.startOffset && end <= initializer.textRange.endOffset
            }.maxByOrNull { candidate ->
                (candidate["end"]?.jsonPrimitive?.intOrNull ?: 0) - (candidate["start"]?.jsonPrimitive?.intOrNull ?: 0)
            }
            return fact?.get("type")?.let { type -> buildJsonObject { put("returnType", type) } }
        }
        return null
    }

    private fun symbolIdentity(pkg: String, declaration: KtNamedDeclaration, module: String = ":", sourceSet: String = "main", resolvedTypes: JsonObject? = null): JsonObject {
        val containing = generateSequence(declaration.parent) { it.parent }.filterIsInstance<KtNamedDeclaration>().toList().asReversed().mapNotNull { it.name }
        val declarationKind = when (declaration) {
            is KtNamedFunction -> "FUNCTION"
            is KtProperty -> "PROPERTY"
            is KtClassOrObject -> "CLASS"
            else -> declaration::class.simpleName.orEmpty().removePrefix("Kt").uppercase()
        }
        val receiverTypes = resolvedTypes?.get("receiverType")?.jsonPrimitive?.contentOrNull?.takeUnless { it == "<unresolved>" }?.let(::listOf)
            ?: (declaration as? KtNamedFunction)?.receiverTypeReference?.text?.let(::listOf).orEmpty()
        val parameterTypes = resolvedTypes?.get("parameterTypes")?.jsonArray?.map { it.jsonPrimitive.content }?.takeUnless { values -> values.any { it == "<unresolved>" } }
            ?: (declaration as? KtNamedFunction)?.valueParameters?.map { it.typeReference?.text ?: "?" }.orEmpty()
        val returnType = when (declaration) {
            is KtNamedFunction -> resolvedTypes?.get("returnType")?.jsonPrimitive?.contentOrNull?.takeUnless { it == "<unresolved>" } ?: declaration.typeReference?.text ?: "<unresolved>"
            is KtProperty -> resolvedTypes?.get("returnType")?.jsonPrimitive?.contentOrNull?.takeUnless { it == "<unresolved>" } ?: declaration.typeReference?.text ?: "<unresolved>"
            is KtClassOrObject -> declaration.name ?: "?"
            else -> "?"
        }
        val contextReceiverTypes = Regex("context\\(([^)]*)\\)").find(declaration.text.substringBefore("fun "))
            ?.groupValues?.get(1)?.split(',')?.map(String::trim)?.filter(String::isNotEmpty).orEmpty()
        return buildJsonObject {
            put("module", module); put("sourceSet", sourceSet); put("package", pkg)
            putJsonArray("containingDeclarations") { containing.forEach(::add) }
            put("declarationName", declaration.name ?: "<anonymous>"); put("declarationKind", declarationKind)
            put("typeParameterArity", (declaration as? KtTypeParameterListOwner)?.typeParameters?.size ?: 0)
            putJsonArray("receiverTypes") { receiverTypes.forEach(::add) }
            putJsonArray("contextReceiverTypes") { contextReceiverTypes.forEach(::add) }
            putJsonArray("parameterTypes") { parameterTypes.forEach(::add) }
            put("returnType", returnType)
            put("suspendFlag", declaration is KtNamedFunction && declaration.hasModifier(KtTokens.SUSPEND_KEYWORD))
            val identityTypes = receiverTypes + parameterTypes + returnType
            val hasGenericArray = identityTypes.any { "Array<" in it }
            val typeParameterNames = (declaration as? KtTypeParameterListOwner)?.typeParameters?.mapNotNull { it.name }.orEmpty()
            val usesTypeParameter = identityTypes.any { type -> typeParameterNames.any { name -> Regex("(^|[<, ])${Regex.escape(name)}([>?, ]|$)").containsMatchIn(type) } }
            val compilerDescriptor = resolvedTypes?.get("jvmDescriptor")?.jsonPrimitive?.contentOrNull
                ?.takeIf { receiverTypes.isEmpty() && !hasGenericArray && !usesTypeParameter && (declaration !is KtNamedFunction || !declaration.hasModifier(KtTokens.SUSPEND_KEYWORD)) }
            put("jvmDescriptor", compilerDescriptor
                ?: jvmDescriptor(declaration, receiverTypes, parameterTypes, returnType, pkg))
        }
    }

    private fun jvmDescriptor(declaration: KtNamedDeclaration, receivers: List<String>, parameters: List<String>, returnType: String, pkg: String): String = when (declaration) {
        is KtNamedFunction -> {
            val arguments = (receivers + parameters).map { jvmTypeDescriptor(it, false, declaration) }.toMutableList()
            if (declaration.hasModifier(KtTokens.SUSPEND_KEYWORD)) {
                arguments += "Lkotlin/coroutines/Continuation;"
                "(${arguments.joinToString("")})Ljava/lang/Object;"
            } else "(${arguments.joinToString("")})${jvmTypeDescriptor(returnType, true, declaration)}"
        }
        is KtProperty -> jvmTypeDescriptor(returnType, false, declaration)
        is KtClassOrObject -> "L${(listOf(pkg).filter(String::isNotBlank) + declaration.name.orEmpty()).joinToString("/")};"
        else -> "Ljava/lang/Object;"
    }

    private fun jvmTypeDescriptor(rawType: String, returnPosition: Boolean, declaration: KtNamedDeclaration? = null, forceBoxed: Boolean = false): String {
        val nullable = rawType.trim().endsWith('?')
        val withoutNullability = rawType.trim().removeSuffix("?")
        if (withoutNullability.startsWith("kotlin/Array<") || withoutNullability.startsWith("kotlin.Array<") || withoutNullability.startsWith("Array<")) {
            val element = withoutNullability.substringAfter('<').substringBeforeLast('>')
            return "[${jvmTypeDescriptor(element, false, declaration, forceBoxed = true)}"
        }
        val type = withoutNullability.substringBefore('<').replace('.', '/')
        val typeParameter = (declaration as? KtTypeParameterListOwner)?.typeParameters?.firstOrNull { it.name == type }
        if (typeParameter != null) {
            return jvmTypeDescriptor(typeParameter.extendsBound?.text ?: "kotlin/Any", returnPosition, declaration, forceBoxed)
        }
        if (!nullable && !forceBoxed) when (type) {
            "Boolean", "kotlin/Boolean" -> return "Z"
            "Byte", "kotlin/Byte" -> return "B"
            "Char", "kotlin/Char" -> return "C"
            "Short", "kotlin/Short" -> return "S"
            "Int", "kotlin/Int" -> return "I"
            "Long", "kotlin/Long" -> return "J"
            "Float", "kotlin/Float" -> return "F"
            "Double", "kotlin/Double" -> return "D"
            "Unit", "kotlin/Unit" -> return if (returnPosition) "V" else "Lkotlin/Unit;"
            "BooleanArray", "kotlin/BooleanArray" -> return "[Z"
            "ByteArray", "kotlin/ByteArray" -> return "[B"
            "CharArray", "kotlin/CharArray" -> return "[C"
            "ShortArray", "kotlin/ShortArray" -> return "[S"
            "IntArray", "kotlin/IntArray" -> return "[I"
            "LongArray", "kotlin/LongArray" -> return "[J"
            "FloatArray", "kotlin/FloatArray" -> return "[F"
            "DoubleArray", "kotlin/DoubleArray" -> return "[D"
        }
        val boxed = when (type) {
            "Boolean", "kotlin/Boolean" -> "java/lang/Boolean"
            "Byte", "kotlin/Byte" -> "java/lang/Byte"
            "Char", "kotlin/Char" -> "java/lang/Character"
            "Short", "kotlin/Short" -> "java/lang/Short"
            "Int", "kotlin/Int" -> "java/lang/Integer"
            "Long", "kotlin/Long" -> "java/lang/Long"
            "Float", "kotlin/Float" -> "java/lang/Float"
            "Double", "kotlin/Double" -> "java/lang/Double"
            "String", "kotlin/String" -> "java/lang/String"
            "Number", "kotlin/Number" -> "java/lang/Number"
            "kotlin/collections/Iterable", "kotlin/collections/MutableIterable" -> "java/lang/Iterable"
            "kotlin/collections/Iterator", "kotlin/collections/MutableIterator" -> "java/util/Iterator"
            "kotlin/collections/Collection", "kotlin/collections/MutableCollection" -> "java/util/Collection"
            "kotlin/collections/List", "kotlin/collections/MutableList" -> "java/util/List"
            "kotlin/collections/ListIterator", "kotlin/collections/MutableListIterator" -> "java/util/ListIterator"
            "kotlin/collections/Set", "kotlin/collections/MutableSet" -> "java/util/Set"
            "kotlin/collections/Map", "kotlin/collections/MutableMap" -> "java/util/Map"
            "kotlin/collections/Map/Entry", "kotlin/collections/MutableMap/MutableEntry" -> "java/util/Map\$Entry"
            "Any", "kotlin/Any", "?", "<unresolved>" -> "java/lang/Object"
            else -> type.ifBlank { "java/lang/Object" }
        }
        return "L$boxed;"
    }

    private fun symbolId(pkg: String, declaration: KtNamedDeclaration, module: String = ":", sourceSet: String = "main"): String =
        symbolIdentity(pkg, declaration, module, sourceSet).toString()

    private fun symbolMatches(query: String, pkg: String, declaration: KtNamedDeclaration, module: String = ":", sourceSet: String = "main"): Boolean {
        val legacy = legacySymbolId(pkg, declaration)
        val containing = generateSequence(declaration.parent) { it.parent }.filterIsInstance<KtNamedDeclaration>().toList().asReversed().mapNotNull { it.name }
        val sourceFqn = (listOf(pkg) + containing + listOfNotNull(declaration.name)).filter(String::isNotBlank).joinToString(".")
        if (query == declaration.name || query == sourceFqn || query == symbolId(pkg, declaration, module, sourceSet) || query == legacy || query == legacy.substringBefore('(')) return true
        val identity = runCatching { json.parseToJsonElement(query).jsonObject }.getOrNull() ?: return false
        if (identity["module"]?.jsonPrimitive?.content != module || identity["sourceSet"]?.jsonPrimitive?.content != sourceSet || identity["package"]?.jsonPrimitive?.content != pkg) return false
        if (identity["declarationName"]?.jsonPrimitive?.content != declaration.name) return false
        if (identity["containingDeclarations"]?.jsonArray?.map { it.jsonPrimitive.content }.orEmpty() != containing) return false
        val function = declaration as? KtNamedFunction ?: return true
        val expectedParameters = identity["parameterTypes"]?.jsonArray?.map { normalizeIdentityType(it.jsonPrimitive.content) }.orEmpty()
        val actualParameters = function.valueParameters.map { normalizeIdentityType(it.typeReference?.text ?: "?") }
        val expectedReceivers = identity["receiverTypes"]?.jsonArray?.map { normalizeIdentityType(it.jsonPrimitive.content) }.orEmpty()
        val actualReceivers = function.receiverTypeReference?.text?.let { listOf(normalizeIdentityType(it)) }.orEmpty()
        return expectedParameters == actualParameters && expectedReceivers == actualReceivers
    }

    private fun normalizeIdentityType(type: String): String = type.trim().removePrefix("kotlin/").removePrefix("kotlin.").substringAfterLast('/').substringAfterLast('.')

    private fun sourceSignature(declaration: KtNamedDeclaration): String = when (declaration) {
        is KtNamedFunction -> buildString {
            append(if (declaration.hasModifier(KtTokens.SUSPEND_KEYWORD)) "suspend " else "")
            append(if (declaration.hasModifier(KtTokens.PRIVATE_KEYWORD)) "private " else if (declaration.hasModifier(KtTokens.PROTECTED_KEYWORD)) "protected " else if (declaration.hasModifier(KtTokens.INTERNAL_KEYWORD)) "internal " else "public ")
            append("fun "); declaration.receiverTypeReference?.text?.let { append(it).append('.') }; append(declaration.name)
            append(declaration.typeParameterList?.text.orEmpty())
            append(declaration.valueParameters.joinToString(",", "(", ")") { parameter -> "${parameter.name}:${parameter.typeReference?.text ?: "?"}" })
            append(':').append(declaration.typeReference?.text ?: "Unit")
        }
        is KtProperty -> buildString {
            append(if (declaration.hasModifier(KtTokens.PRIVATE_KEYWORD)) "private " else if (declaration.hasModifier(KtTokens.PROTECTED_KEYWORD)) "protected " else if (declaration.hasModifier(KtTokens.INTERNAL_KEYWORD)) "internal " else "public ")
            append(if (declaration.isVar) "var " else "val "); append(declaration.name).append(':').append(declaration.typeReference?.text ?: "?")
        }
        is KtClassOrObject -> buildString {
            append(declaration::class.simpleName).append(' ').append(declaration.name).append(declaration.typeParameterList?.text.orEmpty())
            append(':').append(declaration.superTypeListEntries.joinToString(",") { it.typeReference?.text.orEmpty() })
        }
        else -> declaration.name.orEmpty()
    }

    private fun resolveSymbol(repo: Path, query: String, compilation: String = ":/main"): JsonObject {
        val project = inspect(repo, compilation)
        val module = project["module"]?.jsonPrimitive?.content ?: ":"
        val sourceSet = project["sourceSet"]?.jsonPrimitive?.content ?: "main"
        val (path, kt, fn) = findFunction(repo, query, compilation)
        val analysis = analyzeWithK2(repo, compilation = compilation)
        val declaration = declarationJson(repo, path, kt.packageFqName.asString(), fn, analysis, module, sourceSet)
        val file = declaration["file"]!!.jsonPrimitive.content
        val resolvedSymbol = declaration["symbolId"]!!.jsonPrimitive.content
        val semanticFacts = fileFacts(repo, path, analysis).filter { fact ->
            val start = fact["start"]?.jsonPrimitive?.intOrNull ?: -1
            val end = fact["end"]?.jsonPrimitive?.intOrNull ?: -1
            start >= fn.textRange.startOffset && end <= fn.textRange.endOffset
        }
        return buildJsonObject {
            put("schema", "semantic-symbol/0.1"); put("declaration", declaration)
            fn.bodyExpression?.let { put("bodyAnchor", anchor(file, resolvedSymbol, it, kt.text)) }
            putJsonArray("references") { fn.bodyExpression?.let(::usedNames).orEmpty().sorted().forEach(::add) }
            putJsonArray("calls") { PsiTreeUtil.collectElementsOfType(fn, KtCallExpression::class.java).map { it.calleeExpression?.text.orEmpty() }.sorted().forEach(::add) }
            putJsonArray("declaredTypes") { (fn.valueParameters.mapNotNull { it.typeReference?.text } + listOfNotNull(fn.typeReference?.text)).sorted().forEach(::add) }
            putJsonArray("semanticFacts") { semanticFacts.forEach(::add) }
            putJsonArray("resolvedCalls") { semanticFacts.filter { "FunctionCall" in (it["kind"]?.jsonPrimitive?.content.orEmpty()) }.forEach(::add) }
            put("semanticSummaryHash", sha(buildJsonObject { put("bodyHash", declaration["bodyHash"]!!); putJsonArray("facts") { semanticFacts.map(::semanticSignature).forEach(::add) } }.toString().toByteArray()))
            put("k2Validated", analysis.valid); putJsonArray("diagnostics") { analysis.diagnostics.forEach(::add) }
        }
    }

    private fun findFunction(repo: Path, query: String, compilation: String = ":/main"): Triple<Path, KtFile, KtNamedFunction> {
        val project = inspect(repo, compilation)
        val module = project["module"]?.jsonPrimitive?.content ?: ":"
        val sourceSet = project["sourceSet"]?.jsonPrimitive?.content ?: "main"
        val fullIdentityQuery = query.trimStart().startsWith('{')
        val analysis = if (fullIdentityQuery) analyzeWithK2(repo, compilation = compilation) else null
        val matches = mutableListOf<Triple<Path, KtFile, KtNamedFunction>>()
        compilationSourceFiles(repo, compilation).forEach { path ->
            val kt = parse(path); val pkg = kt.packageFqName.asString()
            PsiTreeUtil.collectElementsOfType(kt, KtNamedFunction::class.java).forEach { fn ->
                val matchesQuery = if (analysis == null) {
                    symbolMatches(query, pkg, fn, module, sourceSet)
                } else {
                    declarationJson(repo, path, pkg, fn, analysis, module, sourceSet)["symbolId"]?.jsonPrimitive?.content == query
                }
                if (matchesQuery) matches += Triple(path, kt, fn)
            }
        }
        if (matches.isEmpty()) throw WorkerFailure("SYMBOL_NOT_FOUND", "symbol not found: $query")
        if (matches.size > 1) throw WorkerFailure("AMBIGUOUS_SYMBOL", "symbol is ambiguous: $query")
        return matches.single()
    }

    private fun resolveExpression(repo: Path, relative: String, offset: Int, compilation: String = ":/main"): JsonObject {
        val path = repo.resolve(relative).normalize(); require(path.startsWith(repo.normalize())) { "file escapes repository" }
        require(path in compilationSourceFiles(repo, compilation).map(Path::normalize)) { "file is outside selected compilation" }
        val kt = parse(path); val leaf = kt.findElementAt(offset) ?: throw WorkerFailure("EXPRESSION_NOT_FOUND", "no element at offset $offset")
        val expression = generateSequence(leaf as PsiElement?) { it.parent }.filterIsInstance<KtExpression>().firstOrNull { isGraphExpression(it) }
            ?: throw WorkerFailure("EXPRESSION_NOT_FOUND", "no expression at offset $offset")
        val owner = generateSequence(expression.parent) { it.parent }.filterIsInstance<KtNamedFunction>().firstOrNull()
            ?: throw WorkerFailure("EXPRESSION_NOT_FOUND", "expression is outside a function")
        val project = inspect(repo, compilation)
        val analysis = analyzeWithK2(repo, compilation = compilation)
        val symbol = declarationJson(repo, path, kt.packageFqName.asString(), owner, analysis, project["module"]?.jsonPrimitive?.content ?: ":", project["sourceSet"]?.jsonPrimitive?.content ?: "main")["symbolId"]!!.jsonPrimitive.content
        return buildJsonObject { put("schema", "semantic-anchor/0.1"); put("anchor", anchor(relative, symbol, expression, kt.text)) }
    }

    private fun anchor(file: String, owner: String, node: PsiElement, source: String): JsonObject {
        val start = node.textRange.startOffset; val end = node.textRange.endOffset
        val ancestor = generateSequence(node.parent) { it.parent }.takeWhile { it !is KtNamedFunction }.map { it::class.simpleName.orEmpty() }.toList()
        val sameKind = generateSequence(node.parent) { it.parent }.firstOrNull()?.children?.filter { it::class == node::class } ?: emptyList()
        return buildJsonObject {
            put("fileId", file); put("ownerSymbolId", owner); put("syntaxKind", node::class.simpleName ?: "PsiElement")
            put("normalizedTokenHash", sha(normalizeTokens(node.text).toByteArray())); put("ancestorPathHash", sha(ancestor.joinToString("/").toByteArray()))
            put("localOrdinal", sameKind.indexOf(node).coerceAtLeast(0)); put("leftContextHash", sha(source.substring(maxOf(0, start - 64), start).toByteArray()))
            put("rightContextHash", sha(source.substring(end, minOf(source.length, end + 64)).toByteArray())); put("exactTextHash", sha(node.text.toByteArray()))
            putJsonArray("rangeHint") { add(start); add(end) }; put("sourceText", node.text)
            generateSequence(node as PsiElement?) { it.parent }.filterIsInstance<KtNamedFunction>().firstOrNull()?.let { function ->
                val signature = function.text.substringBefore(function.bodyExpression?.text ?: "")
                put("ownerSignatureHash", sha(signature.toByteArray()))
            }
            put("anchorId", "anchor:" + sha("$file|$owner|${node::class.simpleName}|${normalizeTokens(node.text)}|${ancestor.joinToString("/")}".toByteArray()).removePrefix("sha256:"))
        }
    }

    private fun localGraph(repo: Path, query: String, compilation: String = ":/main"): JsonObject {
        val (path, kt, fn) = findFunction(repo, query, compilation); val relative = repo.relativize(path).invariantSeparatorsPathString
        val project = inspect(repo, compilation)
        val analysis = analyzeWithK2(repo, compilation = compilation)
        val owner = declarationJson(repo, path, kt.packageFqName.asString(), fn, analysis, project["module"]?.jsonPrimitive?.content ?: ":", project["sourceSet"]?.jsonPrimitive?.content ?: "main")["symbolId"]!!.jsonPrimitive.content
        val firCfg = cfgRecords(repo, path, analysis).firstOrNull {
            it["start"]?.jsonPrimitive?.intOrNull == fn.textRange.startOffset && it["end"]?.jsonPrimitive?.intOrNull == fn.textRange.endOffset
        }
        val graph = if (firCfg == null) LocalCfgBuilder(relative, owner, kt.text).build(fn) else normalizeFirCfg(repo, relative, owner, kt, fn, firCfg, analysis, compilation)
        return enrichGraph(repo, path, graph, analysis)
    }

    private fun fileFacts(repo: Path, path: Path, analysis: K2Analysis): List<JsonObject> {
        val absolute = path.toAbsolutePath().normalize().invariantSeparatorsPathString
        val relative = repo.relativize(path).invariantSeparatorsPathString
        return analysis.facts.filter { fact ->
            if (fact["recordType"]?.jsonPrimitive?.content != "SEMANTIC_FACT") return@filter false
            val value = fact["file"]?.jsonPrimitive?.content?.replace('\\', '/') ?: return@filter false
            value == absolute || value == relative || value.endsWith("/$relative")
        }.sortedWith(compareBy({ it["start"]?.jsonPrimitive?.intOrNull ?: -1 }, { it["end"]?.jsonPrimitive?.intOrNull ?: -1 }, { it.toString() }))
    }

    private fun cfgRecords(repo: Path, path: Path, analysis: K2Analysis): List<JsonObject> {
        val absolute = path.toAbsolutePath().normalize().invariantSeparatorsPathString
        val relative = repo.relativize(path).invariantSeparatorsPathString
        return analysis.facts.filter { fact ->
            if (fact["recordType"]?.jsonPrimitive?.content != "FIR_CFG") return@filter false
            val value = fact["file"]?.jsonPrimitive?.content?.replace('\\', '/') ?: return@filter false
            value == absolute || value == relative || value.endsWith("/$relative")
        }
    }

    private fun normalizeFirCfg(repo: Path, file: String, owner: String, kt: KtFile, fn: KtNamedFunction, cfg: JsonObject, analysis: K2Analysis, compilation: String): JsonObject {
        val rawNodes = cfg["nodes"]!!.jsonArray
        val functionLocals = (fn.valueParameters.mapNotNull { it.name } + PsiTreeUtil.collectElementsOfType(fn.bodyExpression ?: fn, KtProperty::class.java).filter { property -> generateSequence(property.parent) { it.parent }.takeWhile { it != fn }.none { it is KtLambdaExpression } }.mapNotNull { it.name }).toSet()
        val hasClassReceiver = generateSequence(fn.parent) { it.parent }.any { it is KtClassOrObject }
        val normalizedNodes = rawNodes.map { value ->
            val raw = value.jsonObject; val rawId = raw["id"]!!.jsonPrimitive.int
            val start = raw["start"]?.jsonPrimitive?.intOrNull; val end = raw["end"]?.jsonPrimitive?.intOrNull
            val psi = if (start != null && end != null && start >= fn.textRange.startOffset && end <= fn.textRange.endOffset) {
                generateSequence(kt.findElementAt(start) as PsiElement?) { it.parent }.firstOrNull { it.textRange.startOffset == start && it.textRange.endOffset == end }
            } else null
            val rawKind = raw["kind"]!!.jsonPrimitive.content
            val expression = (psi as? KtExpression)?.takeUnless { it is KtNamedFunction || it is KtBlockExpression || it is KtWhenExpression || it is KtIfExpression || it is KtLoopExpression }
            val kind = when {
                "FunctionEnter" in rawKind -> "ENTRY"
                "FunctionExit" in rawKind -> "EXIT"
                rawKind == "FunctionCallEnterNode" -> "CALL"
                rawKind == "FunctionCallExitNode" -> "CALL_RESULT"
                expression is KtCallExpression && "Arguments" !in rawKind -> "CALL"
                "BooleanOperator" in rawKind || "ElvisLhs" in rawKind || "SafeCall" in rawKind -> "BRANCH"
                "Condition" in rawKind || "Branch" in rawKind || "When" in rawKind -> "BRANCH"
                "Enter" in rawKind -> "EXPRESSION"
                "Exit" in rawKind -> "EXPRESSION"
                "Throw" in rawKind -> "THROW"
                "Jump" in rawKind || "Return" in rawKind -> "RETURN"
                "Loop" in rawKind -> "LOOP"
                expression != null -> normalizedKind(expression)
                else -> "EXPRESSION"
            }
            val defines = when {
                "VariableDeclaration" in rawKind -> expression?.let(::definedName)
                "VariableAssignment" in rawKind -> expression?.let(::definedName)
                else -> null
            }
            var uses = when {
                "QualifiedAccess" in rawKind -> expression?.let(::usedNames).orEmpty()
                "FunctionCallExit" in rawKind || "VariableAssignment" in rawKind || "VariableDeclaration" in rawKind -> expression?.let(::normalizedUses).orEmpty()
                "ConditionExit" in rawKind -> expression?.let(::usedNames).orEmpty()
                "Jump" in rawKind || "Throw" in rawKind -> expression?.let(::usedNames).orEmpty()
                else -> emptyList()
            }
            if (uses.isEmpty() && start != null && end != null && "ConditionExit" in rawKind) {
                val conditionText = kt.text.substring(start.coerceAtLeast(0), end.coerceAtMost(kt.text.length)).trim()
                if (conditionText.matches(Regex("[A-Za-z_][A-Za-z0-9_]*"))) uses = listOf(conditionText)
            }
            if (uses.isEmpty() && start != null && end != null && "Jump" in rawKind) {
                val returnedName = kt.text.substring(start.coerceAtLeast(0), end.coerceAtMost(kt.text.length)).trim()
                if (returnedName.matches(Regex("[A-Za-z_][A-Za-z0-9_]*"))) uses = listOf(returnedName)
            }
            graphNode("fir:$rawId", kind, defines, psi?.let { anchor(file, owner, it, kt.text) }, uses).let { node ->
                buildJsonObject { node.forEach { (key, item) -> put(key, item) }; putJsonObject("attributes") {
                    put("firNodeKind", rawKind); put("firDead", raw["dead"] ?: JsonPrimitive(false)); put("analysis", "K2_FIR_CFG")
                    cfg["symbol"]?.let { put("ownerCompilerSymbol", it) }
                    if (hasClassReceiver && ((defines != null && defines !in functionLocals) || uses.any { it !in functionLocals })) {
                        putJsonArray("effects") {
                            if (uses.any { it !in functionLocals }) add("READ_STATE")
                            if (defines != null && defines !in functionLocals) add("WRITE_STATE")
                        }
                        val property = defines?.takeIf { it !in functionLocals } ?: uses.first { it !in functionLocals }
                        put("memoryKind", "THIS_PROPERTY"); put("memoryLocation", "THIS_PROPERTY:$owner#$property")
                    }
                } }
            }
        }
        val rawKinds = rawNodes.associate { it.jsonObject["id"]!!.jsonPrimitive.int to it.jsonObject["kind"]!!.jsonPrimitive.content }
        val rawTexts = rawNodes.associate { value ->
            val raw = value.jsonObject
            val start = raw["start"]?.jsonPrimitive?.intOrNull
            val end = raw["end"]?.jsonPrimitive?.intOrNull
            raw["id"]!!.jsonPrimitive.int to if (start != null && end != null && start >= 0 && end <= kt.text.length) kt.text.substring(start, end) else ""
        }
        val rawEdgePairs = cfg["edges"]!!.jsonArray.map { value ->
            value.jsonObject["from"]!!.jsonPrimitive.int to value.jsonObject["to"]!!.jsonPrimitive.int
        }
        val safeCallBranchSources = rawEdgePairs.groupBy({ it.first }, { it.second }).filterValues { targets ->
            targets.any { "EnterSafeCall" in rawKinds[it].orEmpty() } && targets.any { "ElvisRhs" in rawKinds[it].orEmpty() }
        }.keys
        val rawEdges = cfg["edges"]!!.jsonArray.map { value ->
            val raw = value.jsonObject; val label = raw["label"]!!.jsonPrimitive.content; val edgeKind = raw["edgeKind"]!!.jsonPrimitive.content
            val from = raw["from"]!!.jsonPrimitive.int; val to = raw["to"]!!.jsonPrimitive.int
            val fromKind = rawKinds[from].orEmpty(); val toKind = rawKinds[to].orEmpty()
            val branchText = rawTexts[from].orEmpty()
            val kind = when {
                "True" in label -> "CFG_TRUE"
                "False" in label -> "CFG_FALSE"
                "Exception" in label || "Exception" in edgeKind -> "CFG_EXCEPTION"
                "BooleanOperatorExitLeftOperand" in fromKind && "EnterRightOperand" in toKind && "&&" in branchText -> "CFG_TRUE"
                "BooleanOperatorExitLeftOperand" in fromKind && "EnterRightOperand" in toKind && "||" in branchText -> "CFG_FALSE"
                "BooleanOperatorExitLeftOperand" in fromKind && "BooleanOperatorExit" in toKind && "&&" in branchText -> "CFG_FALSE"
                "BooleanOperatorExitLeftOperand" in fromKind && "BooleanOperatorExit" in toKind && "||" in branchText -> "CFG_TRUE"
                "ElvisLhsExit" in fromKind && "ElvisLhsIsNotNull" in toKind -> "CFG_TRUE"
                "ElvisLhsExit" in fromKind && "ElvisRhs" in toKind -> "CFG_FALSE"
                "EnterSafeCall" in toKind -> "CFG_TRUE"
                from in safeCallBranchSources && "ElvisRhs" in toKind -> "CFG_FALSE"
                "ConditionExit" in fromKind && ("SyntheticElse" in toKind || "Exit" in toKind) -> "CFG_FALSE"
                "ConditionExit" in fromKind -> "CFG_TRUE"
                "Throw" in fromKind -> "THROW"
                "Jump" in fromKind && "FunctionExit" in toKind -> "RETURN"
                "Backward" in edgeKind || to <= from && "Loop" in toKind -> "CFG_BACK"
                else -> "CFG_NORMAL"
            }
            edge("fir:$from", "fir:$to", kind)
        }
        val entry = rawKinds.entries.firstOrNull { "FunctionEnter" in it.value }?.key
        val entryEdges = if (entry == null || fn.valueParameters.isEmpty()) emptyList() else rawEdges.filter { it["from"]?.jsonPrimitive?.content == "fir:$entry" }
        val normalizedEdges = rawEdges.filterNot { it in entryEdges }.toMutableList()
        val parameterNodes = fn.valueParameters.mapIndexed { index, parameter ->
            val node = graphNode("param:$index", "PARAMETER", parameter.name, anchor(file, owner, parameter, kt.text))
            buildJsonObject {
                node.forEach { (key, value) -> put(key, value) }
                putJsonObject("attributes") {
                    put("analysis", "K2_FIR")
                    cfg["symbol"]?.let { put("ownerCompilerSymbol", it) }
                    cfg["parameterTypes"]?.jsonArray?.getOrNull(index)?.let { put("declaredType", it) }
                    cfg["returnType"]?.let { put("ownerReturnType", it) }
                }
            }
        }
        if (entry != null && parameterNodes.isNotEmpty()) {
            normalizedEdges += edge("fir:$entry", "param:0", "CFG_NORMAL")
            for (index in 0 until parameterNodes.lastIndex) normalizedEdges += edge("param:$index", "param:${index + 1}", "CFG_NORMAL")
            entryEdges.forEach { original -> normalizedEdges += edge("param:${parameterNodes.lastIndex}", original["to"]!!.jsonPrimitive.content, original["kind"]!!.jsonPrimitive.content) }
        }
        val outerNames = functionLocals + fn.valueParameters.mapNotNull { it.name }
        val captureNodes = mutableListOf<JsonObject>()
        PsiTreeUtil.collectElementsOfType(fn.bodyExpression ?: fn, KtLambdaExpression::class.java).sortedBy { it.textOffset }.forEachIndexed { lambdaIndex, lambda ->
            val localNames = lambda.valueParameters.mapNotNull { it.name }.toSet() + PsiTreeUtil.collectElementsOfType(lambda, KtProperty::class.java).mapNotNull { it.name }
            val captured = usedNames(lambda).filter { it in outerNames && it !in localNames }.distinct().sorted()
            captured.forEach { name ->
                val id = "capture:$lambdaIndex:$name"
                captureNodes += graphNode(id, "CAPTURE", null, anchor(file, owner, lambda, kt.text), listOf(name))
                val host = normalizedNodes.filter { node ->
                    node["kind"]?.jsonPrimitive?.content == "CALL" && node["origin"]?.jsonObject?.get("rangeHint")?.jsonArray?.let { range ->
                        (range[0].jsonPrimitive.int <= lambda.textRange.startOffset && range[1].jsonPrimitive.int >= lambda.textRange.endOffset)
                    } == true
                }.minByOrNull { node -> node["origin"]!!.jsonObject["rangeHint"]!!.jsonArray.let { it[1].jsonPrimitive.int - it[0].jsonPrimitive.int } }
                host?.get("id")?.jsonPrimitive?.content?.let { normalizedEdges += edge(it, id, "CAPTURE") }
            }
        }
        val callEdges = mutableListOf<JsonObject>()
        normalizedNodes.filter { it["kind"]?.jsonPrimitive?.content == "CALL" }.forEach { callNode ->
            val callId = callNode["id"]!!.jsonPrimitive.content
            val callRange = callNode["origin"]?.jsonObject?.get("rangeHint")?.jsonArray
            val callStart = callRange?.getOrNull(0)?.jsonPrimitive?.intOrNull
            val callEnd = callRange?.getOrNull(1)?.jsonPrimitive?.intOrNull
            val mappings = callNode["attributes"]?.jsonObject?.get("argumentToParameter")?.jsonArray.orEmpty()
            mappings.forEach { mapping ->
                val argumentStart = mapping.jsonObject["argumentStart"]?.jsonPrimitive?.intOrNull ?: return@forEach
                normalizedNodes.filter { candidate ->
                    candidate["id"]?.jsonPrimitive?.content != callId && candidate["origin"]?.jsonObject?.get("rangeHint")?.jsonArray?.let { range ->
                        range[0].jsonPrimitive.int <= argumentStart && range[1].jsonPrimitive.int > argumentStart
                    } == true
                }.minByOrNull { candidate -> candidate["origin"]!!.jsonObject["rangeHint"]!!.jsonArray.let { it[1].jsonPrimitive.int - it[0].jsonPrimitive.int } }
                    ?.get("id")?.jsonPrimitive?.content?.let { callEdges += edge(it, callId, "ARG_PARAM") }
            }
            if (callStart != null && callEnd != null) {
                val callPsi = generateSequence(kt.findElementAt(callStart) as PsiElement?) { it.parent }
                    .filterIsInstance<KtCallExpression>().firstOrNull { it.textRange.startOffset >= callStart && it.textRange.endOffset <= callEnd }
                val receiver = (callPsi?.parent as? KtQualifiedExpression)?.receiverExpression
                if (receiver != null) {
                    normalizedNodes.filter { candidate ->
                        candidate["id"]?.jsonPrimitive?.content != callId && candidate["origin"]?.jsonObject?.get("rangeHint")?.jsonArray?.let { range ->
                            range[0].jsonPrimitive.int == receiver.textRange.startOffset && range[1].jsonPrimitive.int == receiver.textRange.endOffset
                        } == true
                    }.minByOrNull { it["id"]!!.jsonPrimitive.content }
                        ?.get("id")?.jsonPrimitive?.content?.let { callEdges += edge(it, callId, "RECEIVER") }
                }
            }
        }
        val project = inspect(repo, compilation)
        val module = project["module"]?.jsonPrimitive?.content ?: ":"
        val sourceSet = project["sourceSet"]?.jsonPrimitive?.content ?: "main"
        val inheritance = PsiTreeUtil.collectElementsOfType(kt, KtClassOrObject::class.java).map { declaration ->
            buildJsonObject {
                put("symbol", symbolId(kt.packageFqName.asString(), declaration, module, sourceSet))
                putJsonArray("supertypes") { declaration.superTypeListEntries.map { it.typeReference?.text.orEmpty() }.sorted().forEach(::add) }
            }
        }.sortedBy { it.toString() }
        return buildJsonObject {
            put("schema", "local-cfg/0.1"); put("symbol", owner); put("file", file); put("graphSource", "K2_FIR_CFG")
            putJsonArray("nodes") { (normalizedNodes + parameterNodes + captureNodes).forEach(::add) }; putJsonArray("edges") { (normalizedEdges + callEdges).distinctBy { it.toString() }.forEach(::add) }; putJsonArray("boundaries") {}
            putJsonArray("diagnostics") { normalizedDiagnostics(analysis.diagnostics).forEach(::add) }
            put("compilerOptionsHash", sha(buildJsonObject { put("languageVersion", project["languageVersion"]!!); put("apiVersion", project["apiVersion"]!!); put("jvmTarget", project["jvmTarget"]!!); put("freeCompilerArguments", project["freeCompilerArguments"]!!); put("compilerPlugins", project["compilerPlugins"]!!); put("compilerPluginOptions", project["compilerPluginOptions"]!!) }.toString().toByteArray()))
            put("classpathHash", sha(project["compileClasspath"]!!.toString().toByteArray()))
            putJsonArray("inheritanceFacts") { inheritance.forEach(::add) }
            put("firCfgHash", sha(cfg.toString().toByteArray()))
        }
    }

    private fun enrichGraph(repo: Path, path: Path, graph: JsonObject, analysis: K2Analysis): JsonObject {
        val facts = fileFacts(repo, path, analysis)
        val kt = parse(path)
        val objectSites = PsiTreeUtil.collectElementsOfType(kt, KtObjectDeclaration::class.java).mapNotNull { declaration ->
            val name = declaration.name ?: return@mapNotNull null
            val owners = generateSequence(declaration.parent) { it.parent }.filterIsInstance<KtNamedDeclaration>().toList().asReversed().mapNotNull { it.name }
            val compilerPrefix = (listOf(kt.packageFqName.asString().replace('.', '/')) + owners + name).filter(String::isNotBlank).joinToString("/")
            compilerPrefix to declaration.textRange.startOffset
        }.toMap()
        val nodes = graph["nodes"]!!.jsonArray.map { raw ->
            val node = raw.jsonObject
            val range = node["origin"]?.jsonObject?.get("rangeHint")?.jsonArray
            val start = range?.getOrNull(0)?.jsonPrimitive?.intOrNull
            val end = range?.getOrNull(1)?.jsonPrimitive?.intOrNull
            val candidates = if (start == null || end == null) emptyList() else facts.filter {
                val fs = it["start"]?.jsonPrimitive?.intOrNull ?: -1; val fe = it["end"]?.jsonPrimitive?.intOrNull ?: -1
                fs >= start && fe <= end || fs <= start && fe >= end
            }
            val firKind = node["attributes"]?.jsonObject?.get("firNodeKind")?.jsonPrimitive?.content.orEmpty()
            val preferred = candidates.filter { fact ->
                val factKind = fact["kind"]?.jsonPrimitive?.content.orEmpty()
                when {
                    "FunctionCall" in firKind -> "FunctionCall" in factKind
                    "QualifiedAccess" in firKind -> "PropertyAccess" in factKind || "QualifiedAccess" in factKind
                    "VariableAssignment" in firKind -> "PropertyAccess" in factKind && fact["receiverType"] != null
                    "Literal" in firKind -> "Literal" in factKind
                    "Jump" in firKind -> "Return" in factKind
                    "Throw" in firKind -> "Throw" in factKind
                    else -> false
                }
            }
            val matching = preferred.firstOrNull {
                it["start"]?.jsonPrimitive?.intOrNull == start && it["end"]?.jsonPrimitive?.intOrNull == end
            } ?: if ("VariableAssignment" in firKind) {
                preferred.maxByOrNull { (it["end"]?.jsonPrimitive?.intOrNull ?: 0) - (it["start"]?.jsonPrimitive?.intOrNull ?: 0) }
            } else {
                preferred.minByOrNull { kotlin.math.abs((it["end"]?.jsonPrimitive?.intOrNull ?: end!!) - (it["start"]?.jsonPrimitive?.intOrNull ?: start!!)) }
            }
            buildJsonObject {
                node.forEach { (key, value) -> put(key, value) }
                putJsonObject("attributes") {
                    node["attributes"]?.jsonObject?.forEach { (key, value) -> put(key, value) }
                    put("analysis", "K2_FIR")
                    matching?.let { fact -> listOf("kind", "type", "symbol", "returnType", "receiverType", "argumentToParameter").forEach { key -> fact[key]?.let { put(key, it) } } }
                    val resolvedSymbol = matching?.get("symbol")?.jsonPrimitive?.content.orEmpty()
                    val factKind = matching?.get("kind")?.jsonPrimitive?.content.orEmpty()
                    val objectSite = objectSites.entries.firstOrNull { (prefix, _) -> resolvedSymbol.startsWith("$prefix.") }
                    val staticProperty = "PropertyAccess" in factKind && resolvedSymbol.startsWith("java/")
                    val receiverType = matching?.get("receiverType")?.jsonPrimitive?.content.orEmpty()
                    val unknownHeapProperty = objectSite == null && !staticProperty && receiverType.isNotEmpty() &&
                        ("PropertyAccess" in factKind || "VariableAssignment" in factKind)
                    val mergedEffects = (node["attributes"]?.jsonObject?.get("effects")?.jsonArray?.map { it.jsonPrimitive.content }.orEmpty()
                        + matching?.get("effects")?.jsonArray?.map { it.jsonPrimitive.content }.orEmpty()
                        + (if (objectSite != null || staticProperty || unknownHeapProperty) listOf("READ_STATE") else emptyList())
                        + (if ((objectSite != null || staticProperty || unknownHeapProperty) && ("VariableAssignment" in firKind || "VariableAssignment" in factKind)) listOf("WRITE_STATE") else emptyList())).distinct().sorted()
                    if (mergedEffects.isNotEmpty()) putJsonArray("effects") { mergedEffects.forEach(::add) }
                    if (objectSite != null) {
                        put("memoryKind", "OBJECT_PROPERTY")
                        put("memoryLocation", "OBJECT_PROPERTY:${objectSite.value}:$resolvedSymbol")
                    } else if (staticProperty) {
                        put("memoryKind", "STATIC_PROPERTY")
                        put("memoryLocation", "STATIC_PROPERTY:$resolvedSymbol")
                    } else if (unknownHeapProperty && node["attributes"]?.jsonObject?.get("memoryKind") == null) {
                        put("memoryKind", "UNKNOWN_HEAP")
                        put("memoryLocation", "UNKNOWN_HEAP:$resolvedSymbol")
                    }
                    matching?.get("symbol")?.jsonPrimitive?.content?.let { symbol -> calleeSummaryHash(repo, analysis, symbol)?.let { put("calleeSummaryHash", it) } }
                }
            }
        }
        val semanticEdges = graph["edges"]!!.jsonArray.map { it.jsonObject }.toMutableList()
        nodes.filter { it["kind"]?.jsonPrimitive?.content == "CALL" }.forEach { callNode ->
            val callId = callNode["id"]!!.jsonPrimitive.content
            callNode["attributes"]?.jsonObject?.get("argumentToParameter")?.jsonArray.orEmpty().forEach { mapping ->
                val argumentStart = mapping.jsonObject["argumentStart"]?.jsonPrimitive?.intOrNull ?: return@forEach
                nodes.filter { candidate ->
                    candidate["id"]?.jsonPrimitive?.content != callId && candidate["origin"]?.jsonObject?.get("rangeHint")?.jsonArray?.let { range ->
                        range[0].jsonPrimitive.int <= argumentStart && range[1].jsonPrimitive.int > argumentStart
                    } == true
                }.minByOrNull { candidate -> candidate["origin"]!!.jsonObject["rangeHint"]!!.jsonArray.let { it[1].jsonPrimitive.int - it[0].jsonPrimitive.int } }
                    ?.get("id")?.jsonPrimitive?.content?.let { semanticEdges += edge(it, callId, "ARG_PARAM") }
            }
            val callRange = callNode["origin"]?.jsonObject?.get("rangeHint")?.jsonArray
            val callStart = callRange?.getOrNull(0)?.jsonPrimitive?.intOrNull
            val callEnd = callRange?.getOrNull(1)?.jsonPrimitive?.intOrNull
            if (callStart != null && callEnd != null) {
                val qualified = PsiTreeUtil.collectElementsOfType(kt, KtQualifiedExpression::class.java)
                    .filter { it.textRange.startOffset == callStart && it.textRange.endOffset == callEnd }
                    .minByOrNull { it.textLength }
                val receiver = qualified?.receiverExpression
                if (receiver != null) {
                    nodes.filter { candidate -> candidate["origin"]?.jsonObject?.get("rangeHint")?.jsonArray?.let { range ->
                        range[0].jsonPrimitive.int == receiver.textRange.startOffset && range[1].jsonPrimitive.int == receiver.textRange.endOffset
                    } == true }.minByOrNull { it["id"]!!.jsonPrimitive.content }
                        ?.get("id")?.jsonPrimitive?.content?.let { semanticEdges += edge(it, callId, "RECEIVER") }
                }
            }
        }
        return buildJsonObject {
            graph.forEach { (key, value) -> if (key != "nodes" && key != "edges" && key != "boundaries") put(key, value) }
            put("graphSource", graph["graphSource"] ?: JsonPrimitive("K2_FIR_VALIDATED_STRUCTURED_CFG"))
            putJsonArray("nodes") { nodes.forEach(::add) }
            putJsonArray("edges") { semanticEdges.distinctBy { it.toString() }.sortedBy { it.toString() }.forEach(::add) }
            putJsonArray("boundaries") {
                graph["boundaries"]?.jsonArray?.forEach(::add)
                if (!analysis.valid) add(buildJsonObject { put("kind", "INCOMPLETE_SEMANTIC_ANALYSIS"); putJsonArray("diagnostics") { analysis.diagnostics.forEach(::add) } })
            }
        }
    }

    private fun calleeSummaryHash(repo: Path, analysis: K2Analysis, symbol: String): String? {
        val cfg = analysis.facts.firstOrNull { it["recordType"]?.jsonPrimitive?.content == "FIR_CFG" && it["symbol"]?.jsonPrimitive?.content == symbol } ?: return null
        val fileName = cfg["file"]?.jsonPrimitive?.content ?: return null
        val path = Path.of(fileName)
        if (!path.isRegularFile()) return null
        val start = cfg["start"]?.jsonPrimitive?.intOrNull ?: return null; val end = cfg["end"]?.jsonPrimitive?.intOrNull ?: return null
        val text = path.readText()
        val bodyHash = sha(normalizeTokens(text.substring(start.coerceAtLeast(0), end.coerceAtMost(text.length))).toByteArray())
        val facts = analysis.facts.filter {
            it["recordType"]?.jsonPrimitive?.content == "SEMANTIC_FACT" && it["file"]?.jsonPrimitive?.content == fileName && (it["start"]?.jsonPrimitive?.intOrNull ?: -1) >= start && (it["end"]?.jsonPrimitive?.intOrNull ?: -1) <= end
        }.map(::semanticSignature)
        return sha(buildJsonObject { put("symbol", symbol); put("bodyHash", bodyHash); putJsonArray("facts") { facts.forEach(::add) } }.toString().toByteArray())
    }

    private fun graphNode(id: String, kind: String, defines: String?, origin: JsonObject?, uses: List<String> = emptyList()) = buildJsonObject {
        put("id", id); put("kind", kind); if (defines != null) put("defines", defines); putJsonArray("uses") { uses.sorted().forEach(::add) }; if (origin != null) put("origin", origin)
    }
    private fun edge(from: String, to: String, kind: String) = buildJsonObject { put("from", from); put("to", to); put("kind", kind) }
    private fun definedName(e: KtExpression): String? = when (e) { is KtProperty -> e.name; is KtBinaryExpression -> if (e.operationReference.text.contains("=")) (e.left as? KtNameReferenceExpression)?.getReferencedName() else null; else -> null }
    private fun usedNames(e: PsiElement): List<String> = (listOfNotNull((e as? KtNameReferenceExpression)?.getReferencedName()) + PsiTreeUtil.collectElementsOfType(e, KtNameReferenceExpression::class.java).map { it.getReferencedName() }).distinct()
    private fun normalizedKind(e: KtExpression) = when (e) { is KtCallExpression -> "CALL"; is KtProperty -> "DEFINITION"; is KtBinaryExpression -> if (e.operationReference.text.contains("=")) "ASSIGNMENT" else "EXPRESSION"; else -> "EXPRESSION" }
    private fun normalizedUses(e: KtExpression): List<String> {
        val names = usedNames(e).toMutableList()
        if (e is KtProperty) e.name?.let(names::remove)
        if (e is KtBinaryExpression && e.operationReference.text == "=") (e.left as? KtNameReferenceExpression)?.getReferencedName()?.let(names::remove)
        return names.distinct()
    }
    private fun isGraphExpression(e: KtExpression) = e is KtReturnExpression || e is KtProperty || e is KtBinaryExpression || e is KtCallExpression || e is KtIfExpression || e is KtWhenExpression || e is KtLoopExpression || e is KtThrowExpression || e is KtTryExpression || e is KtBreakExpression || e is KtContinueExpression || e is KtSafeQualifiedExpression

    private inner class LocalCfgBuilder(private val file: String, private val owner: String, private val source: String) {
        private val nodes = mutableListOf<JsonObject>()
        private val edges = mutableListOf<JsonObject>()
        private val boundaries = mutableListOf<JsonObject>()
        private var counter = 0

        fun build(fn: KtNamedFunction): JsonObject {
            nodes += graphNode("entry", "ENTRY", null, null)
            nodes += graphNode("exit", "EXIT", null, null)
            nodes += graphNode("exception_exit", "EXCEPTION_EXIT", null, null)
            val body = fn.bodyExpression
            var bodyEntry = "exit"
            if (body == null) boundaries += boundary("MISSING_BODY", fn)
            else if (body is KtBlockExpression) bodyEntry = buildElement(body, CfgNext("exit"), null, CfgNext("exception_exit", "CFG_EXCEPTION"))
            else {
                val implicitReturn = addNode(body, "RETURN", null, usedNames(body))
                edges += edge(implicitReturn, "exit", "CFG_NORMAL")
                bodyEntry = if (needsOwnNode(body)) {
                    val valueEntry = buildElement(body, CfgNext(implicitReturn, "RETURN"), null, CfgNext("exception_exit", "CFG_EXCEPTION"))
                    edges += edge(valueEntry, implicitReturn, "RETURN")
                    valueEntry
                } else implicitReturn
            }
            var previous = "entry"
            fn.valueParameters.forEachIndexed { index, parameter ->
                val id = "param:$index"
                nodes += graphNode(id, "PARAMETER", parameter.name, anchor(file, owner, parameter, source))
                edges += edge(previous, id, "CFG_NORMAL"); previous = id
            }
            edges += edge(previous, bodyEntry, "CFG_NORMAL")
            val unsupported = PsiTreeUtil.collectElementsOfType(body ?: fn, KtExpression::class.java).filter {
                it is KtObjectLiteralExpression || it is KtCallableReferenceExpression
            }
            unsupported.forEach { boundaries += boundary("UNSUPPORTED_CONTROL_FLOW", it) }
            return buildJsonObject {
                put("schema", "local-cfg/0.1"); put("symbol", owner); put("file", file)
                putJsonArray("nodes") { nodes.sortedBy { it["id"]!!.jsonPrimitive.content }.forEach(::add) }
                putJsonArray("edges") { edges.distinctBy { it.toString() }.sortedBy { it.toString() }.forEach(::add) }
                putJsonArray("boundaries") { boundaries.sortedBy { it.toString() }.forEach(::add) }
            }
        }

        private fun buildElement(element: KtExpression, next: CfgNext, loop: CfgLoopContext?, exception: CfgNext): String = when (element) {
            is KtBlockExpression -> element.statements.asReversed().fold(next) { continuation, statement -> CfgNext(buildElement(statement, continuation, loop, exception)) }.id
            is KtIfExpression -> {
                val id = addNode(element, "BRANCH", null, element.condition?.let(::usedNames).orEmpty())
                val yes = element.then?.let { buildElement(it, next, loop, exception) } ?: next.id
                val no = element.`else`?.let { buildElement(it, next, loop, exception) } ?: next.id
                edges += edge(id, yes, "CFG_TRUE"); edges += edge(id, no, "CFG_FALSE"); id
            }
            is KtWhenExpression -> {
                val id = addNode(element, "BRANCH", null, element.subjectExpression?.let(::usedNames).orEmpty() + element.entries.flatMap { entry -> entry.conditions.flatMap(::usedNames) })
                if (element.entries.isEmpty()) edges += edge(id, next.id, "CFG_FALSE")
                element.entries.forEach { entry ->
                    val target = entry.expression?.let { buildElement(it, next, loop, exception) } ?: next.id
                    edges += edge(id, target, if (entry.isElse) "CFG_FALSE" else "CFG_TRUE")
                }; id
            }
            is KtWhileExpression -> buildWhile(element, next, exception)
            is KtDoWhileExpression -> buildDoWhile(element, next, exception)
            is KtForExpression -> buildFor(element, next, exception)
            is KtBreakExpression -> addNode(element, "BREAK", null, emptyList()).also { edges += edge(it, loop?.breakTarget?.id ?: exception.id, loop?.breakTarget?.edge ?: "CFG_EXCEPTION") }
            is KtContinueExpression -> addNode(element, "CONTINUE", null, emptyList()).also { edges += edge(it, loop?.continueTarget?.id ?: exception.id, loop?.continueTarget?.edge ?: "CFG_EXCEPTION") }
            is KtReturnExpression -> addNode(element, "RETURN", null, element.returnedExpression?.let(::usedNames).orEmpty()).also { edges += edge(it, "exit", "CFG_NORMAL"); edges += edge(it, "exit", "RETURN") }
            is KtThrowExpression -> addNode(element, "THROW", null, element.thrownExpression?.let(::usedNames).orEmpty()).also { edges += edge(it, exception.id, "CFG_EXCEPTION"); edges += edge(it, exception.id, "THROW") }
            is KtTryExpression -> buildTry(element, next, loop, exception)
            is KtBinaryExpression -> buildBinary(element, next, loop, exception)
            is KtSafeQualifiedExpression -> {
                val id = addNode(element, "BRANCH", null, usedNames(element.receiverExpression))
                val selector = element.selectorExpression?.let { buildElement(it, next, loop, exception) } ?: next.id
                edges += edge(id, selector, "CFG_TRUE"); edges += edge(id, next.id, "CFG_FALSE"); id
            }
            else -> addNode(element, kind(element), definedName(element), usedNamesForNode(element)).also { edges += edge(it, next.id, next.edge) }
        }

        private fun buildWhile(element: KtWhileExpression, next: CfgNext, exception: CfgNext): String {
            val condition = addNode(element, "LOOP", null, element.condition?.let(::usedNames).orEmpty())
            val context = CfgLoopContext(next, CfgNext(condition, "CFG_BACK"))
            val body = element.body?.let { buildElement(it, CfgNext(condition, "CFG_BACK"), context, exception) } ?: condition
            edges += edge(condition, body, "CFG_TRUE"); edges += edge(condition, next.id, "CFG_FALSE"); return condition
        }
        private fun buildDoWhile(element: KtDoWhileExpression, next: CfgNext, exception: CfgNext): String {
            val condition = addNode(element, "LOOP", null, element.condition?.let(::usedNames).orEmpty())
            val context = CfgLoopContext(next, CfgNext(condition, "CFG_NORMAL"))
            val body = element.body?.let { buildElement(it, CfgNext(condition), context, exception) } ?: condition
            edges += edge(condition, body, "CFG_BACK"); edges += edge(condition, next.id, "CFG_FALSE"); return body
        }
        private fun buildFor(element: KtForExpression, next: CfgNext, exception: CfgNext): String {
            val condition = addNode(element, "LOOP", element.loopParameter?.name, element.loopRange?.let(::usedNames).orEmpty())
            val context = CfgLoopContext(next, CfgNext(condition, "CFG_BACK"))
            val body = element.body?.let { buildElement(it, CfgNext(condition, "CFG_BACK"), context, exception) } ?: condition
            edges += edge(condition, body, "CFG_TRUE"); edges += edge(condition, next.id, "CFG_FALSE"); return condition
        }
        private fun buildTry(element: KtTryExpression, next: CfgNext, loop: CfgLoopContext?, exception: CfgNext): String {
            val finallyEntry = element.finallyBlock?.finalExpression?.let { buildElement(it, next, loop, exception) } ?: next.id
            val catches = element.catchClauses.map { clause ->
                val body = clause.catchBody?.let { buildElement(it, CfgNext(finallyEntry), loop, exception) } ?: finallyEntry
                clause.catchParameter?.let { parameter ->
                    addNode(parameter, "DEFINITION", parameter.name, emptyList()).also { edges += edge(it, body, "CFG_NORMAL") }
                } ?: body
            }
            val catchTarget = catches.firstOrNull()?.let { CfgNext(it, "CFG_EXCEPTION") } ?: exception
            val tryEntry = buildElement(element.tryBlock, CfgNext(finallyEntry), loop, catchTarget)
            val id = addNode(element, "TRY", null, emptyList())
            edges += edge(id, tryEntry, "CFG_NORMAL"); catches.forEach { edges += edge(id, it, "CFG_EXCEPTION") }; return id
        }
        private fun buildBinary(element: KtBinaryExpression, next: CfgNext, loop: CfgLoopContext?, exception: CfgNext): String {
            val op = element.operationReference.text
            if (op == "&&" || op == "||" || op == "?:") {
                val id = addNode(element, "BRANCH", null, element.left?.let(::usedNames).orEmpty())
                val rhs = element.right?.let { buildElement(it, next, loop, exception) } ?: next.id
                when (op) {
                    "&&" -> { edges += edge(id, rhs, "CFG_TRUE"); edges += edge(id, next.id, "CFG_FALSE") }
                    else -> { edges += edge(id, next.id, "CFG_TRUE"); edges += edge(id, rhs, "CFG_FALSE") }
                }; return id
            }
            return addNode(element, kind(element), definedName(element), usedNamesForNode(element)).also { edges += edge(it, next.id, next.edge) }
        }
        private fun needsOwnNode(e: KtExpression) = e is KtIfExpression || e is KtWhenExpression || e is KtLoopExpression || e is KtTryExpression || e is KtCallExpression || e is KtSafeQualifiedExpression || (e is KtBinaryExpression && e.operationReference.text in setOf("&&", "||", "?:"))
        private fun kind(e: PsiElement) = when (e) { is KtCallExpression -> "CALL"; is KtProperty -> "DEFINITION"; is KtBinaryExpression -> if (e.operationReference.text.contains("=")) "ASSIGNMENT" else "EXPRESSION"; else -> "EXPRESSION" }
        private fun usedNamesForNode(e: KtExpression): List<String> {
            val names = usedNames(e).toMutableList()
            if (e is KtProperty) e.name?.let(names::remove)
            if (e is KtBinaryExpression && e.operationReference.text == "=") (e.left as? KtNameReferenceExpression)?.getReferencedName()?.let(names::remove)
            return names.distinct()
        }
        private fun addNode(element: PsiElement, kind: String, defines: String?, uses: List<String>): String {
            val id = "n:${counter++}"; nodes += graphNode(id, kind, defines, anchor(file, owner, element, source), uses); return id
        }
        private fun boundary(kind: String, element: PsiElement) = buildJsonObject { put("kind", kind); put("syntaxKind", element::class.simpleName ?: "PsiElement"); putJsonArray("range") { add(element.textRange.startOffset); add(element.textRange.endOffset) } }
    }

    private fun applyEdit(request: JsonObject): JsonObject {
        val repo = Path.of(request.requiredString("repo")).toRealPath(); val relative = request.requiredString("file"); val path = repo.resolve(relative).normalize()
        val source = request["source"]?.jsonPrimitive?.content ?: path.readText()
        val compilation = request["compilation"]?.jsonPrimitive?.content ?: ":/main"
        val project = inspect(repo, compilation)
        val module = project["module"]?.jsonPrimitive?.content ?: ":"
        val sourceSet = project["sourceSet"]?.jsonPrimitive?.content ?: "main"
        val kind = request.requiredString("kind"); val replacement = request.requiredString("replacement")
        if (kind == "MAP_EDGE_WITH_CONTEXT") return applyMapEdgeWithContext(repo, relative, path, source, request, module, sourceSet, compilation)
        if (kind == "ADD_IMPORT" || kind == "REMOVE_IMPORT") return applyImportEdit(repo, relative, path, source, kind, replacement, request)
        if (kind == "REPLACE_DECLARATION") return applyDeclarationEdit(repo, relative, path, source, request, replacement, module, sourceSet)
        if (kind == "REWRITE_DECLARATION") return applyDeclarationRewrite(repo, relative, path, source, request, module, sourceSet)
        val kt = factory.createFile(path.fileName.toString(), source); val ownerQuery = request.requiredString("ownerSymbolId")
        val pkg = kt.packageFqName.asString(); val owner = PsiTreeUtil.collectElementsOfType(kt, KtNamedFunction::class.java).singleOrNull { symbolMatches(ownerQuery, pkg, it, module, sourceSet) }
            ?: throw WorkerFailure("STALE_TARGET", "owner no longer resolves uniquely")
        val expectedHash = request.requiredString("exactTextHash")
        val syntaxKind = request["syntaxKind"]?.jsonPrimitive?.content
        val tokenHash = request["normalizedTokenHash"]?.jsonPrimitive?.content
        var matches: List<PsiElement> = if (kind == "REPLACE_FUNCTION_BODY") listOfNotNull(owner.bodyExpression) else
            PsiTreeUtil.collectElementsOfType(owner, KtExpression::class.java).toList()
        if (syntaxKind != null) matches = matches.filter { it::class.simpleName == syntaxKind }
        if (tokenHash != null) matches = matches.filter { sha(normalizeTokens(it.text).toByteArray()) == tokenHash }
        // Exact text is a precondition, not the identity. Context/path fields are
        // only tie-breakers: a unique token target survives neighboring edits.
        matches = matches.filter { sha(it.text.toByteArray()) == expectedHash }
        if (matches.isEmpty()) throw WorkerFailure("STALE_TARGET", "target hash no longer exists")
        if (matches.size > 1) {
            val requested = listOf("ancestorPathHash", "localOrdinal", "leftContextHash", "rightContextHash")
            requested.forEach { field ->
                val expected = request[field] ?: return@forEach
                val narrowed = matches.filter { candidate -> anchor(relative, ownerQuery, candidate, source)[field] == expected }
                if (narrowed.isNotEmpty()) matches = narrowed
            }
        }
        if (matches.size > 1) throw WorkerFailure("AMBIGUOUS_TARGET", "target hash resolves to ${matches.size} nodes")
        val oldEffects = effects(matches.single())
        val replacementNode = try {
            if (kind == "REPLACE_FUNCTION_BODY") {
                if (replacement.trimStart().startsWith("{")) factory.createFunction("fun __semantic_thread_candidate__() $replacement").bodyExpression as? KtBlockExpression ?: throw IllegalArgumentException("function body must be a Kotlin block")
                else factory.createBlock(replacement)
            } else factory.createExpression(replacement)
        }
            catch (e: Throwable) { throw WorkerFailure("REPLACEMENT_PARSE_ERROR", e.message ?: "replacement parse failed") }
        val matchedRange = matches.single().textRange
        val editStart = if (kind == "REPLACE_FUNCTION_BODY" && !owner.hasBlockBody()) owner.equalsToken?.textRange?.startOffset ?: matchedRange.startOffset else matchedRange.startOffset
        val editEnd = matchedRange.endOffset
        val candidate = source.substring(0, editStart) + replacementNode.text + source.substring(editEnd)
        val candidateFile = factory.createFile(path.fileName.toString(), candidate)
        val errors = PsiTreeUtil.collectElementsOfType(candidateFile, PsiErrorElement::class.java).map { it.errorDescription }.sorted()
        if (errors.isNotEmpty()) throw WorkerFailure("REPLACEMENT_PARSE_ERROR", errors.joinToString("; "))
        val syntacticIntroducedEffects = effects(replacementNode) - oldEffects
        if (request["deferSemanticValidation"]?.jsonPrimitive?.booleanOrNull == true) {
            return buildJsonObject {
                put("schema", "semantic-candidate/0.1"); put("file", relative)
                put("originalHash", sha(source.toByteArray())); put("candidateHash", sha(candidate.toByteArray())); putCandidateSource(repo, candidate)
                putJsonArray("diagnostics") {}; putJsonArray("introducedEffects") { syntacticIntroducedEffects.sorted().forEach(::add) }
                // A task transaction may temporarily break bindings while its
                // declaration/file candidates are still being assembled.
                // The detached worktree validates the complete state once.
                put("k2Validated", false)
            }
        }
        val baselineOverrides = if (source == path.readText()) emptyMap() else mapOf(relative to source)
        val baseline = analyzeWithK2(repo, baselineOverrides, compilation)
        val candidateAnalysis = analyzeWithK2(repo, mapOf(relative to candidate), compilation)
        val baselineFacts = fileFacts(repo, path, baseline)
        val candidateFacts = fileFacts(repo, path, candidateAnalysis)
        val oldSemanticEffects = semanticEffects(baselineFacts, matchedRange.startOffset, matchedRange.endOffset)
        val newSemanticEffects = semanticEffects(candidateFacts, editStart, editStart + replacementNode.text.length)
        val introducedEffects = syntacticIntroducedEffects + (newSemanticEffects - oldSemanticEffects)
        val baselineErrors = baseline.diagnostics.filter(::isErrorDiagnostic).map(::diagnosticIdentity).toSet()
        val allowedDiagnostics = request["postconditions"]?.jsonObject?.get("allowedDiagnostics")?.jsonArray?.map { it.jsonPrimitive.content }.orEmpty()
        val newErrors = candidateAnalysis.diagnostics.filter(::isErrorDiagnostic).filter { diagnostic ->
            val identity = diagnosticIdentity(diagnostic)
            identity !in baselineErrors && allowedDiagnostics.none { it in identity }
        }
        if (!candidateAnalysis.valid || newErrors.isNotEmpty()) {
            throw WorkerFailure("NEW_DIAGNOSTICS", (newErrors.ifEmpty { candidateAnalysis.diagnostics }).joinToString("; ") { it["message"]?.jsonPrimitive?.content.orEmpty() })
        }
        val delta = replacementNode.text.length - (editEnd - editStart)
        val changedBinding = baselineFacts.firstOrNull { fact ->
            val start = fact["start"]?.jsonPrimitive?.intOrNull ?: return@firstOrNull false
            val end = fact["end"]?.jsonPrimitive?.intOrNull ?: return@firstOrNull false
            if (start < editEnd && end > editStart) return@firstOrNull false
            val expectedStart = if (start >= editEnd) start + delta else start
            val expectedEnd = if (end > editEnd) end + delta else end
            candidateFacts.none { candidateFact ->
                candidateFact["start"]?.jsonPrimitive?.intOrNull == expectedStart && candidateFact["end"]?.jsonPrimitive?.intOrNull == expectedEnd && semanticSignature(candidateFact) == semanticSignature(fact)
            }
        }
        if (changedBinding != null) throw WorkerFailure("BINDING_CHANGED", "protected K2 FIR fact changed: ${semanticSignature(changedBinding)}")
        val preconditions = request["preconditions"]?.jsonObject ?: buildJsonObject {}
        val postconditions = request["postconditions"]?.jsonObject ?: buildJsonObject {}
        val protectedFacts = baselineFacts.filter {
            val start = it["start"]?.jsonPrimitive?.intOrNull ?: -1; val end = it["end"]?.jsonPrimitive?.intOrNull ?: -1
            start >= owner.textRange.startOffset && end <= owner.textRange.endOffset && !(start < editEnd && end > editStart)
        }
        preconditions["scopeBindingsHash"]?.jsonPrimitive?.content?.let { expected ->
            val actual = sha(JsonArray(protectedFacts.map(::semanticSignature)).toString().toByteArray())
            if (actual != expected) throw WorkerFailure("BINDING_CHANGED", "scopeBindingsHash precondition failed")
        }
        val originalType = expressionType(baselineFacts, matchedRange.startOffset, matchedRange.endOffset)
        val replacementEnd = editStart + replacementNode.text.length
        val replacementType = expressionType(candidateFacts, editStart, replacementEnd)
        preconditions["expectedType"]?.jsonPrimitive?.content?.let { expected ->
            if (!sameType(originalType, expected)) throw WorkerFailure("TYPE_MISMATCH", "expected target type $expected, resolved ${originalType ?: "<unknown>"}")
        }
        postconditions["typeAssignableTo"]?.jsonPrimitive?.content?.let { expected ->
            if (kind == "REPLACE_EXPRESSION") {
                val probe = "run { val __semantic_thread_type_probe: $expected = $replacement; $replacement }"
                val probeSource = source.substring(0, matchedRange.startOffset) + probe + source.substring(matchedRange.endOffset)
                val probeAnalysis = analyzeWithK2(repo, mapOf(relative to probeSource), compilation)
                if (!probeAnalysis.valid || probeAnalysis.diagnostics.any(::isErrorDiagnostic)) throw WorkerFailure("TYPE_MISMATCH", "replacement type ${replacementType ?: "<unknown>"} is not assignable to $expected: ${probeAnalysis.diagnostics.filter(::isErrorDiagnostic).joinToString("; ") { it["message"]?.jsonPrimitive?.content.orEmpty() }}")
            } else if (!sameType(replacementType, expected)) throw WorkerFailure("TYPE_MISMATCH", "replacement type ${replacementType ?: "<unknown>"} is not assignable to $expected")
        }
        if (postconditions["preserveEffects"]?.jsonPrimitive?.booleanOrNull == true && introducedEffects.isNotEmpty()) throw WorkerFailure("EFFECT_CHANGED", "replacement changes effects: ${introducedEffects.sorted()}")
        val candidateOwner = PsiTreeUtil.collectElementsOfType(candidateFile, KtNamedFunction::class.java).singleOrNull { symbolMatches(ownerQuery, pkg, it, module, sourceSet) }
            ?: throw WorkerFailure("BINDING_CHANGED", "owner declaration changed or disappeared")
        fun signature(function: KtNamedFunction) = normalizeTokens(sourceSignature(function))
        fun summary(function: KtNamedFunction, facts: List<JsonObject>) = sha(buildJsonObject {
            put("bodyHash", sha(normalizeTokens(function.bodyExpression?.text.orEmpty()).toByteArray()))
            putJsonArray("facts") { facts.filter { fact ->
                val start = fact["start"]?.jsonPrimitive?.intOrNull ?: -1; val end = fact["end"]?.jsonPrimitive?.intOrNull ?: -1
                start >= function.textRange.startOffset && end <= function.textRange.endOffset
            }.map(::semanticSignature).forEach(::add) }
        }.toString().toByteArray())
        val beforeSignature = sha(signature(owner).toByteArray()); val afterSignature = sha(signature(candidateOwner).toByteArray())
        val beforeBody = sha(normalizeTokens(owner.bodyExpression?.text.orEmpty()).toByteArray()); val afterBody = sha(normalizeTokens(candidateOwner.bodyExpression?.text.orEmpty()).toByteArray())
        val beforeEffects = (effects(owner.bodyExpression ?: owner) + semanticEffects(baselineFacts, owner.textRange.startOffset, owner.textRange.endOffset)).sorted()
        val afterEffects = (effects(candidateOwner.bodyExpression ?: candidateOwner) + semanticEffects(candidateFacts, candidateOwner.textRange.startOffset, candidateOwner.textRange.endOffset)).sorted()
        return buildJsonObject {
            put("schema", "semantic-candidate/0.1"); put("file", relative); put("originalHash", sha(source.toByteArray())); put("candidateHash", sha(candidate.toByteArray())); putCandidateSource(repo, candidate)
            putJsonArray("diagnostics") { candidateAnalysis.diagnostics.forEach(::add) }; putJsonArray("introducedEffects") { introducedEffects.sorted().forEach(::add) }
            originalType?.let { put("originalType", it) }; replacementType?.let { put("replacementType", it) }
            put("protectedBindingsHash", sha(JsonArray(protectedFacts.map(::semanticSignature)).toString().toByteArray())); put("k2Validated", candidateAnalysis.valid)
            putJsonObject("semanticDelta") {
                putJsonObject("body") { put("key", ownerQuery); put("beforeHash", beforeBody); put("afterHash", afterBody) }
                putJsonObject("signature") { put("key", ownerQuery); put("beforeHash", beforeSignature); put("afterHash", afterSignature) }
                putJsonObject("abi") { put("key", ownerQuery); put("beforeHash", sha("$ownerQuery|$beforeSignature".toByteArray())); put("afterHash", sha("$ownerQuery|$afterSignature".toByteArray())) }
                putJsonObject("summary") { put("key", ownerQuery); put("beforeHash", summary(owner, baselineFacts)); put("afterHash", summary(candidateOwner, candidateFacts)) }
                putJsonObject("effects") { put("key", ownerQuery); put("beforeHash", sha(JsonArray(beforeEffects.map(::JsonPrimitive)).toString().toByteArray())); put("afterHash", sha(JsonArray(afterEffects.map(::JsonPrimitive)).toString().toByteArray())) }
                putJsonObject("diagnostics") { put("key", relative); put("beforeHash", sha(JsonArray(baseline.diagnostics).toString().toByteArray())); put("afterHash", sha(JsonArray(candidateAnalysis.diagnostics).toString().toByteArray())) }
            }
        }
    }

    private fun applyMapEdgeWithContext(
        repo: Path,
        relative: String,
        path: Path,
        source: String,
        request: JsonObject,
        module: String,
        sourceSet: String,
        compilation: String,
    ): JsonObject {
        if (request.requiredString("replacement").isNotEmpty()) {
            throw WorkerFailure("INVALID_INPUT", "MAP_EDGE_WITH_CONTEXT forbids Kotlin replacement text")
        }
        val operation = request["semanticOperation"]?.jsonObject
            ?: throw WorkerFailure("INVALID_INPUT", "MAP_EDGE_WITH_CONTEXT requires typed semanticOperation")
        if (operation.requiredString("kind") != "MAP_EDGE_WITH_CONTEXT") {
            throw WorkerFailure("INVALID_INPUT", "semantic operation kind does not match edit kind")
        }
        val workflowSymbol = operation.requiredString("workflowSymbol")
        val contextSymbol = operation.requiredString("contextProducerSymbol")
        val transformerSymbol = operation.requiredString("transformerSymbol")
        val parameterIndex = operation.requiredInt("valueParameterIndex")
        val collectionType = operation.requiredString("collectionType")
        val elementType = operation.requiredString("elementType")
        val contextType = operation.requiredString("contextType")
        val placement = operation.requiredString("placement")
        val strategy = operation.requiredString("strategy")
        if (placement != "$workflowSymbol#FUNCTION_ENTRY" || strategy != "KOTLIN_EAGER_LIST_MAP_WITH_CONTEXT_ONCE") {
            throw WorkerFailure("PRECONDITION_FAILED", "unsupported map-edge placement or strategy")
        }
        val compilerFqName = { symbol: String ->
            if (!symbol.matches(Regex("[A-Za-z_][A-Za-z0-9_]*(?:/[A-Za-z_][A-Za-z0-9_]*)+"))) {
                throw WorkerFailure("INVALID_INPUT", "compiler symbol is not a callable FQN: $symbol")
            }
            symbol.replace('/', '.')
        }
        val contextFqName = compilerFqName(contextSymbol)
        val transformerFqName = compilerFqName(transformerSymbol)
        val kt = factory.createFile(path.fileName.toString(), source)
        val ownerQuery = request.requiredString("ownerSymbolId")
        val pkg = kt.packageFqName.asString()
        val owner = PsiTreeUtil.collectElementsOfType(kt, KtNamedFunction::class.java)
            .singleOrNull { symbolMatches(ownerQuery, pkg, it, module, sourceSet) }
            ?: throw WorkerFailure("STALE_TARGET", "workflow owner no longer resolves uniquely")
        if (owner.parent !is KtFile || owner.receiverTypeReference != null || owner.hasModifier(KtTokens.SUSPEND_KEYWORD)) {
            throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "workflow must be a top-level non-receiver non-suspend function")
        }
        if ("$pkg.${owner.name}" != compilerFqName(workflowSymbol)) {
            throw WorkerFailure("BINDING_CHANGED", "workflow compiler symbol no longer matches owner")
        }
        val parameter = owner.valueParameters.getOrNull(parameterIndex)
            ?: throw WorkerFailure("BINDING_CHANGED", "bound value parameter index is absent")
        val parameterName = parameter.name
            ?: throw WorkerFailure("BINDING_CHANGED", "bound value parameter has no name")
        val resolution = resolveSymbol(repo, ownerQuery, compilation)
        val identity = resolution["declaration"]?.jsonObject?.get("symbolIdentity")?.jsonObject
            ?: throw WorkerFailure("INCOMPLETE_SEMANTIC_ANALYSIS", "workflow has no compiler identity")
        if (identity["containingDeclarations"]?.jsonArray?.isNotEmpty() != false ||
            identity["receiverTypes"]?.jsonArray?.isNotEmpty() != false ||
            identity["contextReceiverTypes"]?.jsonArray?.isNotEmpty() != false
        ) {
            throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "workflow compiler identity has an unsupported receiver")
        }
        val parameterTypes = identity["parameterTypes"]?.jsonArray?.map { it.jsonPrimitive.content }.orEmpty()
        if (parameterTypes.getOrNull(parameterIndex) != collectionType || identity["returnType"]?.jsonPrimitive?.content != collectionType) {
            throw WorkerFailure("TYPE_MISMATCH", "workflow collection input/return type changed")
        }
        if (collectionType != "kotlin/collections/List<$elementType>" || '?' in collectionType || '?' in elementType || '?' in contextType) {
            throw WorkerFailure("TYPE_MISMATCH", "operation requires an exact non-null eager List<T> contour")
        }
        val directValue = when (val body = owner.bodyExpression) {
            is KtNameReferenceExpression -> body.getReferencedName() == parameterName
            is KtBlockExpression -> {
                val statement = body.statements.singleOrNull() as? KtReturnExpression
                (statement?.returnedExpression as? KtNameReferenceExpression)?.getReferencedName() == parameterName
            }
            else -> false
        }
        if (!directValue) {
            throw WorkerFailure("INCOMPLETE_SEMANTIC_ANALYSIS", "workflow is no longer one direct parameter-to-return edge")
        }
        val occupied = PsiTreeUtil.collectElementsOfType(owner, KtNamedDeclaration::class.java)
            .mapNotNull { it.name }.toMutableSet()
        fun fresh(base: String): String {
            var candidate = base
            var suffix = 0
            while (!occupied.add(candidate)) candidate = "$base${++suffix}"
            return candidate
        }
        val contextName = fresh("__codeclewContext")
        val valueName = fresh("__codeclewValue")
        if (owner.annotationEntries.isNotEmpty() || owner.modifierList?.text?.isNotBlank() == true || owner.typeParameterList != null || owner.typeConstraintList != null) {
            throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "current PSI materializer supports a plain top-level workflow declaration")
        }
        val valueParameterList = owner.valueParameterList
            ?: throw WorkerFailure("INCOMPLETE_SEMANTIC_ANALYSIS", "workflow has no PSI value-parameter list")
        val returnType = owner.typeReference
            ?: throw WorkerFailure("INCOMPLETE_SEMANTIC_ANALYSIS", "workflow must declare its return type")
        val generatedOwner = factory.createFunction(
            "fun ${owner.name}${valueParameterList.text}: ${returnType.text} {\n" +
                "    val $contextName = $contextFqName()\n" +
                "    return $parameterName.map { $valueName -> $transformerFqName($valueName, $contextName) }\n" +
                "}"
        )
        val candidate = source.substring(0, owner.textRange.startOffset) + generatedOwner.text + source.substring(owner.textRange.endOffset)
        val candidateFile = factory.createFile(path.fileName.toString(), candidate)
        val candidateErrors = PsiTreeUtil.collectElementsOfType(candidateFile, PsiErrorElement::class.java)
            .map { it.errorDescription }.sorted()
        if (candidateErrors.isNotEmpty()) {
            throw WorkerFailure("REPLACEMENT_PARSE_ERROR", candidateErrors.joinToString("; "))
        }
        val baseline = analyzeWithK2(repo, emptyMap(), compilation)
        val candidateAnalysis = analyzeWithK2(repo, mapOf(relative to candidate), compilation)
        val baselineErrors = baseline.diagnostics.filter(::isErrorDiagnostic).map(::diagnosticIdentity).toSet()
        val newErrors = candidateAnalysis.diagnostics.filter(::isErrorDiagnostic)
            .filter { diagnosticIdentity(it) !in baselineErrors }
        if (!candidateAnalysis.valid || newErrors.isNotEmpty()) {
            throw WorkerFailure("NEW_DIAGNOSTICS", newErrors.ifEmpty { candidateAnalysis.diagnostics }.joinToString("; ") { it["message"]?.jsonPrimitive?.content.orEmpty() })
        }
        val baselineFacts = fileFacts(repo, path, baseline)
        val candidateFacts = fileFacts(repo, path, candidateAnalysis)
        val resolvedCalls = candidateFacts.filter {
            "FunctionCall" in it["kind"]?.jsonPrimitive?.content.orEmpty()
        }
        fun callCount(symbol: String) = resolvedCalls.count { it["symbol"]?.jsonPrimitive?.content == symbol }
        val contextCalls = callCount(contextSymbol)
        val transformerCalls = callCount(transformerSymbol)
        val mapCalls = resolvedCalls.count {
            it["symbol"]?.jsonPrimitive?.content?.let { symbol -> symbol == "kotlin/collections/map" || symbol.endsWith("/map") } == true
        }
        if (contextCalls != 1 || transformerCalls != 1 || mapCalls != 1) {
            throw WorkerFailure(
                "BINDING_CHANGED",
                "candidate must resolve one exact context call, transformer call, and eager List.map; found $contextCalls/$transformerCalls/$mapCalls",
            )
        }
        val finalOwner = PsiTreeUtil.collectElementsOfType(candidateFile, KtNamedFunction::class.java)
            .single { symbolMatches(ownerQuery, pkg, it, module, sourceSet) }
        val beforeSignature = sha(normalizeTokens(sourceSignature(owner)).toByteArray())
        val afterSignature = sha(normalizeTokens(sourceSignature(finalOwner)).toByteArray())
        if (beforeSignature != afterSignature) {
            throw WorkerFailure("ABI_CHANGED", "semantic map-edge operation changed workflow signature")
        }
        val beforeBody = sha(normalizeTokens(owner.bodyExpression?.text.orEmpty()).toByteArray())
        val afterBody = sha(normalizeTokens(finalOwner.bodyExpression?.text.orEmpty()).toByteArray())
        return buildJsonObject {
            put("schema", "semantic-candidate/0.2"); put("file", relative)
            put("originalHash", sha(source.toByteArray())); put("candidateHash", sha(candidate.toByteArray())); putCandidateSource(repo, candidate)
            putJsonArray("diagnostics") { candidateAnalysis.diagnostics.forEach(::add) }
            putJsonArray("introducedEffects") {}
            put("k2Validated", true)
            putJsonObject("semanticOperationProof") {
                put("kind", "MAP_EDGE_WITH_CONTEXT")
                put("workflowSymbol", workflowSymbol); put("contextProducerSymbol", contextSymbol); put("transformerSymbol", transformerSymbol)
                put("typeAssignable", true); put("contextEvaluatedOnce", true); put("placementDominatesUses", true)
                put("orderPreserved", true); put("cardinalityPreserved", true); put("lazinessPreserved", true)
                put("effectsPreserved", true); put("nullabilityPreserved", true); put("consumerContractPreserved", true)
                put("abiPreserved", true); put("behavioralOracleRequired", true); put("noUnsupportedBoundary", true)
            }
            putJsonObject("semanticDelta") {
                putJsonObject("body") { put("key", ownerQuery); put("beforeHash", beforeBody); put("afterHash", afterBody) }
                putJsonObject("signature") { put("key", ownerQuery); put("beforeHash", beforeSignature); put("afterHash", afterSignature) }
                putJsonObject("abi") { put("key", ownerQuery); put("beforeHash", sha("$ownerQuery|$beforeSignature".toByteArray())); put("afterHash", sha("$ownerQuery|$afterSignature".toByteArray())) }
                putJsonObject("summary") { put("key", ownerQuery); put("beforeHash", sha(JsonArray(baselineFacts.map(::semanticSignature)).toString().toByteArray())); put("afterHash", sha(JsonArray(candidateFacts.map(::semanticSignature)).toString().toByteArray())) }
                putJsonObject("effects") { put("key", ownerQuery); put("beforeHash", sha("[]".toByteArray())); put("afterHash", sha("[]".toByteArray())) }
                putJsonObject("diagnostics") { put("key", relative); put("beforeHash", sha(JsonArray(baseline.diagnostics).toString().toByteArray())); put("afterHash", sha(JsonArray(candidateAnalysis.diagnostics).toString().toByteArray())) }
            }
        }
    }

    private fun applyDeclarationEdit(repo: Path, relative: String, path: Path, source: String, request: JsonObject, replacement: String, module: String, sourceSet: String): JsonObject {
        val kt = factory.createFile(path.fileName.toString(), source)
        val ownerQuery = request.requiredString("ownerSymbolId")
        val pkg = kt.packageFqName.asString()
        val owner = PsiTreeUtil.collectElementsOfType(kt, KtNamedDeclaration::class.java)
            .singleOrNull { symbolMatches(ownerQuery, pkg, it, module, sourceSet) }
            ?: throw WorkerFailure("STALE_TARGET", "declaration owner no longer resolves uniquely")
        val expectedHash = request.requiredString("exactTextHash")
        if (sha(owner.text.toByteArray()) != expectedHash) throw WorkerFailure("STALE_TARGET", "declaration hash no longer matches")
        request["syntaxKind"]?.jsonPrimitive?.content?.takeIf(String::isNotBlank)?.let { expected ->
            if (owner::class.simpleName != expected) throw WorkerFailure("STALE_TARGET", "declaration syntax kind no longer matches")
        }
        val replacementFile = factory.createFile(path.fileName.toString(), replacement)
        val replacementErrors = PsiTreeUtil.collectElementsOfType(replacementFile, PsiErrorElement::class.java)
            .map { it.errorDescription }.sorted()
        val declarations = replacementFile.declarations
        if (replacementErrors.isNotEmpty() || declarations.size != 1 || declarations.single() !is KtNamedDeclaration || replacementFile.importDirectives.isNotEmpty()) {
            throw WorkerFailure("REPLACEMENT_PARSE_ERROR", "replacement must contain exactly one Kotlin declaration")
        }
        val replacementDeclaration = declarations.single()
        val candidate = source.substring(0, owner.textRange.startOffset) + replacementDeclaration.text + source.substring(owner.textRange.endOffset)
        val candidateFile = factory.createFile(path.fileName.toString(), candidate)
        val errors = PsiTreeUtil.collectElementsOfType(candidateFile, PsiErrorElement::class.java).map { it.errorDescription }.sorted()
        if (errors.isNotEmpty()) throw WorkerFailure("REPLACEMENT_PARSE_ERROR", errors.joinToString("; "))
        return buildJsonObject {
            put("schema", "semantic-candidate/0.1"); put("file", relative)
            put("originalHash", sha(source.toByteArray())); put("candidateHash", sha(candidate.toByteArray())); putCandidateSource(repo, candidate)
            putJsonArray("diagnostics") {}; putJsonArray("introducedEffects") {}
            // Declaration changes may make callers temporarily inconsistent.
            // The detached transaction worktree performs the authoritative K2/build validation after every candidate is assembled.
            put("k2Validated", false)
        }
    }

    private fun applyDeclarationRewrite(repo: Path, relative: String, path: Path, source: String, request: JsonObject, module: String, sourceSet: String): JsonObject {
        val kt = factory.createFile(path.fileName.toString(), source)
        val ownerQuery = request.requiredString("ownerSymbolId")
        val pkg = kt.packageFqName.asString()
        val owner = PsiTreeUtil.collectElementsOfType(kt, KtNamedDeclaration::class.java)
            .singleOrNull { symbolMatches(ownerQuery, pkg, it, module, sourceSet) }
            ?: throw WorkerFailure("STALE_TARGET", "declaration owner no longer resolves uniquely")
        if (sha(owner.text.toByteArray()) != request.requiredString("exactTextHash")) {
            throw WorkerFailure("STALE_TARGET", "declaration hash no longer matches")
        }
        request["syntaxKind"]?.jsonPrimitive?.content?.takeIf(String::isNotBlank)?.let { expected ->
            if (owner::class.simpleName != expected) throw WorkerFailure("STALE_TARGET", "declaration syntax kind no longer matches")
        }
        val substitutions = request["preconditions"]?.jsonObject?.get("substitutions")?.jsonArray
            ?: throw WorkerFailure("INVALID_INPUT", "REWRITE_DECLARATION requires preconditions.substitutions")
        if (substitutions.isEmpty()) throw WorkerFailure("INVALID_INPUT", "REWRITE_DECLARATION substitutions are empty")
        var rewritten = owner.text
        substitutions.forEachIndexed { index, item ->
            val old = item.jsonObject["old"]?.jsonPrimitive?.content
                ?: throw WorkerFailure("INVALID_INPUT", "substitution $index has no old text")
            val new = item.jsonObject["new"]?.jsonPrimitive?.content
                ?: throw WorkerFailure("INVALID_INPUT", "substitution $index has no new text")
            if (old.isEmpty()) throw WorkerFailure("INVALID_INPUT", "substitution $index old text is empty")
            val occurrence = item.jsonObject["occurrence"]?.jsonPrimitive?.intOrNull
            val expectedOccurrences = item.jsonObject["occurrences"]?.jsonPrimitive?.intOrNull
                ?: if (occurrence == null) 1 else null
            val lineMode = item.jsonObject["lineMode"]?.jsonPrimitive?.booleanOrNull == true
            if (occurrence != null && occurrence < 1) throw WorkerFailure("INVALID_INPUT", "substitution $index occurrence must be positive")
            if (expectedOccurrences != null && expectedOccurrences < 1) throw WorkerFailure("INVALID_INPUT", "substitution $index occurrences must be positive")
            val matcher = if (lineMode) {
                val lines = old.lines()
                Regex(
                    "(?m)^([\\t ]*)" + lines.first().let(Regex::escape) + "[\\t ]*$" +
                        lines.drop(1).joinToString("") { line ->
                            "\\r?\\n^[\\t ]*${Regex.escape(line)}[\\t ]*$"
                        }
                )
            } else {
                Regex(Regex.escape(old))
            }
            val matches = matcher.findAll(rewritten).toList()
            if (expectedOccurrences != null && matches.size != expectedOccurrences) {
                throw WorkerFailure("PRECONDITION_FAILED", "substitution $index expected $expectedOccurrences exact matches, found ${matches.size}")
            }
            val selectedMatches = if (occurrence != null) {
                listOf(matches.getOrNull(occurrence - 1)
                    ?: throw WorkerFailure("PRECONDITION_FAILED", "substitution $index occurrence $occurrence is absent; found ${matches.size} exact matches")
                )
            } else {
                matches
            }
            selectedMatches.asReversed().forEach { match ->
                val replacement = if (lineMode) {
                    val indent = match.groupValues[1]
                    new.lines().joinToString("\n") { line ->
                        if (line.isEmpty()) "" else indent + line
                    }
                } else {
                    new
                }
                rewritten = rewritten.replaceRange(match.range, replacement)
            }
        }
        val replacementFile = factory.createFile(path.fileName.toString(), rewritten)
        val replacementErrors = PsiTreeUtil.collectElementsOfType(replacementFile, PsiErrorElement::class.java)
            .map { it.errorDescription }.sorted()
        if (replacementErrors.isNotEmpty() || replacementFile.declarations.size != 1 || replacementFile.declarations.single() !is KtNamedDeclaration || replacementFile.importDirectives.isNotEmpty()) {
            throw WorkerFailure("REPLACEMENT_PARSE_ERROR", "rewritten result must contain exactly one Kotlin declaration")
        }
        val replacementDeclaration = replacementFile.declarations.single()
        val candidate = source.substring(0, owner.textRange.startOffset) + replacementDeclaration.text + source.substring(owner.textRange.endOffset)
        val candidateFile = factory.createFile(path.fileName.toString(), candidate)
        val errors = PsiTreeUtil.collectElementsOfType(candidateFile, PsiErrorElement::class.java).map { it.errorDescription }.sorted()
        if (errors.isNotEmpty()) throw WorkerFailure("REPLACEMENT_PARSE_ERROR", errors.joinToString("; "))
        return buildJsonObject {
            put("schema", "semantic-candidate/0.1"); put("file", relative)
            put("originalHash", sha(source.toByteArray())); put("candidateHash", sha(candidate.toByteArray())); putCandidateSource(repo, candidate)
            putJsonArray("diagnostics") {}; putJsonArray("introducedEffects") {}; put("k2Validated", false)
        }
    }

    private fun applyImportEdit(repo: Path, relative: String, path: Path, source: String, kind: String, replacement: String, request: JsonObject): JsonObject {
        val fqName = replacement.trim().removePrefix("import ").trim()
        if (!fqName.matches(Regex("[A-Za-z_][A-Za-z0-9_.]*(?:\\.\\*)?"))) throw WorkerFailure("REPLACEMENT_PARSE_ERROR", "invalid import: $fqName")
        val lineEnding = if ("\r\n" in source) "\r\n" else "\n"
        val line = "import $fqName"
        val existing = source.lineSequence().any { it.trim() == line }
        val candidate = when {
            kind == "ADD_IMPORT" && existing -> source
            kind == "REMOVE_IMPORT" && !existing -> source
            kind == "ADD_IMPORT" -> {
                val kt = factory.createFile(path.fileName.toString(), source)
                val imports = kt.importDirectives
                if (imports.isEmpty()) {
                    val insertion = kt.packageDirective?.textRange?.endOffset ?: 0
                    source.substring(0, insertion) + lineEnding + lineEnding + line + source.substring(insertion)
                } else {
                    val names = (imports.mapNotNull { it.importPath?.pathStr } + fqName).distinct().sorted()
                    val start = imports.first().textRange.startOffset; val end = imports.last().textRange.endOffset
                    source.substring(0, start) + names.joinToString(lineEnding) { "import $it" } + source.substring(end)
                }
            }
            else -> {
                val pattern = Regex("(?m)^import\\s+${Regex.escape(fqName)}\\s*(?:\\r?\\n)?")
                source.replaceFirst(pattern, "")
            }
        }
        if (request["deferSemanticValidation"]?.jsonPrimitive?.booleanOrNull == true) {
            return buildJsonObject {
                put("schema", "semantic-candidate/0.1"); put("file", relative)
                put("originalHash", sha(source.toByteArray())); put("candidateHash", sha(candidate.toByteArray())); putCandidateSource(repo, candidate)
                putJsonArray("diagnostics") {}; putJsonArray("introducedEffects") {}; put("k2Validated", false)
            }
        }
        val baselineOverrides = if (source == path.readText()) emptyMap() else mapOf(relative to source)
        val baseline = analyzeWithK2(repo, baselineOverrides); val candidateAnalysis = analyzeWithK2(repo, mapOf(relative to candidate))
        if (!candidateAnalysis.valid || candidateAnalysis.diagnostics.any(::isErrorDiagnostic)) throw WorkerFailure("NEW_DIAGNOSTICS", candidateAnalysis.diagnostics.joinToString("; ") { it["message"]?.jsonPrimitive?.content.orEmpty() })
        val before = fileFacts(repo, path, baseline).map(::semanticSignature).groupingBy { it.toString() }.eachCount()
        val after = fileFacts(repo, path, candidateAnalysis).map(::semanticSignature).groupingBy { it.toString() }.eachCount()
        if (before != after) throw WorkerFailure("BINDING_CHANGED", "import operation changes protected K2 bindings")
        return buildJsonObject {
            put("schema", "semantic-candidate/0.1"); put("file", relative); put("originalHash", sha(source.toByteArray())); put("candidateHash", sha(candidate.toByteArray())); putCandidateSource(repo, candidate)
            putJsonArray("diagnostics") { candidateAnalysis.diagnostics.forEach(::add) }; putJsonArray("introducedEffects") {}; put("k2Validated", candidateAnalysis.valid)
        }
    }

    private fun isErrorDiagnostic(diagnostic: JsonObject) = diagnostic["severity"]?.jsonPrimitive?.content == "ERROR"
    private fun normalizedDiagnostics(diagnostics: List<JsonObject>): List<JsonObject> = diagnostics.map { diagnostic ->
        buildJsonObject {
            put("severity", diagnostic["severity"] ?: JsonPrimitive("INFO"))
            put("message", diagnostic["message"]?.jsonPrimitive?.content.orEmpty().replace(Regex("^(?:[^:]+/)*[^:]+\\.kt:\\d+:\\d+:\\s*"), ""))
        }
    }.sortedBy { it.toString() }
    private fun JsonObjectBuilder.putCandidateSource(repo: Path, source: String) {
        val bytes = source.toByteArray()
        if (bytes.size <= 64 * 1024 || stateFor(repo).mode == "EXTERNAL") {
            put("source", source)
            return
        }
        val hash = sha(bytes); val relative = ".semantic-thread/blobs/sha256/${hash.removePrefix("sha256:")}"
        val path = repo.resolve(relative)
        if (!path.isRegularFile()) writeCacheAtomically(path, source)
        putJsonObject("sourceBlob") { put("contentHash", hash); put("relativePath", relative); put("sizeBytes", bytes.size) }
    }
    private fun diagnosticIdentity(diagnostic: JsonObject) = diagnostic["message"]?.jsonPrimitive?.content.orEmpty().replace(Regex("/[^ :]+/semantic-thread-k2[^ :]+"), "<candidate>")
    private fun semanticSignature(fact: JsonObject) = buildJsonObject {
        listOf("kind", "type", "symbol", "returnType", "receiverType", "effects").forEach { key -> fact[key]?.let { put(key, it) } }
        fact["argumentToParameter"]?.jsonArray?.let { mapping ->
            putJsonArray("argumentToParameter") {
                mapping.forEach { item -> add(buildJsonObject { item.jsonObject["parameter"]?.let { put("parameter", it) }; item.jsonObject["parameterType"]?.let { put("parameterType", it) } }) }
            }
        }
    }
    private fun semanticEffects(facts: List<JsonObject>, start: Int, end: Int): Set<String> = facts.filter {
        val fs = it["start"]?.jsonPrimitive?.intOrNull ?: -1; val fe = it["end"]?.jsonPrimitive?.intOrNull ?: -1
        fs >= start && fe <= end
    }.flatMap { it["effects"]?.jsonArray?.map { effect -> effect.jsonPrimitive.content }.orEmpty() }.toSet()
    private fun expressionType(facts: List<JsonObject>, start: Int, end: Int): String? = facts.filter {
        val fs = it["start"]?.jsonPrimitive?.intOrNull ?: -1; val fe = it["end"]?.jsonPrimitive?.intOrNull ?: -1
        fs >= start && fe <= end && it["type"]?.jsonPrimitive?.content?.let { type -> type != "kotlin/Nothing" } == true
    }.minByOrNull { (it["end"]!!.jsonPrimitive.int - it["start"]!!.jsonPrimitive.int) }?.get("type")?.jsonPrimitive?.content
    private fun sameType(actual: String?, expected: String): Boolean {
        if (actual == null) return false
        fun normalize(value: String) = value.removePrefix("kotlin/").replace('/', '.')
        return normalize(actual) == normalize(expected) || normalize(actual).substringAfterLast('.') == normalize(expected).substringAfterLast('.')
    }

    private fun validateCandidate(request: JsonObject): JsonObject {
        val source = request.requiredString("source")
        val parseStarted = System.nanoTime()
        val kt = factory.createFile(request["file"]?.jsonPrimitive?.content ?: "Candidate.kt", source)
        val errors = PsiTreeUtil.collectElementsOfType(kt, PsiErrorElement::class.java).map { it.errorDescription }.sorted()
        val psiParseMicros = (System.nanoTime() - parseStarted) / 1_000
        requestPsiParseMicros += psiParseMicros
        return buildJsonObject { put("valid", errors.isEmpty()); put("psiParseMicros", psiParseMicros); putJsonArray("diagnostics") { errors.forEach(::add) } }
    }

    private fun effects(element: PsiElement): Set<String> {
        val result = mutableSetOf<String>()
        if (element is KtThrowExpression || PsiTreeUtil.collectElementsOfType(element, KtThrowExpression::class.java).isNotEmpty()) result += "THROW"
        PsiTreeUtil.collectElementsOfType(element, KtCallExpression::class.java).forEach { call ->
            val name = call.calleeExpression?.text.orEmpty()
            if (name in setOf("print", "println", "readLine")) result += "IO" else result += "READ_STATE"
        }
        PsiTreeUtil.collectElementsOfType(element, KtBinaryExpression::class.java).forEach { binary ->
            if (binary.operationReference.text.contains("=") && (binary.left is KtDotQualifiedExpression || binary.left?.text?.startsWith("this.") == true)) result += "WRITE_STATE"
        }
        return result
    }

    override fun close() {
        Disposer.dispose(disposable)
        externalBuildStateRuntimeRoots.forEach { it.toFile().deleteRecursively() }
        externalBuildStateRuntimeRoots.clear()
    }
}

internal class WorkerFailure(val code: String, override val message: String) : RuntimeException(message)
private data class CfgNext(val id: String, val edge: String = "CFG_NORMAL")
private data class CfgLoopContext(val breakTarget: CfgNext, val continueTarget: CfgNext)
private data class K2Analysis(val valid: Boolean, val facts: List<JsonObject>, val diagnostics: List<JsonObject>)
private fun JsonObject.requiredString(name: String) = this[name]?.jsonPrimitive?.content ?: error("missing field $name")
private fun JsonObject.requiredInt(name: String) = this[name]?.jsonPrimitive?.int ?: error("missing field $name")
internal fun semanticK2CacheKey(extractorAuthority: JsonObject, semanticInput: String): String = sha(
    buildString {
        append("extractorAuthority=")
        append(canonicalJson(extractorAuthority))
        append('\u0000')
        append(semanticInput)
    }.toByteArray()
)
internal fun cacheMatchesExtractorAuthority(cached: JsonObject, expected: JsonObject): Boolean =
    cached["schema"]?.jsonPrimitive?.content == SEMANTIC_K2_CACHE_SCHEMA &&
        listOf(
            "extractorSchema",
            "pluginArtifactFingerprint",
            "workerCompilerVersion",
            "workerVersion",
            "workerProtocolVersion",
        ).all { field -> cached[field] == expected[field] }
internal fun semanticK2CachePayloadIntegrity(cache: JsonObject): String = sha(
    canonicalJson(buildJsonObject {
        put("valid", cache["valid"] ?: JsonNull)
        put("facts", cache["facts"] ?: JsonNull)
        put("diagnostics", cache["diagnostics"] ?: JsonNull)
    }).toByteArray()
)
internal fun cachePayloadIntegrityMatches(cached: JsonObject): Boolean {
    val integrity = cached["payloadIntegrity"]?.jsonPrimitive?.contentOrNull ?: return false
    if (cached["valid"] == null || cached["facts"] !is JsonArray || cached["diagnostics"] !is JsonArray) {
        return false
    }
    return integrity == semanticK2CachePayloadIntegrity(cached)
}
private fun canonicalJsonValue(value: JsonElement): JsonElement = when (value) {
    is JsonObject -> JsonObject(linkedMapOf<String, JsonElement>().also { sorted ->
        value.entries.sortedBy(Map.Entry<String, JsonElement>::key).forEach { (key, item) ->
            sorted[key] = canonicalJsonValue(item)
        }
    })
    is JsonArray -> JsonArray(value.map(::canonicalJsonValue))
    else -> value
}
private fun canonicalJson(value: JsonElement): String = canonicalJsonValue(value).toString()
private fun ByteArray.hex() = joinToString("") { "%02x".format(it) }
private fun sha(bytes: ByteArray) = "sha256:" + MessageDigest.getInstance("SHA-256").digest(bytes).hex()
private fun normalizeTokens(text: String): String {
    val out = StringBuilder(); var i = 0; var line = false; var block = false
    while (i < text.length) {
        val c = text[i]; val n = text.getOrNull(i + 1)
        if (line) { if (c == '\n') line = false; i++; continue }
        if (block) { if (c == '*' && n == '/') { block = false; i += 2 } else i++; continue }
        if (c == '/' && n == '/') { line = true; i += 2; continue }
        if (c == '/' && n == '*') { block = true; i += 2; continue }
        if (!c.isWhitespace()) out.append(c); i++
    }
    return out.toString()
}
