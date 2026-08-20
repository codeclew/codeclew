package dev.semanticthread.worker

import java.nio.charset.StandardCharsets
import java.nio.channels.FileChannel
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.nio.file.StandardCopyOption.ATOMIC_MOVE
import java.nio.file.StandardCopyOption.REPLACE_EXISTING
import java.nio.file.StandardOpenOption.CREATE_NEW
import java.nio.file.StandardOpenOption.READ
import java.nio.file.StandardOpenOption.WRITE
import java.security.MessageDigest
import java.util.Comparator
import java.util.UUID
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put

internal interface BtaCompilation21 {
    fun compilerVersion(): String
    fun compile(request: BtaCompileRequest21): BtaCompileResult21
}

internal data class BtaCompileRequest21(
    val request: IncrementalK2Request,
    val sources: List<Path>,
    val workingRoot: Path,
    val factsOutput: Path,
    val full: Boolean,
    val modifiedSources: List<Path>,
    val removedSources: List<Path>,
)

internal data class BtaCompileResult21(
    val success: Boolean,
    val compilerVersion: String,
    val rawFactLines: List<String>,
    val diagnostics: List<JsonObject>,
    val compilerMicros: Long,
)

class BtaIncrementalBackend21 internal constructor(
    private val compiler: BtaCompilation21,
    private val storeHooks: K2StoreHooks21 = K2StoreHooks21(),
) : IncrementalK2Backend {
    constructor() : this(RealBtaCompilation21, K2StoreHooks21())

    override fun analyze(request: IncrementalK2Request): IncrementalK2Result {
        val started = System.nanoTime()
        return try {
            val configDigest = btaConfigurationDigest21(request)
            val store = K2FactGenerationStore21.open(
                request.indexRoot,
                request.repo,
                request.compilation,
                request.expectedCompilerVersion,
                configDigest,
                storeHooks,
            )
            when (val locked = store.withLock {
                analyzeLocked21(request, store, configDigest, started)
            }) {
                K2StoreLockResult21.Busy -> failed21(
                    IncrementalK2Status.BUSY,
                    started,
                    "K2_INDEX_BUSY",
                )
                is K2StoreLockResult21.Acquired -> locked.value
            }
        } catch (error: Exception) {
            if (error is InterruptedException) Thread.currentThread().interrupt()
            failed21(IncrementalK2Status.FAILED_RECOVERABLE, started, "K2_BACKEND_EXCEPTION")
        } catch (_: LinkageError) {
            failed21(IncrementalK2Status.FAILED_RECOVERABLE, started, "K2_BACKEND_LINKAGE")
        }
    }

    private fun K2FactGenerationStore21.Locked.analyzeLocked21(
        request: IncrementalK2Request,
        store: K2FactGenerationStore21,
        configDigest: String,
        started: Long,
    ): IncrementalK2Result {
        val snapshot = snapshot(request.sources)
        val current = loadCurrent()
        val delta = delta(snapshot, current)
        val mutable = store.stateRoot.resolve("mutable")
        val recovery = current is K2Current21.RecoveryRequired ||
            current is K2Current21.Ready && !validMutableAuthority21(
                mutable,
                store.stateRoot,
                configDigest,
                request.expectedCompilerVersion,
                current.generation.sourceSetDigest,
            ) ||
            current is K2Current21.Empty && Files.exists(mutable, NOFOLLOW_LINKS)
        if (current is K2Current21.Ready && !recovery && delta.isEmpty21()) {
            return success21(
                current.generation,
                IncrementalK2Status.UNCHANGED_HIT,
                started,
                compilerMicros = 0,
                firMicros = 0,
                compiledFiles = 0,
                reusedFiles = snapshot.files.size,
                recovered = false,
            )
        }

        val full = current !is K2Current21.Ready || recovery
        val status = when {
            recovery -> IncrementalK2Status.RECOVERED_FULL
            full -> IncrementalK2Status.COLD_FULL
            else -> IncrementalK2Status.INCREMENTAL
        }
        val version = compiler.compilerVersion()
        if (version != request.expectedCompilerVersion) {
            return failed21(
                IncrementalK2Status.FAILED_RECOVERABLE,
                started,
                "K2_COMPILER_VERSION_MISMATCH",
                totalFiles = snapshot.files.size,
            )
        }

        val transaction = beginDirty(snapshot, if (full) K2Current21.Empty else current)
        if (full) resetMutable21(mutable, store.stateRoot)
        val factsOutput = mutable.resolve("facts.jsonl")
        require(factsOutput.parent == mutable && !Files.isSymbolicLink(factsOutput))
        Files.deleteIfExists(factsOutput)
        val byPath = snapshot.files.associateBy(K2SourceFile21::path)
        val modified = if (full) {
            snapshot.files.map(K2SourceFile21::file)
        } else {
            delta.addedOrModified.sorted().map { requireNotNull(byPath[it]).file }
        }
        val removed = if (full) emptyList() else delta.removed.sorted().map { relative ->
            request.repo.resolve(relative).also { path ->
                require(path.isAbsolute && path == path.normalize() && path.startsWith(request.repo))
            }
        }
        val sortedSources = snapshot.files.map(K2SourceFile21::file)
        val compilation = compiler.compile(
            BtaCompileRequest21(
                request = request.copy(sources = sortedSources),
                sources = sortedSources,
                workingRoot = mutable,
                factsOutput = factsOutput,
                full = full,
                modifiedSources = modified,
                removedSources = removed,
            ),
        )
        require(compilation.compilerMicros >= 0) { "negative compiler time" }
        require(compilation.compilerVersion == request.expectedCompilerVersion) {
            "compiler version changed during compilation"
        }
        if (!compilation.success) {
            return failed21(
                status,
                started,
                "K2_COMPILATION_FAILED",
                compilerMicros = compilation.compilerMicros,
                totalFiles = snapshot.files.size,
                recovered = recovery,
                diagnostics = sanitizeDiagnostics21(compilation.diagnostics),
            )
        }
        val afterCompile = snapshot(snapshot.files.map(K2SourceFile21::file))
        require(afterCompile == snapshot) { "sources changed during compilation" }
        val normalized = normalizeFacts21(request.repo, snapshot, compilation.rawFactLines)
        makeMutableDurable21(
            mutable,
            store.stateRoot,
            configDigest,
            request.expectedCompilerVersion,
            snapshot.sourceSetDigest,
        )
        val published = transaction.publishCompiledGeneration(normalized.lines)
        require(published.committed) { "store did not commit generation" }
        return success21(
            published.generation,
            status,
            started,
            compilerMicros = compilation.compilerMicros,
            firMicros = normalized.firMicros,
            compiledFiles = normalized.receipts,
            reusedFiles = snapshot.files.size - normalized.receipts,
            recovered = recovery,
            diagnostics = if (published.cleanupPending) {
                listOf(diagnostic21("K2_DIRTY_CLEANUP_PENDING"))
            } else {
                emptyList()
            },
        )
    }
}

