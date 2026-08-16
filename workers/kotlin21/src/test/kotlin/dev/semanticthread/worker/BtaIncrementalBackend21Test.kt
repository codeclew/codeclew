package dev.semanticthread.worker

import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.util.Comparator
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertIs
import kotlin.test.assertNotEquals
import kotlin.test.assertTrue

class BtaIncrementalBackend21Test {
    @Test
    fun configurationIdentityIsConstantTimeTrustedAuthority() {
        val fixture = fixture21()
        val request = fixture.request()
        val digest = btaConfigurationDigest21(request)
        Files.writeString(fixture.a, "class A { val sourceOnly = true }")
        Files.delete(fixture.classpath)
        Files.delete(fixture.friend)
        Files.delete(fixture.plugin)
        Files.delete(fixture.factsPlugin)
        deleteTree21(fixture.jdk)
        val otherIndex = Files.createDirectory(fixture.root.resolve("other-index"))
        assertEquals(
            digest,
            btaConfigurationDigest21(
                request.copy(indexRoot = otherIndex, sources = request.sources.reversed()),
            ),
        )
        assertNotEquals(
            digest,
            btaConfigurationDigest21(
                request.copy(semanticConfigurationDigest = "sha256:" + "2".repeat(64)),
            ),
        )
        assertNotEquals(
            digest,
            btaConfigurationDigest21(request.copy(expectedCompilerVersion = "2.1.22")),
        )
        assertFailsWith<IllegalArgumentException> {
            btaConfigurationDigest21(request.copy(semanticConfigurationDigest = "untrusted"))
        }
    }

    @Test
    fun coldFullAndUnchangedHitSkipsCompiler() {
        val fixture = fixture21()
        val compiler = FixtureCompiler21()
        val backend = BtaIncrementalBackend21(compiler)

        val cold = backend.analyze(fixture.request())
        assertTrue(cold.valid)
        assertEquals(IncrementalK2Status.COLD_FULL, cold.status)
        assertEquals(4, cold.totalFiles)
        assertEquals(4, cold.compiledFiles)
        assertEquals(0, cold.reusedFiles)
        assertEquals(6L, cold.firExtractionMicros)
        assertTrue(cold.facts.all { "firExtractionMicros" !in it })
        assertTrue(cold.diagnostics.isEmpty())
        assertEquals(1, compiler.versionCalls)
        assertEquals(1, compiler.compileCalls)

        val hit = backend.analyze(fixture.request())
        assertTrue(hit.valid)
        assertEquals(IncrementalK2Status.UNCHANGED_HIT, hit.status)
        assertEquals(cold.facts, hit.facts)
        assertEquals(cold.graphDigest, hit.graphDigest)
        assertEquals(cold.diagnostics, hit.diagnostics)
        assertEquals(0, hit.compilerMicros)
        assertEquals(0, hit.firExtractionMicros)
        assertEquals(0, hit.compiledFiles)
        assertEquals(4, hit.reusedFiles)
        assertEquals(1, compiler.versionCalls)
        assertEquals(1, compiler.compileCalls)
    }

    @Test
    fun incrementalDependentClosureAddsDeletesAndReusesOnlyUnreceiptedShard() {
        val fixture = fixture21()
        val compiler = FixtureCompiler21()
        val backend = BtaIncrementalBackend21(compiler)
        assertTrue(backend.analyze(fixture.request()).valid)
        val before = fixture.generation()

        Files.writeString(fixture.a, "class A { fun changed() = 1 }")
        Files.delete(fixture.b)
        val d = Files.writeString(fixture.repo.resolve("D.kt"), "class D")
        compiler.dependents = setOf("C.kt")
        val request = fixture.request(listOf(fixture.a, fixture.c, fixture.empty, d))
        val incremental = backend.analyze(request)

        assertTrue(incremental.valid)
        assertEquals(IncrementalK2Status.INCREMENTAL, incremental.status)
        assertEquals(4, incremental.totalFiles)
        assertEquals(3, incremental.compiledFiles)
        assertEquals(1, incremental.reusedFiles)
        assertEquals(6L, incremental.firExtractionMicros)
        val call = compiler.requests.last()
        assertFalse(call.full)
        assertEquals(listOf(fixture.a, d), call.modifiedSources)
        assertEquals(listOf(fixture.repo.resolve("B.kt")), call.removedSources)
        val after = fixture.generation(request)
        assertFalse(after.files.any { it.path == "B.kt" })
        assertNotEquals(before.objectHash21("C.kt"), after.objectHash21("C.kt"))
        assertEquals(before.objectHash21("Empty.kt"), after.objectHash21("Empty.kt"))

        val fresh = fixture21()
        Files.writeString(fresh.a, Files.readString(fixture.a))
        Files.writeString(fresh.c, Files.readString(fixture.c))
        Files.delete(fresh.b)
        val freshD = Files.writeString(fresh.repo.resolve("D.kt"), Files.readString(d))
        val full = BtaIncrementalBackend21(FixtureCompiler21()).analyze(
            fresh.request(listOf(fresh.a, fresh.c, fresh.empty, freshD)),
        )
        assertEquals(full.facts, incremental.facts)
        assertEquals(full.graphDigest, incremental.graphDigest)
    }

