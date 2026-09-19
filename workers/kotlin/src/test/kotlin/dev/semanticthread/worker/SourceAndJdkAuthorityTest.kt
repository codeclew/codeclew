package dev.semanticthread.worker

import java.nio.file.Files
import java.nio.file.Path
import kotlin.io.path.createDirectories
import kotlin.io.path.writeText
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNotEquals
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.putJsonArray

class SourceAndJdkAuthorityTest {
    @Test
    fun sourceMembershipAndBytesAreBoundForIndexAndAnalysis() {
        val repo = Files.createTempDirectory("worker-source-authority").toRealPath()
        try {
            val a = repo.resolve("src/main/kotlin/p/A.kt").also { it.parent.createDirectories(); it.writeText("package p\nclass A\n") }
            val b = repo.resolve("src/main/kotlin/p/B.kt").also { it.writeText("package p\nclass B\n") }
            val indexA = sourceSelectionAuthority(repo, model(listOf(a), listOf(a)))
            val indexAB = sourceSelectionAuthority(repo, model(listOf(a, b), listOf(a, b)))
            assertNotEquals(sourceDigest(indexA, "sourceFiles"), sourceDigest(indexAB, "sourceFiles"))
            assertNotEquals(sourceDigest(indexA, "analysisSourceFiles"), sourceDigest(indexAB, "analysisSourceFiles"))
            assertEquals(indexA, sourceSelectionAuthority(repo, model(listOf(a), listOf(a))))

            a.writeText("package p\nclass A2\n")
            val changed = sourceSelectionAuthority(repo, model(listOf(a), listOf(a)))
            assertNotEquals(sourceDigest(indexA, "sourceFiles"), sourceDigest(changed, "sourceFiles"))
        } finally {
            repo.toFile().deleteRecursively()
        }
    }

    @Test
    fun analysisOnlySelectionDeltaIsBoundSeparately() {
        val repo = Files.createTempDirectory("worker-analysis-authority").toRealPath()
        try {
            val a = repo.resolve("A.kt").also { it.writeText("class A") }
            val b = repo.resolve("B.kt").also { it.writeText("class B") }
            val selected = sourceSelectionAuthority(repo, model(listOf(a), listOf(a)))
            val analysisExpanded = sourceSelectionAuthority(repo, model(listOf(a), listOf(a, b)))
            assertEquals(sourceDigest(selected, "sourceFiles"), sourceDigest(analysisExpanded, "sourceFiles"))
            assertNotEquals(
                sourceDigest(selected, "analysisSourceFiles"),
                sourceDigest(analysisExpanded, "analysisSourceFiles"),
            )
        } finally {
            repo.toFile().deleteRecursively()
        }
    }

    @Test
    fun sourceAuthorityRejectsMissingAndUnsafeFiles() {
        val repo = Files.createTempDirectory("worker-source-authority-reject").toRealPath()
        val outside = Files.createTempFile("worker-source-authority-outside", ".kt").toRealPath()
        try {
            val inside = repo.resolve("A.kt").also { it.writeText("class A") }
            assertFailsWith<WorkerFailure> {
                sourceSelectionAuthority(repo, model(listOf(repo.resolve("missing.kt")), listOf(inside)))
            }
            assertFailsWith<WorkerFailure> {
                sourceSelectionAuthority(repo, model(listOf(outside), listOf(inside)))
            }
            val link = repo.resolve("Linked.kt")
            Files.createSymbolicLink(link, outside)
            assertFailsWith<WorkerFailure> {
                sourceSelectionAuthority(repo, model(listOf(link), listOf(link)))
            }
        } finally {
            repo.toFile().deleteRecursively()
            Files.deleteIfExists(outside)
        }
    }

    @Test
    fun jdkFingerprintBindsRuntimeImageAndApiSignatures() {
        val jdk = fakeJdk()
        try {
            val baseline = jdkFingerprint(jdk)
            assertEquals(baseline, jdkFingerprint(jdk))

            jdk.resolve("lib/modules").writeText("modules-v2")
            val changedModules = jdkFingerprint(jdk)
            assertNotEquals(baseline, changedModules)

            jdk.resolve("lib/ct.sym").writeText("ct-sym-v1")
            val appearedCtSym = jdkFingerprint(jdk)
            assertNotEquals(changedModules, appearedCtSym)
            jdk.resolve("lib/ct.sym").writeText("ct-sym-v2")
            assertNotEquals(appearedCtSym, jdkFingerprint(jdk))
        } finally {
            jdk.toFile().deleteRecursively()
        }
    }

    @Test
    fun jdkFingerprintRequiresARealRuntimeImage() {
        val jdk = Files.createTempDirectory("worker-jdk-missing-image").toRealPath()
        try {
            jdk.resolve("bin").createDirectories()
            jdk.resolve("release").writeText("JAVA_VERSION=\"21\"\n")
            jdk.resolve("bin/java").writeText("launcher")
            assertFailsWith<WorkerFailure> { jdkFingerprint(jdk) }
        } finally {
            jdk.toFile().deleteRecursively()
        }
    }

    private fun model(sourceFiles: List<Path>, analysisSourceFiles: List<Path>) = buildJsonObject {
        putJsonArray("sourceFiles") { sourceFiles.forEach { add(JsonPrimitive(it.toString())) } }
        putJsonArray("analysisSourceFiles") { analysisSourceFiles.forEach { add(JsonPrimitive(it.toString())) } }
    }

    private fun sourceDigest(authority: kotlinx.serialization.json.JsonObject, key: String): String =
        authority[key]!!.jsonObject["digest"]!!.jsonPrimitive.content

    private fun fakeJdk(): Path = Files.createTempDirectory("worker-jdk-authority").toRealPath().also {
        it.resolve("bin").createDirectories()
        it.resolve("lib").createDirectories()
        it.resolve("release").writeText("JAVA_VERSION=\"21\"\n")
        it.resolve("bin/java").writeText("launcher")
        it.resolve("lib/modules").writeText("modules-v1")
    }
}