@OptIn(org.jetbrains.kotlin.buildtools.api.ExperimentalBuildToolsApi::class)
internal object RealBtaCompilation21 : BtaCompilation21 {
    private val compilationLock = Any()
    private val service by lazy(LazyThreadSafetyMode.SYNCHRONIZED) {
        org.jetbrains.kotlin.buildtools.api.CompilationService
            .loadImplementation(javaClass.classLoader)
    }

    override fun compilerVersion(): String = synchronized(compilationLock) {
        service.getCompilerVersion()
    }

    override fun compile(request: BtaCompileRequest21): BtaCompileResult21 =
        synchronized(compilationLock) { compileLocked(request) }

    private fun compileLocked(input: BtaCompileRequest21): BtaCompileResult21 {
        Files.createDirectories(requireNotNull(input.factsOutput.parent))
        Files.deleteIfExists(input.factsOutput)
        val semantic = input.request
        val compilerVersion = service.getCompilerVersion()
        if (compilerVersion != semantic.expectedCompilerVersion) {
            return BtaCompileResult21(
                success = false,
                compilerVersion = compilerVersion,
                rawFactLines = emptyList(),
                diagnostics = listOf(diagnostic21("K2_BTA_VERSION_MISMATCH")),
                compilerMicros = 0,
            )
        }
        require(semantic.compilerPlugins.none { it == semantic.factsPlugin }) {
            "facts plugin must not appear in compilerPlugins"
        }
        require(semantic.compilerPluginOptions.none { it.startsWith("plugin:$FACTS_PLUGIN_ID:") }) {
            "facts plugin options are reserved"
        }

        val classes = input.workingRoot.resolve("classes")
        val working = input.workingRoot.resolve("ic")
        Files.createDirectories(classes)
        Files.createDirectories(working)
        val snapshots = semantic.classpath.mapIndexed { index, entry ->
            val snapshot = input.workingRoot.resolve("classpath-snapshots/$index.bin")
            Files.createDirectories(requireNotNull(snapshot.parent))
            if (!Files.isRegularFile(snapshot, NOFOLLOW_LINKS)) {
                service.calculateClasspathSnapshot(
                    entry.toFile(),
                    org.jetbrains.kotlin.buildtools.api.jvm.ClassSnapshotGranularity.CLASS_MEMBER_LEVEL,
                ).saveSnapshot(snapshot.toFile())
            }
            snapshot.toFile()
        }
        val configuration = service.makeJvmCompilationConfiguration().useLogger(SilentBtaLogger21)
        val incrementalConfiguration = configuration
            .makeClasspathSnapshotBasedIncrementalCompilationConfiguration()
            .setRootProjectDir(semantic.repo.toFile())
            .setBuildDir(input.workingRoot.toFile())
            .usePreciseJavaTracking(true)
            .usePreciseCompilationResultsBackup(true)
            .keepIncrementalCompilationCachesInMemory(false)
            .forceNonIncrementalMode(input.full)
            .useOutputDirs(listOf(classes.toFile(), working.toFile()))
            .assureNoClasspathSnapshotsChanges(true)
        val sourceChanges = if (input.full) {
            org.jetbrains.kotlin.buildtools.api.SourcesChanges.Unknown
        } else {
            org.jetbrains.kotlin.buildtools.api.SourcesChanges.Known(
                input.modifiedSources.map(Path::toFile),
                input.removedSources.map(Path::toFile),
            )
        }
        configuration.useIncrementalCompilation(
            working.toFile(),
            sourceChanges,
            org.jetbrains.kotlin.buildtools.api.jvm.ClasspathSnapshotBasedIncrementalCompilationApproachParameters(
                snapshots,
                input.workingRoot.resolve("shrunk-classpath-snapshot.bin").toFile(),
            ),
            incrementalConfiguration,
        )

        val arguments = mutableListOf(
            "-d",
            classes.toString(),
            "-classpath",
            semantic.classpath.joinToString(java.io.File.pathSeparator),
            "-no-stdlib",
            "-no-reflect",
            "-jdk-home",
            semantic.jdkHome.toString(),
            "-jvm-target",
            semantic.jvmTarget.removePrefix("JVM_"),
            "-module-name",
            semantic.moduleName,
        )
        semantic.languageVersion?.let { arguments += listOf("-language-version", it) }
        semantic.apiVersion?.let { arguments += listOf("-api-version", it) }
        if (semantic.friendPaths.isNotEmpty()) {
            arguments += "-Xfriend-paths=${semantic.friendPaths.joinToString(java.io.File.pathSeparator)}"
        }
        arguments += semantic.freeCompilerArguments
        arguments += semantic.optIns.map { "-opt-in=$it" }
        arguments += semantic.compilerPlugins.map { "-Xplugin=$it" }
        semantic.compilerPluginOptions.forEach { option ->
            arguments += listOf("-P", option)
        }
        arguments += listOf(
            "-Xplugin=${semantic.factsPlugin}",
            "-P",
            "plugin:$FACTS_PLUGIN_ID:output=${input.factsOutput}",
        )

        val projectId = org.jetbrains.kotlin.buildtools.api.ProjectId.ProjectUUID(
            UUID.nameUUIDFromBytes(input.workingRoot.toString().toByteArray(StandardCharsets.UTF_8)),
        )
        val strategy = service.makeCompilerExecutionStrategyConfiguration().useInProcessStrategy()
        var outcome: org.jetbrains.kotlin.buildtools.api.CompilationResult? = null
        var failure: Throwable? = null
        val started = System.nanoTime()
        try {
            outcome = service.compileJvm(
                projectId,
                strategy,
                configuration,
                input.sources.map(Path::toFile),
                arguments,
            )
        } catch (caught: Throwable) {
            failure = caught
        }
        try {
            service.finishProjectCompilation(projectId)
        } catch (caught: Throwable) {
            failure = combineFailure21(failure, caught)
        }
        val compilerMicros = (System.nanoTime() - started) / 1_000

        var rawFactLines = emptyList<String>()
        if (failure == null && Files.isRegularFile(input.factsOutput, NOFOLLOW_LINKS)) {
            try {
                rawFactLines = Files.readAllLines(input.factsOutput, StandardCharsets.UTF_8)
            } catch (caught: Throwable) {
                failure = combineFailure21(failure, caught)
            }
        }
        try {
            Files.deleteIfExists(input.factsOutput)
        } catch (caught: Throwable) {
            failure = combineFailure21(failure, caught)
        }
        failure?.let { throw it }

        val success = outcome == org.jetbrains.kotlin.buildtools.api.CompilationResult.COMPILATION_SUCCESS
        return BtaCompileResult21(
            success = success,
            compilerVersion = compilerVersion,
            rawFactLines = rawFactLines,
            diagnostics = if (success) emptyList()
                else listOf(diagnostic21("K2_BTA_COMPILATION_FAILED")),
            compilerMicros = compilerMicros,
        )
    }