    @Test
    fun compilerAndFactContractFailuresNeverReturnStaleFactsAndRecoverFull() {
        val fixture = fixture21()
        val compiler = FixtureCompiler21().also { it.outcome = FixtureOutcome21.COMPILER_FAILURE }
        val backend = BtaIncrementalBackend21(compiler)

        val failedCompile = backend.analyze(fixture.request())
        assertFalse(failedCompile.valid)
        assertEquals(IncrementalK2Status.COLD_FULL, failedCompile.status)
        assertTrue(failedCompile.facts.isEmpty())
        assertTrue(fixture.dirtyExists())
        compiler.outcome = FixtureOutcome21.SUCCESS
        assertEquals(IncrementalK2Status.RECOVERED_FULL, backend.analyze(fixture.request()).status)

        Files.writeString(fixture.a, "class A { val malformed = true }")
        val current = Files.readAllBytes(fixture.store().stateRoot.resolve("CURRENT"))
        compiler.outcome = FixtureOutcome21.MALFORMED_JSON
        val malformed = backend.analyze(fixture.request())
        assertFalse(malformed.valid)
        assertEquals(IncrementalK2Status.FAILED_RECOVERABLE, malformed.status)
        assertTrue(malformed.facts.isEmpty())
        assertTrue(current.contentEquals(Files.readAllBytes(fixture.store().stateRoot.resolve("CURRENT"))))
        assertTrue(fixture.dirtyExists())
        compiler.outcome = FixtureOutcome21.SUCCESS
        assertEquals(IncrementalK2Status.RECOVERED_FULL, backend.analyze(fixture.request()).status)

        Files.writeString(fixture.c, "class C { val alias = true }")
        val beforeBlank = Files.readAllBytes(fixture.store().stateRoot.resolve("CURRENT"))
        compiler.outcome = FixtureOutcome21.BLANK_ROW
        val blank = backend.analyze(fixture.request())
        assertFalse(blank.valid)
        assertEquals(IncrementalK2Status.FAILED_RECOVERABLE, blank.status)
        assertTrue(beforeBlank.contentEquals(Files.readAllBytes(fixture.store().stateRoot.resolve("CURRENT"))))
        assertTrue(fixture.dirtyExists())
        compiler.outcome = FixtureOutcome21.SUCCESS
        assertEquals(IncrementalK2Status.RECOVERED_FULL, backend.analyze(fixture.request()).status)

        Files.writeString(fixture.c, "class C { val alias = false }")
        compiler.outcome = FixtureOutcome21.ALIASED_FILE
        val aliased = backend.analyze(fixture.request())
        assertFalse(aliased.valid)
        assertEquals(IncrementalK2Status.FAILED_RECOVERABLE, aliased.status)
        assertTrue(aliased.facts.isEmpty())
        assertTrue(fixture.dirtyExists())
    }

    @Test
    fun missingMutableForcesRecoveredFullEvenWithoutSourceDelta() {
        val fixture = fixture21()
        val compiler = FixtureCompiler21()
        val backend = BtaIncrementalBackend21(compiler)
        assertTrue(backend.analyze(fixture.request()).valid)
        deleteTree21(fixture.store().stateRoot.resolve("mutable"))

        val recovered = backend.analyze(fixture.request())
        assertTrue(recovered.valid)
        assertEquals(IncrementalK2Status.RECOVERED_FULL, recovered.status)
        assertTrue(recovered.recovered)
        assertTrue(compiler.requests.last().full)
        assertEquals(2, compiler.compileCalls)
    }

