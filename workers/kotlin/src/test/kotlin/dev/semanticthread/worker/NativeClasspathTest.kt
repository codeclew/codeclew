package dev.semanticthread.worker

import java.nio.file.Files
import kotlin.io.path.createTempDirectory
import kotlin.io.path.deleteIfExists
import kotlin.io.path.writeText
import kotlin.test.Test
import kotlin.test.assertFails
import kotlin.test.assertTrue
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

class NativeClasspathTest {
    @Test
    fun sealedAbsenceBindsPositionPathProducerAndPhysicalState() {
        val temporary = createTempDirectory("selected-classpath")
        val repo = temporary.toRealPath()
        val output = repo.resolve("derived/survey/classes")
        val slot = buildJsonObject {
            put("kind", "ABSENT_DECLARED_DIRECTORY")
            put("index", 2)
            put("path", "derived/survey/classes")
            put("producerTask", ":survey")
            put("outputProperty", "output")
        }
        val compiler = JsonPrimitive(":compileKotlin")
        try {
            assertTrue(absentNativeClasspathIdentity(repo, output, 2, compiler, slot)
                .startsWith("repo:derived/survey/classes:absent-directory:sha256:"))
            assertFails { absentNativeClasspathIdentity(repo, output, 1, compiler, slot) }
            assertFails { absentNativeClasspathIdentity(repo, output.resolveSibling("other"), 2, compiler, slot) }
            assertFails { absentNativeClasspathIdentity(repo, output, 2, JsonPrimitive(":survey"), slot) }
            Files.createDirectories(output)
            assertFails { absentNativeClasspathIdentity(repo, output, 2, compiler, slot) }
            output.deleteIfExists()
            output.writeText("file instead of the directory")
            assertFails { absentNativeClasspathIdentity(repo, output, 2, compiler, slot) }
            output.deleteIfExists()
            Files.createSymbolicLink(output, repo.resolve("missing"))
            assertFails { absentNativeClasspathIdentity(repo, output, 2, compiler, slot) }
            Files.delete(output)
            Files.delete(output.parent)
            Files.createSymbolicLink(output.parent, repo.resolve("absent-parent"))
            assertFails { absentNativeClasspathIdentity(repo, output, 2, compiler, slot) }
        } finally {
            repo.toFile().deleteRecursively()
        }
    }
}
