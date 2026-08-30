package dev.semanticthread.worker

import java.nio.file.Files
import java.nio.file.Path
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNull

private fun directReceipt(file: String, schema: String = "fir-file-receipt/0.1"): JsonObject =
    buildJsonObject {
        put("recordType", "FIR_FILE_RECEIPT")
        put("schema", schema)
        put("file", file)
    }

private fun directFact(file: String): JsonObject = buildJsonObject {
    put("recordType", "SEMANTIC_FACT")
    put("file", file)
    put("kind", "TEST")
}

class DirectK2ReceiptClosureTest {
    private data class Fixture(
        val root: Path,
        val repo: Path,
        val a: Path,
        val b: Path,
        val overrideB: Path,
    ) : AutoCloseable {
        override fun close() {
            check(root.toFile().deleteRecursively()) { "failed to delete direct K2 receipt fixture" }
        }
    }

    private fun fixture(): Fixture {
        val root = Files.createTempDirectory("direct-k2-receipt")
        val repo = Files.createDirectories(root.resolve("repo")).toRealPath()
        val sources = Files.createDirectories(repo.resolve("src"))
        val a = Files.writeString(sources.resolve("A.kt"), "class A").toRealPath()
        val b = Files.writeString(sources.resolve("B.kt"), "class B").toRealPath()
        val overrideB = Files.writeString(root.resolve("override-B.kt"), "class B2").toRealPath()
        return Fixture(root, repo, a, b, overrideB)
    }

    private fun failure(
        fixture: Fixture,
        selected: List<Path>,
        compilerInputs: List<Path>,
        facts: List<JsonObject>,
    ): WorkerFailure = assertFailsWith<WorkerFailure> {
        validateDirectK2ReceiptClosure(fixture.repo, selected, compilerInputs, facts)
    }

    @Test
    fun acceptsOneExactReceiptPerSelectedSourceIncludingOverrideCompilerIdentity() = fixture().use { fx ->
        val facts = listOf(
            directReceipt("src/A.kt"),
            directReceipt(fx.overrideB.toString()),
            directFact(fx.overrideB.toString()),
        )

        assertEquals(
            facts,
            validateDirectK2ReceiptClosure(
                fx.repo,
                listOf(fx.a, fx.b),
                listOf(fx.a, fx.overrideB),
                facts,
            ),
        )
    }

    @Test
    fun rejectsMissingReceipt() = fixture().use { fx ->
        val result = failure(
            fx,
            listOf(fx.a, fx.b),
            listOf(fx.a, fx.b),
            listOf(directReceipt("src/A.kt")),
        )

        assertEquals("INCOMPLETE_SEMANTIC_ANALYSIS", result.code)
        assertEquals("direct K2 FIR file receipt closure is incomplete", result.message)
    }

    @Test
    fun rejectsDuplicateReceiptEvenWhenIdentitiesUseRelativeAndAbsoluteForms() = fixture().use { fx ->
        val result = failure(
            fx,
            listOf(fx.a),
            listOf(fx.a),
            listOf(directReceipt("src/A.kt"), directReceipt(fx.a.toString())),
        )

        assertEquals("INCOMPLETE_SEMANTIC_ANALYSIS", result.code)
        assertEquals("direct K2 FIR file receipt is duplicated", result.message)
    }

    @Test
    fun rejectsMalformedReceiptSchemaAndShape() = fixture().use { fx ->
        val wrongSchema = failure(
            fx,
            listOf(fx.a),
            listOf(fx.a),
            listOf(directReceipt("src/A.kt", schema = "fir-file-receipt/9.9")),
        )
        val extraField = failure(
            fx,
            listOf(fx.a),
            listOf(fx.a),
            listOf(buildJsonObject {
                directReceipt("src/A.kt").forEach { (key, value) -> put(key, value) }
                put("untrusted", true)
            }),
        )

        assertEquals("direct K2 FIR file receipt is malformed", wrongSchema.message)
        assertEquals("direct K2 FIR file receipt is malformed", extraField.message)
    }

    @Test
    fun rejectsOutOfScopeAndNoncanonicalReceiptIdentities() = fixture().use { fx ->
        val outOfScope = failure(
            fx,
            listOf(fx.a),
            listOf(fx.a),
            listOf(directReceipt("src/B.kt")),
        )
        val aliased = failure(
            fx,
            listOf(fx.a),
            listOf(fx.a),
            listOf(directReceipt("./src/A.kt")),
        )

        assertEquals("direct K2 FIR file receipt is outside the selected source set", outOfScope.message)
        assertEquals("direct K2 FIR file receipt has a noncanonical source identity", aliased.message)
    }

    @Test
    fun rejectsFileBoundFactOutsideSelectedSources() = fixture().use { fx ->
        val result = failure(
            fx,
            listOf(fx.a),
            listOf(fx.a),
            listOf(directReceipt("src/A.kt"), directFact("src/B.kt")),
        )

        assertEquals("INCOMPLETE_SEMANTIC_ANALYSIS", result.code)
        assertEquals("direct K2 compiler fact is outside the selected source set", result.message)
    }

    @Test
    fun rejectsFileBoundFactWithoutItsReceipt() = fixture().use { fx ->
        val result = failure(
            fx,
            listOf(fx.a, fx.b),
            listOf(fx.a, fx.b),
            listOf(directReceipt("src/B.kt"), directFact("src/A.kt")),
        )

        assertEquals("INCOMPLETE_SEMANTIC_ANALYSIS", result.code)
        assertEquals("direct K2 compiler fact has no admitted FIR file receipt", result.message)
    }

    @Test
    fun emptyCompilerFactsProduceAnExplicitPartialityBoundary() {
        val boundary = requireNotNull(missingCompilerFactsRelationBoundary(emptyList()))

        assertEquals("NO_COMPILER_FACTS", boundary["code"].toString().trim('"'))
        assertEquals("UNKNOWN", boundary["resolution"].toString().trim('"'))
        assertNull(missingCompilerFactsRelationBoundary(listOf(directReceipt("src/A.kt"))))
    }
}
