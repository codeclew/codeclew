package dev.semanticthread.worker

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import java.nio.file.Files
import java.nio.file.Path
import kotlin.io.path.invariantSeparatorsPathString
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertIs
import kotlin.test.assertNotEquals
import kotlin.test.assertTrue

private fun receipt(path: String): String = buildJsonObject {
    put("recordType", "FIR_FILE_RECEIPT")
    put("schema", "fir-file-receipt/0.1")
    put("file", path)
}.toString()

private fun fact(path: String, value: String): String = buildJsonObject {
    put("recordType", "SEMANTIC_FACT")
    put("file", path)
    put("kind", "TEST")
    put("value", value)
}.toString()

class K2FactGenerationStore21Test {
    private data class Fixture(
        val root: Path,
        val repo: Path,
        val store: K2FactGenerationStore21,
        val a: Path,
        val b: Path,
        val c: Path,
        val empty: Path,
    ) {
        fun rows(vararg sources: Path): List<String> = sources.flatMap { source ->
            val relative = repo.relativize(source).invariantSeparatorsPathString
            buildList {
                add(receipt(relative))
                if (source.fileName.toString() != "Empty.kt") {
                    add(fact(relative, Files.readString(source)))
                }
            }
        }
    }

    private fun fixture(hooks: K2StoreHooks21 = K2StoreHooks21()): Fixture {
        val root = Files.createTempDirectory("k2-store-21").toRealPath()
        val repo = Files.createDirectories(root.resolve("repo"))
        val index = Files.createDirectories(root.resolve("index"))
        val a = Files.writeString(repo.resolve("A.kt"), "class A")
        val b = Files.writeString(repo.resolve("B.kt"), "class B")
        val c = Files.writeString(repo.resolve("C.kt"), "class C")
        val empty = Files.writeString(repo.resolve("Empty.kt"), "")
        return Fixture(
            root,
            repo,
            K2FactGenerationStore21.open(index, repo, ":workers:kotlin21/main", "2.1.21", "cfg", hooks),
            a,
            b,
            c,
            empty,
        )
    }

    @Suppress("UNCHECKED_CAST")
    private fun <T> acquired(result: K2StoreLockResult21<T>): T =
        (result as K2StoreLockResult21.Acquired<T>).value

    private fun publish(fx: Fixture, sources: List<Path>, lines: List<String>): K2PublishResult21 =
        acquired(fx.store.withLock {
            val snapshot = snapshot(sources)
            beginDirty(snapshot, loadCurrent()).publishCompiledGeneration(lines)
        })

    private fun currentBytes(fx: Fixture): ByteArray = Files.readAllBytes(fx.store.stateRoot.resolve("CURRENT"))

    @Test
    fun fullGenerationIncludesReceiptOnlyEmptyShard() {
        val fx = fixture()
        val published = publish(fx, listOf(fx.a, fx.empty), fx.rows(fx.a, fx.empty))
        assertTrue(published.committed)
        assertFalse(published.cleanupPending)
        assertEquals(listOf("A.kt", "Empty.kt"), published.generation.files.map { it.path })
        assertEquals(0, published.generation.files.single { it.path == "Empty.kt" }.factCount)
        assertEquals(listOf("class A"), published.generation.facts.map { it["value"].toString().trim('"') })
        assertFalse(Files.exists(fx.store.stateRoot.resolve("DIRTY")))
    }

    @Test
    fun incrementalReplaceDeleteAndUnchangedObjectReuse() {
        val fx = fixture()
        val first = publish(
            fx, listOf(fx.a, fx.b, fx.c, fx.empty), fx.rows(fx.a, fx.b, fx.c, fx.empty),
        ).generation
        val emptyObject = first.files.single { it.path == "Empty.kt" }.objectHash
        val cObject = first.files.single { it.path == "C.kt" }.objectHash
        Files.writeString(fx.a, "class A2")
        Files.delete(fx.b)
        val delta = acquired(fx.store.withLock {
            delta(snapshot(listOf(fx.a, fx.c, fx.empty)), loadCurrent())
        })
        assertEquals(setOf("A.kt"), delta.addedOrModified)
        assertEquals(setOf("B.kt"), delta.removed)
        assertEquals(setOf("C.kt", "Empty.kt"), delta.unchanged)
        val dependentRows = fx.rows(fx.a) + listOf(receipt("Empty.kt"), fact("Empty.kt", "dependent"))
        val second = publish(fx, listOf(fx.a, fx.c, fx.empty), dependentRows).generation
        assertEquals(listOf("A.kt", "C.kt", "Empty.kt"), second.files.map { it.path })
        assertEquals(cObject, second.files.single { it.path == "C.kt" }.objectHash)
        assertNotEquals(emptyObject, second.files.single { it.path == "Empty.kt" }.objectHash)
        assertNotEquals(first.graphDigest, second.graphDigest)
        val unchanged = acquired(fx.store.withLock {
            delta(snapshot(listOf(fx.a, fx.c, fx.empty)), loadCurrent())
        })
        assertTrue(unchanged.isEmpty)
    }

    @Test
    fun invalidReceiptsNeverChangeCurrent() {
        listOf<(Fixture) -> List<String>>(
            { listOf("{bad") },
            { listOf(fact("A.kt", "changed")) },
            { listOf(receipt("Foreign.kt")) },
            { listOf(receipt("A.kt"), fact("A.kt", "changed"), fact("A.kt", "changed")) },
        ).forEach { invalid ->
            val fx = fixture()
            publish(fx, listOf(fx.a), fx.rows(fx.a))
            Files.writeString(fx.a, "changed")
            val before = currentBytes(fx)
            assertFailsWith<Exception> { publish(fx, listOf(fx.a), invalid(fx)) }
            assertTrue(before.contentEquals(currentBytes(fx)))
            assertTrue(Files.exists(fx.store.stateRoot.resolve("DIRTY")))
        }
    }

