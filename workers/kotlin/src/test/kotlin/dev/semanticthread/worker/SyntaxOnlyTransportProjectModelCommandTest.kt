package dev.semanticthread.worker

import dev.semanticthread.worker.syntaxOnlyIndexSourceFiles
import java.nio.file.Files
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse

class SyntaxOnlyTransportProjectModelCommandTest {
    @Test
fun syntaxOnlyIndexSourceDiscoveryDoesNotRequireGradleOrMavenModel() {
        val root = Files.createTempDirectory("syntax-only-index").toRealPath()
        val outside = Files.createTempDirectory("syntax-only-outside").toRealPath()
        val linked = root.resolve("src/main/kotlin/sample/Linked.kt")
        val escape = root.resolve("src/main/kotlin/escape")
        val unrelatedLink = root.resolve("unrelated-link")
        try {
            val first = root.resolve("src/main/kotlin/sample/First.kt")
            val second = root.resolve("src/main/kotlin/sample/Second.kt")
            Files.createDirectories(first.parent)
            Files.writeString(first, "package sample; class First")
            Files.writeString(second, "package sample; class Second")
            val large = root.resolve("unrelated/cache")
            Files.createDirectories(large)
            repeat(256) { Files.writeString(large.resolve("$it.bin"), "ignored") }
            val outsideSource = outside.resolve("Outside.kt")
            Files.writeString(outsideSource, "class Outside")
            Files.createSymbolicLink(unrelatedLink, outside)

            assertEquals(
                listOf(first.toRealPath()),
                syntaxOnlyIndexSourceFiles(root, listOf("src/main/kotlin/sample/First.kt")),
            )
            assertEquals(0, syntaxOnlyIndexSourceFiles(root, emptyList()).size)

            val assertRejected: (List<String>) -> Unit = { requested ->
                var rejected = false
                try {
                    syntaxOnlyIndexSourceFiles(root, requested)
                } catch (_: IllegalArgumentException) {
                    rejected = true
                }
                assertEquals(true, rejected)
            }

            Files.createSymbolicLink(linked, first.fileName)
            Files.createSymbolicLink(escape, outside)
            Files.createDirectories(root.resolve("src/main/kotlin/sample/Directory.kt"))
            Files.writeString(root.resolve("src/main/kotlin/sample/Script.kts"), "class Script")
            assertRejected(listOf(first.toString()))
            assertRejected(listOf("src/main/kotlin/sample/../sample/First.kt"))
            assertRejected(listOf("../Outside.kt"))
            assertRejected(listOf("src/main/kotlin/sample/First.kt", "src/main/kotlin/sample/First.kt"))
            assertRejected(listOf("src/main/kotlin/sample/Missing.kt"))
            assertRejected(listOf("src/main/kotlin/sample/Linked.kt"))
            assertRejected(listOf("src/main/kotlin/escape/Outside.kt"))
            assertRejected(listOf("src/main/kotlin/sample/Directory.kt"))
            assertRejected(listOf("src/main/kotlin/sample/Script.kts"))
            assertFalse(Files.exists(root.resolve("build.gradle")))
            assertFalse(Files.exists(root.resolve("build.gradle.kts")))
            assertFalse(Files.exists(root.resolve("pom.xml")))
        } finally {
            Files.deleteIfExists(linked)
            Files.deleteIfExists(escape)
            Files.deleteIfExists(unrelatedLink)
            root.toFile().deleteRecursively()
            outside.toFile().deleteRecursively()
        }
    }
}