    private fun combineFailure21(primary: Throwable?, additional: Throwable): Throwable {
        if (primary == null) return additional
        primary.addSuppressed(additional)
        return primary
    }

    private object SilentBtaLogger21 : org.jetbrains.kotlin.buildtools.api.KotlinLogger {
        override val isDebugEnabled: Boolean = false
        override fun debug(message: String) = Unit
        override fun info(message: String) = Unit
        override fun lifecycle(message: String) = Unit
        override fun warn(message: String, cause: Throwable?) = Unit
        override fun error(message: String, cause: Throwable?) = Unit
    }
}

private data class NormalizedFacts21(
    val lines: List<String>,
    val receipts: Int,
    val firMicros: Long,
)

private fun normalizeFacts21(
    repo: Path,
    snapshot: K2SourceSnapshot21,
    lines: List<String>,
): NormalizedFacts21 {
    val canonicalRepo = repo.toRealPath()
    require(repo.isAbsolute && repo == repo.normalize() && repo == canonicalRepo)
    val files = snapshot.files.associateBy(K2SourceFile21::path)
    val absolute = snapshot.files.associate { it.file.toString() to it.path }
    val normalized = mutableListOf<String>()
    val receipts = linkedSetOf<String>()
    var firMicros = 0L
    require(lines.all { it.isNotBlank() }) { "compiler output contains a blank JSONL row" }
    lines.forEach { line ->
        val row = Json.parseToJsonElement(line).jsonObject
        val type = row["recordType"]?.jsonPrimitive?.also { require(it.isString) }?.contentOrNull
            ?: throw IllegalArgumentException("compiler row has no recordType")
        require(type.isNotBlank()) { "compiler row has blank recordType" }
        val raw = row["file"]?.jsonPrimitive?.also { require(it.isString) }?.contentOrNull
            ?: throw IllegalArgumentException("compiler row has no file")
        val relative = when {
            raw in files -> raw
            raw in absolute -> requireNotNull(absolute[raw])
            else -> throw IllegalArgumentException("compiler row has noncanonical file identity")
        }
        val timing = row["firExtractionMicros"]
        if (timing != null) {
            require(type == "FIR_CFG") { "FIR timing is only valid on FIR_CFG" }
            val primitive = timing.jsonPrimitive
            require(!primitive.isString)
            val value = requireNotNull(primitive.longOrNull).also { require(it >= 0) }
            firMicros = Math.addExact(firMicros, value)
        }
        if (type == "FIR_FILE_RECEIPT") receipts.add(relative)
        normalized += buildJsonObject {
            row.forEach { (key, value) ->
                if (key != "firExtractionMicros") {
                    put(key, if (key == "file") JsonPrimitive(relative) else value)
                }
            }
        }.toString()
    }
    return NormalizedFacts21(
        deduplicateCanonicalK2FactLines21(normalized),
        receipts.size,
        firMicros,
    )
}

