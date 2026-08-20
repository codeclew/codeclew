package dev.semanticthread.worker

import dev.semanticthread.worker.IncrementalK2Runtime
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

class IncrementalK2RuntimeTest {
    private data class ProfileCase(
        val status: IncrementalK2Status,
        val valid: Boolean,
        val totalMicros: Long,
        val compilerMicros: Long,
        val firExtractionMicros: Long,
        val totalFiles: Int,
        val compiledFiles: Int,
        val reusedFiles: Int,
        val recovered: Boolean,
        val fallbackUsed: Boolean,
        val graphDigest: String?,
    )

    private val cases = listOf(
        ProfileCase(IncrementalK2Status.COLD_FULL, true, 100, 60, 20, 3, 3, 0, false, false, "a".repeat(64)),
        ProfileCase(IncrementalK2Status.INCREMENTAL, true, 120, 70, 30, 4, 1, 3, false, false, "b".repeat(64)),
        ProfileCase(IncrementalK2Status.RECOVERED_FULL, true, 130, 80, 40, 2, 2, 0, true, false, "c".repeat(64)),
        ProfileCase(IncrementalK2Status.UNCHANGED_HIT, true, 20, 0, 0, 4, 0, 4, false, false, "d".repeat(64)),
        ProfileCase(IncrementalK2Status.COLD_FULL, false, 100, 60, 20, 3, 0, 0, false, false, null),
        ProfileCase(IncrementalK2Status.INCREMENTAL, false, 120, 70, 30, 4, 0, 0, false, false, null),
        ProfileCase(IncrementalK2Status.RECOVERED_FULL, false, 130, 80, 40, 2, 0, 0, true, false, null),
        ProfileCase(IncrementalK2Status.BUSY, false, 0, 0, 0, 0, 0, 0, false, true, null),
        ProfileCase(IncrementalK2Status.FAILED_RECOVERABLE, false, 100, 60, 20, 5, 0, 0, false, true, null),
    )

    private val baseKeys = setOf(
        "backend",
        "status",
        "valid",
        "totalMicros",
        "compilerMicros",
        "firExtractionMicros",
        "totalFiles",
        "compiledFiles",
        "reusedFiles",
        "recovered",
        "fallbackUsed",
    )

    @Test
    fun `records the exact parser-valid status matrix`() {
        try {
            cases.forEach { case ->
                IncrementalK2Runtime.reset()
                IncrementalK2Runtime.record(result(case), case.fallbackUsed)
                val profile = assertNotNull(IncrementalK2Runtime.takeProfiling())

                assertEquals(baseKeys + listOfNotNull(case.graphDigest?.let { "graphDigest" }), profile.keys)
                assertEquals(JsonPrimitive("BTA_PERSISTENT"), profile["backend"])
                assertEquals(JsonPrimitive(case.status.name), profile["status"])
                assertEquals(JsonPrimitive(case.valid), profile["valid"])
                assertEquals(JsonPrimitive(case.totalMicros), profile["totalMicros"])
                assertEquals(JsonPrimitive(case.compilerMicros), profile["compilerMicros"])
                assertEquals(JsonPrimitive(case.firExtractionMicros), profile["firExtractionMicros"])
                assertEquals(JsonPrimitive(case.totalFiles), profile["totalFiles"])
                assertEquals(JsonPrimitive(case.compiledFiles), profile["compiledFiles"])
                assertEquals(JsonPrimitive(case.reusedFiles), profile["reusedFiles"])
                assertEquals(JsonPrimitive(case.recovered), profile["recovered"])
                assertEquals(JsonPrimitive(case.fallbackUsed), profile["fallbackUsed"])
                if (case.graphDigest == null) {
                    assertNull(profile["graphDigest"])
                } else {
                    assertEquals(JsonPrimitive(case.graphDigest), profile["graphDigest"])
                }
            }
        } finally {
            IncrementalK2Runtime.reset()
        }
    }

    @Test
    fun `take is one shot and reset clears the current thread`() {
        try {
            IncrementalK2Runtime.reset()
            IncrementalK2Runtime.record(result(cases.first()), false)
            assertNotNull(IncrementalK2Runtime.takeProfiling())
            assertNull(IncrementalK2Runtime.takeProfiling())

            IncrementalK2Runtime.record(result(cases.first()), false)
            IncrementalK2Runtime.reset()
            assertNull(IncrementalK2Runtime.takeProfiling())
        } finally {
            IncrementalK2Runtime.reset()
        }
    }