    @Test
    fun corruptStateAndDirtyNeverReturnStaleAndCanRecoverFull() {
        val fx = fixture()
        val initial = publish(fx, listOf(fx.a, fx.empty), fx.rows(fx.a, fx.empty)).generation
        val shard = initial.files.single { it.path == "A.kt" }.objectHash
        Files.writeString(fx.store.stateRoot.resolve("objects/$shard.json"), "corrupt")
        assertIs<K2Current21.RecoveryRequired>(acquired(fx.store.withLock { loadCurrent() }))
        Files.writeString(fx.store.stateRoot.resolve("DIRTY"), "dirty\n")
        val recovered = acquired(fx.store.withLock {
            val snapshot = snapshot(listOf(fx.a, fx.empty))
            val current = loadCurrent()
            assertIs<K2Current21.RecoveryRequired>(current)
            val txn = beginDirty(snapshot, current)
            assertTrue(txn.full && txn.recovered)
            txn.publishCompiledGeneration(fx.rows(fx.a, fx.empty))
        })
        assertEquals(initial.graphDigest, recovered.generation.graphDigest)
        assertFalse(recovered.cleanupPending)
        assertTrue(Files.list(fx.store.stateRoot.resolve("quarantine")).use { it.findAny().isPresent })
        Files.writeString(fx.store.stateRoot.resolve("CURRENT"), "not-a-digest\n")
        Files.writeString(fx.store.stateRoot.resolve("DIRTY"), "dirty\n")
        assertIs<K2Current21.RecoveryRequired>(acquired(fx.store.withLock { loadCurrent() }))
    }

    @Test
    fun snapshotLockAndCommitFailureBoundariesAreExplicit() {
        val mutation = fixture()
        acquired(mutation.store.withLock {
            val snapshot = snapshot(listOf(mutation.a))
            val txn = beginDirty(snapshot, loadCurrent())
            Files.writeString(mutation.a, "mutated")
            assertFailsWith<IllegalArgumentException> {
                txn.publishCompiledGeneration(listOf(receipt("A.kt"), fact("A.kt", "class A")))
            }
            assertIs<K2StoreLockResult21.Busy>(mutation.store.withLock { loadCurrent() })
        })
        assertFalse(Files.exists(mutation.store.stateRoot.resolve("CURRENT")))
        assertTrue(Files.exists(mutation.store.stateRoot.resolve("DIRTY")))

        var failCommit = false
        val precommit = fixture(K2StoreHooks21(beforeCommit = { if (failCommit) error("precommit") }))
        publish(precommit, listOf(precommit.a), precommit.rows(precommit.a))
        Files.writeString(precommit.a, "next")
        val before = currentBytes(precommit)
        failCommit = true
        assertFailsWith<IllegalStateException> {
            publish(precommit, listOf(precommit.a), precommit.rows(precommit.a))
        }
        assertTrue(before.contentEquals(currentBytes(precommit)))
        assertTrue(Files.exists(precommit.store.stateRoot.resolve("DIRTY")))

        var failCleanup = false
        val cleanup = fixture(K2StoreHooks21(beforeCleanup = { if (failCleanup) error("cleanup") }))
        publish(cleanup, listOf(cleanup.a), cleanup.rows(cleanup.a))
        val oldCurrent = currentBytes(cleanup)
        Files.writeString(cleanup.a, "next")
        failCleanup = true
        val committed = publish(cleanup, listOf(cleanup.a), cleanup.rows(cleanup.a))
        assertTrue(committed.committed && committed.cleanupPending)
        assertFalse(oldCurrent.contentEquals(currentBytes(cleanup)))
        assertTrue(Files.exists(cleanup.store.stateRoot.resolve("DIRTY")))
        assertIs<K2Current21.RecoveryRequired>(acquired(cleanup.store.withLock { loadCurrent() }))
    }

    @Test
    fun snapshotAndStatePathsRejectAlternateOrSymlinkedSpelling() {
        val fx = fixture()
        val outside = Files.writeString(fx.root.resolve("Outside.kt"), "outside")
        val alias = fx.repo.resolve("Alias.kt")
        Files.createSymbolicLink(alias, fx.a)
        val directory = Files.createDirectories(fx.repo.resolve("Directory.kt"))
        val alternate = Path.of(fx.repo.toString(), ".", "A.kt")
        val traversal = fx.repo.resolve("nested/../A.kt")
        listOf(Path.of("A.kt"), alternate, traversal, outside, alias, directory).forEach { candidate ->
            acquired(fx.store.withLock {
                assertFailsWith<IllegalArgumentException> { snapshot(listOf(candidate)) }
            })
        }
        acquired(fx.store.withLock {
            assertFailsWith<IllegalArgumentException> { snapshot(listOf(fx.a, fx.a)) }
        })

        val state = fixture()
        val outsideState = Files.createDirectories(state.root.resolve("outside-state"))
        Files.createSymbolicLink(state.store.stateRoot.resolve("objects"), outsideState)
        acquired(state.store.withLock {
            val txn = beginDirty(snapshot(listOf(state.a)), loadCurrent())
            assertFailsWith<IllegalArgumentException> {
                txn.publishCompiledGeneration(state.rows(state.a))
            }
        })
        assertFalse(Files.exists(state.store.stateRoot.resolve("CURRENT")))
        assertTrue(Files.exists(state.store.stateRoot.resolve("DIRTY")))
        assertEquals(0L, Files.list(outsideState).use { it.count() })
    }
}