internal fun btaConfigurationDigest21(request: IncrementalK2Request): String {
    require(SEMANTIC_CONFIGURATION_DIGEST_21.matches(request.semanticConfigurationDigest)) {
        "semanticConfigurationDigest must be trusted sha256 authority"
    }
    require(COMPILER_VERSION_21.matches(request.expectedCompilerVersion)) {
        "expectedCompilerVersion is not canonical"
    }
    val authority = listOf(
        "codeclew-bta-configuration/0.2",
        request.semanticConfigurationDigest,
        request.expectedCompilerVersion,
    ).joinToString("\u0000").toByteArray(StandardCharsets.UTF_8)
    return sha256Backend21(authority)
}

private fun K2SourceDelta21.isEmpty21(): Boolean =
    addedOrModified.isEmpty() && removed.isEmpty()

private fun validMutableAuthority21(
    mutable: Path,
    stateRoot: Path,
    configDigest: String,
    compilerVersion: String,
    sourceSetDigest: String,
): Boolean = try {
    val entries = checkedMutableEntries21(mutable, stateRoot)
    val marker = mutable.resolve(MUTABLE_AUTHORITY_FILE_21)
    marker in entries && Files.isRegularFile(marker, NOFOLLOW_LINKS) &&
        readMutableAuthority21(marker).contentEquals(
            mutableAuthorityBytes21(configDigest, compilerVersion, sourceSetDigest),
        )
} catch (_: Exception) {
    false
}