    @Test
    fun `profiling state is isolated per thread`() {
        try {
            IncrementalK2Runtime.reset()
            IncrementalK2Runtime.record(result(cases.first()), false)
            var childBefore: JsonObject? = null
            var childProfile: JsonObject? = null
            val thread = Thread {
                childBefore = IncrementalK2Runtime.takeProfiling()
                val busy = cases.first { it.status == IncrementalK2Status.BUSY }
                IncrementalK2Runtime.record(result(busy), true)
                childProfile = IncrementalK2Runtime.takeProfiling()
            }

            thread.start()
            thread.join()

            assertNull(childBefore)
            assertEquals(JsonPrimitive(IncrementalK2Status.BUSY.name), childProfile?.get("status"))
            assertEquals(JsonPrimitive(IncrementalK2Status.COLD_FULL.name), IncrementalK2Runtime.takeProfiling()?.get("status"))
            assertNull(IncrementalK2Runtime.takeProfiling())
        } finally {
            IncrementalK2Runtime.reset()
        }
    }

    @Test
fun `merge preserves semantic fields and combines profiling purely`() {
        val response = buildJsonObject {
            put("semanticFactsDigest", "facts")
            put("pathFactSetDigest", "paths")
            put("profiling", buildJsonObject {
                put("elapsedMicros", 7)
                put("status", "LEGACY")
            })
        }
        IncrementalK2Runtime.reset()
        IncrementalK2Runtime.recordProjectModel(
            status = "PERSISTENT_HIT",
            totalMicros = 17,
            keyMicros = 4,
            loadMicros = 11,
            extractionMicros = 0,
            publishMicros = 0,
            persistentConfigured = true,
            published = false,
        )
        val projectModel = assertNotNull(IncrementalK2Runtime.takeProfiling())
        val withProjectModel = IncrementalK2Runtime.mergeProfiling(response, projectModel)
        val incremental = buildJsonObject {
            put("status", "INCREMENTAL")
            put("valid", true)
        }

        val merged = IncrementalK2Runtime.mergeProfiling(withProjectModel, incremental)
        val mergedProfiling = assertNotNull(merged["profiling"] as? JsonObject)

        assertEquals(response["semanticFactsDigest"], merged["semanticFactsDigest"])
        assertEquals(response["pathFactSetDigest"], merged["pathFactSetDigest"])
        assertEquals(JsonPrimitive(7), mergedProfiling["elapsedMicros"])
        assertEquals(JsonPrimitive("INCREMENTAL"), mergedProfiling["status"])
        assertEquals(JsonPrimitive(true), mergedProfiling["valid"])
        assertEquals(JsonPrimitive("PERSISTENT_HIT"), mergedProfiling["projectModelCacheStatus"])
        assertEquals(JsonPrimitive(17), mergedProfiling["projectModelTotalMicros"])
        assertEquals(JsonPrimitive(4), mergedProfiling["projectModelKeyMicros"])
        assertEquals(JsonPrimitive(11), mergedProfiling["projectModelLoadMicros"])
        assertEquals(JsonPrimitive(true), mergedProfiling["projectModelPersistentConfigured"])
        assertEquals(JsonPrimitive(false), mergedProfiling["projectModelPublished"])
        assertEquals(JsonPrimitive("LEGACY"), (response["profiling"] as JsonObject)["status"])
        assertEquals(response, IncrementalK2Runtime.mergeProfiling(response, null))
    }

    private fun result(case: ProfileCase) = IncrementalK2Result(
        valid = case.valid,
        facts = emptyList(),
        diagnostics = emptyList(),
        status = case.status,
        totalMicros = case.totalMicros,
        compilerMicros = case.compilerMicros,
        firExtractionMicros = case.firExtractionMicros,
        totalFiles = case.totalFiles,
        compiledFiles = case.compiledFiles,
        reusedFiles = case.reusedFiles,
        recovered = case.recovered,
        graphDigest = case.graphDigest,
    )
}