    @Test
    fun mutableAuthorityMismatchAndCorruptionForceRecoveredFull() {
        val fixture = fixture21()
        val compiler = FixtureCompiler21()
        val backend = BtaIncrementalBackend21(compiler)
        val request = fixture.request()
        assertTrue(backend.analyze(request).valid)
        val marker = fixture.store(request).stateRoot.resolve("mutable/AUTHORITY")
        val authority = Files.readString(marker)
        val configDigest = btaConfigurationDigest21(request)
        Files.writeString(marker, authority.replace(configDigest, "0".repeat(64)))

        val mismatch = backend.analyze(request)
        assertTrue(mismatch.valid)
        assertEquals(IncrementalK2Status.RECOVERED_FULL, mismatch.status)
        assertEquals(2, compiler.compileCalls)

        Files.writeString(marker, "corrupt\n")
        val corrupt = backend.analyze(request)
        assertTrue(corrupt.valid)
        assertEquals(IncrementalK2Status.RECOVERED_FULL, corrupt.status)
        assertEquals(3, compiler.compileCalls)
    }

    @Test
    fun cleanupFailureReturnsCommittedWarningAndNextRunRecovers() {
        val fixture = fixture21()
        val compiler = FixtureCompiler21()
        val backend = BtaIncrementalBackend21(
            compiler,
            K2StoreHooks21(beforeCleanup = { error("sensitive ${fixture.root}") }),
        )

        val committed = backend.analyze(fixture.request())
        assertTrue(committed.valid)
        assertEquals(IncrementalK2Status.COLD_FULL, committed.status)
        assertTrue(committed.facts.isNotEmpty())
        assertEquals(
            listOf(diagnostic21("K2_DIRTY_CLEANUP_PENDING")),
            committed.diagnostics,
        )
        assertEquals(setOf("code"), committed.diagnostics.single().keys)
        assertFalse(committed.diagnostics.toString().contains(fixture.root.toString()))
        assertTrue(fixture.dirtyExists())

        val recovered = BtaIncrementalBackend21(compiler).analyze(fixture.request())
        assertTrue(recovered.valid)
        assertEquals(IncrementalK2Status.RECOVERED_FULL, recovered.status)
        assertTrue(recovered.diagnostics.isEmpty())
        assertFalse(fixture.dirtyExists())
    }

    @Test
    fun versionMismatchIsPreDirtyAndBusySkipsCompiler() {
        val fixture = fixture21()
        val compiler = FixtureCompiler21().also { it.version = "2.1.20" }
        val backend = BtaIncrementalBackend21(compiler)
        val mismatch = backend.analyze(fixture.request())
        assertFalse(mismatch.valid)
        assertEquals(IncrementalK2Status.FAILED_RECOVERABLE, mismatch.status)
        assertEquals(0, compiler.compileCalls)
        assertFalse(fixture.dirtyExists())

        compiler.version = "2.1.21"
        val probes = compiler.versionCalls
        val held = fixture.store().withLock { backend.analyze(fixture.request()) }
        val busy = assertIs<IncrementalK2Result>(
            assertIs<K2StoreLockResult21.Acquired<*>>(held).value,
        )
        assertFalse(busy.valid)
        assertEquals(IncrementalK2Status.BUSY, busy.status)
        assertEquals(probes, compiler.versionCalls)
        assertEquals(0, compiler.compileCalls)
    }
}

private enum class FixtureOutcome21 {
    SUCCESS,
    COMPILER_FAILURE,
    MALFORMED_JSON,
    BLANK_ROW,
    ALIASED_FILE,
}

private class FixtureCompiler21 : BtaCompilation21 {
    var version = "2.1.21"
    var versionCalls = 0
    var compileCalls = 0
    var outcome = FixtureOutcome21.SUCCESS
    var dependents: Set<String> = emptySet()
    val requests = mutableListOf<BtaCompileRequest21>()

    override fun compilerVersion(): String {
        versionCalls++
        return version
    }

    override fun compile(request: BtaCompileRequest21): BtaCompileResult21 {
        compileCalls++
        requests += request
        if (outcome == FixtureOutcome21.COMPILER_FAILURE) {
            return BtaCompileResult21(
                false,
                version,
                emptyList(),
                listOf(buildJsonObject {
                    put("code", "K2_TEST_COMPILATION_ERROR")
                    put("message", request.workingRoot.toString())
                }),
                11,
            )
        }
        if (outcome == FixtureOutcome21.MALFORMED_JSON) {
            return BtaCompileResult21(true, version, listOf("{bad"), emptyList(), 11)
        }
        val relative = request.sources.associateBy { request.request.repo.relativize(it).toString() }
        val selected = if (request.full) request.sources else {
            (request.modifiedSources + dependents.map { requireNotNull(relative[it]) }).distinct()
        }.sortedBy { request.request.repo.relativize(it).toString() }
        val world = request.sources.sortedBy(Path::toString).joinToString("|") { source ->
            "${request.request.repo.relativize(source)}:${Files.readString(source)}"
        }
        val lines = selected.flatMapIndexed { index, source ->
            val path = request.request.repo.relativize(source).toString()
            val receiptFile = if (outcome == FixtureOutcome21.ALIASED_FILE && index == 0) "./$path"
                else if (index % 2 == 0) path else source.toString()
            buildList {
                add(buildJsonObject {
                    put("recordType", "FIR_FILE_RECEIPT")
                    put("schema", "fir-file-receipt/0.1")
                    put("file", receiptFile)
                }.toString())
                if (Files.readString(source).isNotEmpty()) add(buildJsonObject {
                    put("recordType", "FIR_CFG")
                    put("file", if (index % 2 == 0) source.toString() else path)
                    put("source", Files.readString(source))
                    put("world", world)
                    put("firExtractionMicros", 2)
                }.toString())
            }
        }
        val output = if (outcome == FixtureOutcome21.BLANK_ROW) lines + " " else lines
        return BtaCompileResult21(
            true,
            version,
            output,
            listOf(buildJsonObject {
                put("code", "K2_TEST_WARNING")
                put("message", request.workingRoot.toString())
            }),
            11,
        )
    }
}

