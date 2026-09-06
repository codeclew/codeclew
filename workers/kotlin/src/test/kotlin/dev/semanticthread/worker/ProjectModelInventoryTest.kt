package dev.semanticthread.worker

import java.nio.file.Files
import kotlin.io.path.createDirectories
import kotlin.io.path.writeText
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals

class ProjectModelInventoryTest {
    @Test
    fun inventoryTracksBuildInputsAndSourceMembershipButPrunesDerivedTrees() {
        val repo = Files.createTempDirectory("model-inventory").toRealPath()
        try {
            repo.resolve("build.gradle.kts").writeText("plugins {}")
            repo.resolve("src").createDirectories()
            repo.resolve("src/A.kt").writeText("fun answer() = 1")
            repo.resolve("buildSrc/src").createDirectories()
            repo.resolve("buildSrc/src/Plugin.kt").writeText("class Plugin")
            val before = ProjectModelInventory.capture(repo)
            assertEquals(listOf("build.gradle.kts", "buildSrc/src/Plugin.kt"), before.modelInputs.map { it.path })
            assertEquals(listOf("buildSrc/src/Plugin.kt", "src/A.kt"), before.sourcePaths)
            for (excluded in listOf("build", "target", ".gradle", ".kotlin", ".semantic-thread", ".git", "buildSrc/build")) {
                repo.resolve("$excluded/nested").createDirectories()
                repo.resolve("$excluded/nested/Noise.kt").writeText("class Noise")
                repo.resolve("$excluded/nested/pom.xml").writeText("ignored")
            }
            assertEquals(before, ProjectModelInventory.capture(repo))
            repo.resolve("src/A.kt").writeText("fun answer() = 2")
            assertEquals(before, ProjectModelInventory.capture(repo)) // Source bytes belong to analysis identity.
            repo.resolve("src/B.kt").writeText("class B")
            assertNotEquals(before.inputHash("RAW"), ProjectModelInventory.capture(repo).inputHash("RAW"))
            Files.delete(repo.resolve("src/B.kt"))
            repo.resolve("buildSrc/src/Plugin.kt").writeText("class ChangedPlugin")
            assertNotEquals(before.inputHash("RAW"), ProjectModelInventory.capture(repo).inputHash("RAW"))
            assertNotEquals(before.inputHash("RAW"), before.inputHash("CANONICAL"))
        } finally { repo.toFile().deleteRecursively() }
    }

    @Test
    fun inventoryDoesNotFollowSymlinkedSourceOrModelTrees() {
        val repo = Files.createTempDirectory("inventory-links").toRealPath()
        val outside = Files.createTempDirectory("inventory-outside").toRealPath()
        try {
            outside.resolve("build.gradle.kts").writeText("external")
            outside.resolve("A.kt").writeText("class External")
            Files.createSymbolicLink(repo.resolve("buildSrc"), outside)
            Files.createSymbolicLink(repo.resolve("A.kt"), outside.resolve("A.kt"))
            assertEquals(ProjectModelInventory(emptyList(), emptyList()), ProjectModelInventory.capture(repo))
        } finally {
            repo.toFile().deleteRecursively()
            outside.toFile().deleteRecursively()
        }
    }
}