private fun resetMutable21(mutable: Path, stateRoot: Path) {
    require(mutable.isAbsolute && mutable == mutable.normalize() && mutable.parent == stateRoot)
    if (Files.exists(mutable, NOFOLLOW_LINKS)) {
        Files.walk(mutable).use { stream ->
            stream.iterator().asSequence().toList().sortedWith(Comparator.reverseOrder())
                .forEach { Files.deleteIfExists(it) }
        }
    }
    Files.createDirectory(mutable)
    require(checkedMutableEntries21(mutable, stateRoot) == listOf(mutable))
}

private fun makeMutableDurable21(
    mutable: Path,
    stateRoot: Path,
    configDigest: String,
    compilerVersion: String,
    sourceSetDigest: String,
) {
    val entries = checkedMutableEntries21(mutable, stateRoot)
    entries.sortedWith(compareByDescending<Path> { it.nameCount }.thenBy(Path::toString))
        .forEach { entry ->
            when {
                Files.isRegularFile(entry, NOFOLLOW_LINKS) -> forceRegularFile21(entry)
                Files.isDirectory(entry, NOFOLLOW_LINKS) -> forceDirectoryBackend21(entry)
                else -> error("mutable tree contains a special file")
            }
        }
    writeMutableAuthority21(
        mutable,
        stateRoot,
        mutableAuthorityBytes21(configDigest, compilerVersion, sourceSetDigest),
    )
}

private fun checkedMutableEntries21(mutable: Path, stateRoot: Path): List<Path> {
    require(mutable.isAbsolute && mutable == mutable.normalize() && mutable.parent == stateRoot)
    require(!Files.isSymbolicLink(mutable) && Files.isDirectory(mutable, NOFOLLOW_LINKS))
    require(mutable.toRealPath() == mutable)
    return Files.walk(mutable).use { stream ->
        stream.iterator().asSequence().toList().also { entries ->
            entries.forEach { entry ->
                require(
                    entry.isAbsolute && entry == entry.normalize() && entry.startsWith(mutable) &&
                        !Files.isSymbolicLink(entry) && entry.toRealPath() == entry,
                )
                require(
                    Files.isDirectory(entry, NOFOLLOW_LINKS) ||
                        Files.isRegularFile(entry, NOFOLLOW_LINKS),
                ) { "mutable tree contains a special file" }
            }
        }
    }
}

private fun writeMutableAuthority21(
    mutable: Path,
    stateRoot: Path,
    bytes: ByteArray,
) {
    val marker = mutable.resolve(MUTABLE_AUTHORITY_FILE_21)
    require(marker.parent == mutable && !Files.isSymbolicLink(marker))
    val temporary = mutable.resolve(".$MUTABLE_AUTHORITY_FILE_21.${UUID.randomUUID()}.tmp")
    try {
        FileChannel.open(temporary, CREATE_NEW, WRITE, NOFOLLOW_LINKS).use { channel ->
            val buffer = java.nio.ByteBuffer.wrap(bytes)
            while (buffer.hasRemaining()) channel.write(buffer)
            channel.force(true)
        }
        Files.move(temporary, marker, ATOMIC_MOVE, REPLACE_EXISTING)
        require(!Files.isSymbolicLink(marker) && Files.isRegularFile(marker, NOFOLLOW_LINKS))
        require(readMutableAuthority21(marker).contentEquals(bytes))
        forceDirectoryBackend21(mutable)
        forceDirectoryBackend21(stateRoot)
    } finally {
        Files.deleteIfExists(temporary)
    }
}

private fun readMutableAuthority21(path: Path): ByteArray {
    require(!Files.isSymbolicLink(path) && Files.isRegularFile(path, NOFOLLOW_LINKS))
    return FileChannel.open(path, READ, NOFOLLOW_LINKS).use { channel ->
        val size = channel.size()
        require(size in 1L..4_096L)
        val buffer = java.nio.ByteBuffer.allocate(size.toInt())
        while (buffer.hasRemaining()) require(channel.read(buffer) >= 0)
        require(!Files.isSymbolicLink(path) && Files.isRegularFile(path, NOFOLLOW_LINKS))
        buffer.array()
    }
}