private data class Fixture21(
    val root: Path,
    val repo: Path,
    val index: Path,
    val jdk: Path,
    val classpath: Path,
    val friend: Path,
    val plugin: Path,
    val factsPlugin: Path,
    val a: Path,
    val b: Path,
    val c: Path,
    val empty: Path,
) {
    fun request(sources: List<Path> = listOf(a, b, c, empty)): IncrementalK2Request =
        IncrementalK2Request(
            indexRoot = index,
            repo = repo,
            compilation = ":workers:kotlin21/main",
            semanticConfigurationDigest = TEST_SEMANTIC_CONFIGURATION_DIGEST_21,
            expectedCompilerVersion = "2.1.21",
            moduleName = "fixture",
            sources = sources,
            classpath = listOf(classpath),
            friendPaths = listOf(friend),
            compilerPlugins = listOf(plugin),
            compilerPluginOptions = listOf("plugin:fixture:enabled=true"),
            freeCompilerArguments = listOf("-Xfixture"),
            optIns = listOf("fixture.Experimental"),
            jdkHome = jdk,
            jvmTarget = "21",
            languageVersion = "2.1",
            apiVersion = "2.1",
            factsPlugin = factsPlugin,
        )

    fun store(semantic: IncrementalK2Request = request()): K2FactGenerationStore21 =
        K2FactGenerationStore21.open(
            index,
            repo,
            semantic.compilation,
            semantic.expectedCompilerVersion,
            btaConfigurationDigest21(semantic),
        )

    fun generation(semantic: IncrementalK2Request = request()): K2StoredGeneration21 {
        val result = store(semantic).withLock {
            assertIs<K2Current21.Ready>(loadCurrent()).generation
        }
        return assertIs<K2StoredGeneration21>(
            assertIs<K2StoreLockResult21.Acquired<*>>(result).value,
        )
    }

    fun dirtyExists(): Boolean = Files.exists(store().stateRoot.resolve("DIRTY"), NOFOLLOW_LINKS)
}

private fun fixture21(): Fixture21 {
    val root = Files.createTempDirectory("bta-backend-21").toRealPath()
    val repo = Files.createDirectory(root.resolve("repo"))
    val index = Files.createDirectory(root.resolve("index"))
    val jdk = Files.createDirectory(root.resolve("jdk"))
    Files.writeString(jdk.resolve("release"), "JAVA_VERSION=21")
    val classpath = Files.writeString(root.resolve("classpath.jar"), "classpath")
    val friend = Files.writeString(root.resolve("friend.jar"), "friend")
    val plugin = Files.writeString(root.resolve("plugin.jar"), "plugin")
    val factsPlugin = Files.writeString(root.resolve("facts.jar"), "facts")
    return Fixture21(
        root,
        repo,
        index,
        jdk,
        classpath,
        friend,
        plugin,
        factsPlugin,
        Files.writeString(repo.resolve("A.kt"), "class A"),
        Files.writeString(repo.resolve("B.kt"), "class B"),
        Files.writeString(repo.resolve("C.kt"), "class C"),
        Files.writeString(repo.resolve("Empty.kt"), ""),
    )
}

private fun K2StoredGeneration21.objectHash21(path: String): String =
    files.single { it.path == path }.objectHash

private fun deleteTree21(root: Path) {
    if (!Files.exists(root, NOFOLLOW_LINKS)) return
    Files.walk(root).use { stream ->
        stream.iterator().asSequence().toList().sortedWith(Comparator.reverseOrder())
            .forEach { Files.deleteIfExists(it) }
    }
}

private val TEST_SEMANTIC_CONFIGURATION_DIGEST_21 = "sha256:" + "1".repeat(64)
