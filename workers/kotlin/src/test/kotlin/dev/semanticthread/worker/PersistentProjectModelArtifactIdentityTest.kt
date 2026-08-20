package dev.semanticthread.worker

import java.io.File
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.security.MessageDigest
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlin.io.path.createDirectories
import kotlin.io.path.writeText
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class PersistentProjectModelArtifactIdentityTest {
    @Test
    fun artifactIdentityMustMatchOneExactTrustedClasspathFile() {
        val repo = Files.createTempDirectory("project-model-artifact-identity").toRealPath()
        try {
            val artifact = System.getProperty("java.class.path").split(File.pathSeparatorChar)
                .asSequence().map { Path.of(it).toAbsolutePath().normalize() }
                .first { !Files.isSymbolicLink(it) && Files.isRegularFile(it, LinkOption.NOFOLLOW_LINKS) && it.toRealPath() == it }
            val digest = MessageDigest.getInstance("SHA-256").digest(Files.readAllBytes(artifact))
                .joinToString("", "sha256:") { "%02x".format(it) }
            val valid = "artifact:${artifact.fileName}:$digest"
            assertTrue(PersistentProjectModelCache.validateResourceIdentities(repo, buildJsonObject { put("plugin", valid) }))
            val forged = "artifact:${artifact.fileName}:sha256:${"0".repeat(64)}"
            assertFalse(PersistentProjectModelCache.validateResourceIdentities(repo, buildJsonObject { put("plugin", forged) }))
            assertFalse(PersistentProjectModelCache.validateResourceIdentities(repo, buildJsonObject { put("plugin", "artifact:missing.jar:$digest") }))
            assertFalse(PersistentProjectModelCache.validateResourceIdentities(repo, buildJsonObject { put("plugin", "artifact:../${artifact.fileName}:$digest") }))
            assertFalse(PersistentProjectModelCache.validateResourceIdentities(repo, buildJsonObject {
                put("plugins", JsonArray(listOf(JsonPrimitive(valid), JsonPrimitive(forged))))
            }))
            assertFalse(PersistentProjectModelCache.validateResourceIdentities(repo, buildJsonObject { put("plugin", "foreign:${artifact.fileName}:$digest") }))
        } finally { repo.toFile().deleteRecursively() }
    }

    @Test
    fun repositoryIdentityValidationRemainsExact() {
        val repo = Files.createTempDirectory("project-model-repository-identity").toRealPath()
        try {
            val file = repo.resolve("gradle/libs.versions.toml")
            file.parent.createDirectories()
            file.writeText("[versions]\nkotlin = \"2.1.21\"\n")
            val digest = MessageDigest.getInstance("SHA-256").digest(Files.readAllBytes(file))
                .joinToString("", "sha256:") { "%02x".format(it) }
            val identity = "repo:gradle/libs.versions.toml:$digest"
            assertTrue(PersistentProjectModelCache.validateResourceIdentities(repo, buildJsonObject { put("input", identity) }))
            file.writeText("[versions]\nkotlin = \"2.1.22\"\n")
            assertFalse(PersistentProjectModelCache.validateResourceIdentities(repo, buildJsonObject { put("input", identity) }))
        } finally { repo.toFile().deleteRecursively() }
    }
}