private fun forceRegularFile21(path: Path) {
    require(!Files.isSymbolicLink(path) && Files.isRegularFile(path, NOFOLLOW_LINKS))
    FileChannel.open(path, WRITE, NOFOLLOW_LINKS).use { it.force(true) }
    require(!Files.isSymbolicLink(path) && Files.isRegularFile(path, NOFOLLOW_LINKS))
}

private fun forceDirectoryBackend21(path: Path) {
    require(!Files.isSymbolicLink(path) && Files.isDirectory(path, NOFOLLOW_LINKS))
    FileChannel.open(path, READ, NOFOLLOW_LINKS).use { it.force(true) }
    require(!Files.isSymbolicLink(path) && Files.isDirectory(path, NOFOLLOW_LINKS))
}

private fun mutableAuthorityBytes21(
    configDigest: String,
    compilerVersion: String,
    sourceSetDigest: String,
): ByteArray {
    require(HEX_21.matches(configDigest) && HEX_21.matches(sourceSetDigest))
    require(COMPILER_VERSION_21.matches(compilerVersion))
    return buildString {
        append("codeclew-k2-mutable-authority/0.1\n")
        append("configDigest=").append(configDigest).append('\n')
        append("compilerVersion=").append(compilerVersion).append('\n')
        append("sourceSetDigest=").append(sourceSetDigest).append('\n')
    }.toByteArray(StandardCharsets.UTF_8)
}

private fun sha256Backend21(bytes: ByteArray): String =
    MessageDigest.getInstance("SHA-256").digest(bytes)
        .joinToString("") { "%02x".format(it.toInt() and 0xff) }

private const val MUTABLE_AUTHORITY_FILE_21 = "AUTHORITY"
private val SEMANTIC_CONFIGURATION_DIGEST_21 = Regex("sha256:[0-9a-f]{64}")
private val HEX_21 = Regex("[0-9a-f]{64}")
private val COMPILER_VERSION_21 = Regex("[A-Za-z0-9][A-Za-z0-9._+-]{0,63}")

private fun sanitizeDiagnostics21(rows: List<JsonObject>): List<JsonObject> {
    val safe = Regex("[A-Z][A-Z0-9_]{0,95}")
    return rows.map { row ->
        (row["code"] as? JsonPrimitive)?.takeIf { it.isString }?.contentOrNull
            ?.takeIf(safe::matches) ?: "K2_COMPILATION_FAILED"
    }.distinct().sorted().map(::diagnostic21)
}

internal fun diagnostic21(code: String): JsonObject = buildJsonObject { put("code", code) }

private fun success21(
    generation: K2StoredGeneration21,
    status: IncrementalK2Status,
    started: Long,
    compilerMicros: Long,
    firMicros: Long,
    compiledFiles: Int,
    reusedFiles: Int,
    recovered: Boolean,
    diagnostics: List<JsonObject> = emptyList(),
): IncrementalK2Result = IncrementalK2Result(
    valid = true,
    facts = generation.facts,
    diagnostics = diagnostics,
    status = status,
    totalMicros = maxOf(elapsedMicros21(started), compilerMicros),
    compilerMicros = compilerMicros,
    firExtractionMicros = firMicros,
    totalFiles = generation.files.size,
    compiledFiles = compiledFiles,
    reusedFiles = reusedFiles,
    recovered = recovered,
    graphDigest = generation.graphDigest,
)

private fun failed21(
    status: IncrementalK2Status,
    started: Long,
    code: String,
    compilerMicros: Long = 0,
    totalFiles: Int = 0,
    recovered: Boolean = false,
    diagnostics: List<JsonObject> = emptyList(),
): IncrementalK2Result = IncrementalK2Result(
    valid = false,
    facts = emptyList(),
    diagnostics = (diagnostics + diagnostic21(code)).distinctBy {
        it["code"]?.jsonPrimitive?.contentOrNull
    }.sortedBy { it["code"]?.jsonPrimitive?.contentOrNull },
    status = status,
    totalMicros = maxOf(elapsedMicros21(started), compilerMicros),
    compilerMicros = compilerMicros,
    firExtractionMicros = 0,
    totalFiles = totalFiles,
    compiledFiles = 0,
    reusedFiles = 0,
    recovered = recovered,
    graphDigest = null,
)

private fun elapsedMicros21(started: Long): Long =
    ((System.nanoTime() - started) / 1_000).coerceAtLeast(0)
