package dev.semanticthread.worker

import java.nio.file.Path
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertSame

class CompilerFactIndexTest {
    private val repo = Path.of("/compiler-index-fixture")

    private fun fact(file: String, start: Int, end: Int, id: String, kind: String = "SEMANTIC_FACT") =
        buildJsonObject {
            put("recordType", kind)
            put("file", file)
            put("start", start)
            put("end", end)
            put("id", id)
        }

    @Test
    fun exactPathsDoNotMixSuffixesAndRejectEscapes() {
        val local = fact("src/A.kt", 0, 10, "local")
        val nested = fact("module/src/A.kt", 0, 10, "nested")
        val absolute = fact("$repo/src/A.kt", 11, 12, "absolute")
        val windows = fact("src\\A.kt", 13, 14, "separator")
        val index = CompilerFactIndex(repo, listOf(
            local, nested, absolute, windows,
            fact("../src/A.kt", 0, 1, "escape"),
            fact("/elsewhere/src/A.kt", 0, 1, "outside"),
        ))
        assertEquals(listOf(local, absolute, windows), index.semanticFacts(repo.resolve("src/A.kt")))
        assertEquals(listOf(nested), index.semanticFacts(repo.resolve("module/src/A.kt")))
        assertEquals(emptyList(), index.semanticFacts(repo.resolve("missing.kt")))
    }

    @Test
    fun sortingAndContainmentMatchThePreviousPredicate() {
        val rows = listOf(
            fact("A.kt", 10, 50, "outer"), fact("A.kt", 12, 20, "z"),
            fact("A.kt", 12, 20, "a"), fact("A.kt", 25, 27, "inner"),
            fact("A.kt", 100, -1, "malformed-end"), fact("A.kt", -1, -1, "unknown"),
        )
        val ordered = rows.sortedWith(compareBy(
            { it["start"]?.jsonPrimitive?.intOrNull ?: -1 },
            { it["end"]?.jsonPrimitive?.intOrNull ?: -1 }, { it.toString() },
        ))
        val index = CompilerFactIndex(repo, rows.reversed())
        val path = repo.resolve("A.kt")
        assertEquals(ordered, index.semanticFacts(path))
        assertSame(index.semanticFacts(path), index.semanticFacts(path))
        for ((start, end) in listOf(0 to 100, 10 to 50, 12 to 20, 26 to 26, 200 to 300)) {
            assertEquals(ordered.filter {
                (it["start"]?.jsonPrimitive?.intOrNull ?: -1) >= start &&
                    (it["end"]?.jsonPrimitive?.intOrNull ?: -1) <= end
            }, index.containedSemanticFacts(path, start, end))
        }
    }

    @Test
    fun cfgLookupPreservesFirstDuplicateAndArrivalOrder() {
        val first = fact("A.kt", 1, 5, "first", "FIR_CFG")
        val duplicate = fact("A.kt", 1, 5, "duplicate", "FIR_CFG")
        val earlier = fact("A.kt", 0, 1, "earlier", "FIR_CFG")
        val index = CompilerFactIndex(repo, listOf(first, duplicate, earlier))
        assertEquals(listOf(first, duplicate, earlier), index.cfgRecords(repo.resolve("A.kt")))
        assertEquals(first, index.cfgAt(repo.resolve("A.kt"), 1, 5))
        assertNull(index.cfgAt(repo.resolve("A.kt"), 1, 6))
    }

    @Test
    fun indexedLookupsMatchFullScansAcrossManyFiles() {
        val rows = (0 until 100).flatMap { file ->
            (0 until 40).map { offset -> fact("src/File$file.kt", offset, offset + 1, "$offset") }
        }.reversed()
        val index = CompilerFactIndex(repo, rows)
        for (file in 0 until 100) {
            val expected = rows.filter { it["file"]!!.jsonPrimitive.content == "src/File$file.kt" }
                .sortedWith(compareBy<JsonObject> { it["start"]!!.jsonPrimitive.intOrNull })
            assertEquals(expected, index.semanticFacts(repo.resolve("src/File$file.kt")))
        }
    }
}
