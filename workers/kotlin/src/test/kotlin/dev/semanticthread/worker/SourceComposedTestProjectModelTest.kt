package dev.semanticthread.worker

import java.nio.file.Files
import kotlin.io.path.createDirectories
import kotlin.io.path.writeText
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlinx.serialization.json.putJsonObject

class SourceComposedTestProjectModelTest {
    @Test
    fun testCompilationUsesMainSourcesWithoutRequiringAbsentBuildOutputs() {
        val repo = Files.createTempDirectory("worker-test-source-composition").toRealPath()
        try {
            val mainSource = repo.resolve("src/main/kotlin/p/Main.kt")
            val testSource = repo.resolve("src/test/kotlin/p/MainTest.kt")
            mainSource.parent.createDirectories()
            testSource.parent.createDirectories()
            mainSource.writeText("package p\ninternal fun answer() = 42\n")
            testSource.writeText("package p\nfun checksAnswer() = answer() == 42\n")
            val missingOutput = repo.resolve("build/classes/kotlin/main")
            val selected = buildJsonObject {
                putJsonArray("sourceFiles") { add(JsonPrimitive(testSource.toString())) }
                putJsonArray("friendPaths") { add(JsonPrimitive(missingOutput.toString())) }
                putJsonArray("buildModelBoundaries") {}
                putJsonObject("fieldBoundaries") { put("friendPaths", "AVAILABLE_ORDERED") }
            }
            val main = buildJsonObject {
                putJsonArray("sourceFiles") { add(JsonPrimitive(mainSource.toString())) }
            }

            Worker().use { worker ->
                var mainModelReads = 0
                val composed = worker.sourceComposedTestProjectModel(repo, ":app/test", selected) {
                    mainModelReads += 1
                    main
                }
                assertEquals(1, mainModelReads)
                val expectedSources = listOf(mainSource.toString(), testSource.toString())
                assertEquals(
                    expectedSources,
                    composed["sourceFiles"]?.jsonArray?.map { it.jsonPrimitive.content },
                )
                assertEquals(
                    expectedSources,
                    composed["analysisSourceFiles"]?.jsonArray?.map { it.jsonPrimitive.content },
                )
                assertTrue(composed["friendPaths"]?.jsonArray?.isEmpty() == true)
                assertEquals(
                    "SOURCE_COMPOSED_MAIN_SOURCES",
                    composed["fieldBoundaries"]?.jsonObject
                        ?.get("friendPaths")?.jsonPrimitive?.content,
                )
                assertTrue(
                    composed["buildModelBoundaries"]?.jsonArray
                        ?.map { it.jsonPrimitive.content }
                        ?.contains("KOTLIN_TEST_FRIEND_OUTPUT_UNAVAILABLE_SOURCE_COMPOSED") == true,
                )

                val alreadyComposed = buildJsonObject {
                    selected.forEach(::put)
                    putJsonArray("analysisSourceFiles") {
                        add(JsonPrimitive(mainSource.toString()))
                        add(JsonPrimitive(testSource.toString()))
                    }
                }
                worker.sourceComposedTestProjectModel(repo, ":app/test", alreadyComposed) {
                    error("an authoritative precomposed model must not be re-extracted")
                }
            }
        } finally {
            repo.toFile().deleteRecursively()
        }
    }
}
