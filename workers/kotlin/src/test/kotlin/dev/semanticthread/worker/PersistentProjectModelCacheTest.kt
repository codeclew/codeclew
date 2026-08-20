package dev.semanticthread.worker

import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.attribute.PosixFilePermission
import java.security.MessageDigest
import kotlin.io.path.createDirectories
import kotlin.io.path.writeText
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlinx.serialization.json.putJsonObject

class PersistentProjectModelCacheTest {
    @Test
    fun verifiedRoundTripInvalidatesChangedModelAndArtifactInputs() {
        val root = privateDirectory("project-model-cache-root")
        val repo = privateDirectory("project-model-cache-repo")
        try {
            val source = repo.resolve("src/main/kotlin/p/A.kt").also { it.parent.createDirectories(); it.writeText("package p\nclass A\n") }
            val build = repo.resolve("build.gradle.kts").also { it.writeText("plugins {}\n") }
            val artifact = repo.resolve(".gradle/cache/library.jar").also { it.parent.createDirectories(); it.writeText("artifact-v1") }
            val model = model(repo, source, build, artifact)
            assertTrue(PersistentProjectModelCache.publish(root.toString(), repo, "key-a", model))
            assertEquals(model, PersistentProjectModelCache.load(root.toString(), repo, "key-a"))
            assertNull(PersistentProjectModelCache.load(root.toString(), repo, "key-b"))

            artifact.writeText("artifact-v2")
            assertNull(PersistentProjectModelCache.load(root.toString(), repo, "key-a"))
            artifact.writeText("artifact-v1")
            assertTrue(PersistentProjectModelCache.publish(root.toString(), repo, "key-a", model))
            build.writeText("plugins { id(\"changed\") }\n")
            assertNull(PersistentProjectModelCache.load(root.toString(), repo, "key-a"))

            val tampered = buildJsonObject { model.forEach(::put); put("semanticInputManifestHash", "sha256:${"0".repeat(64)}") }
            assertFalse(PersistentProjectModelCache.publish(root.toString(), repo, "tampered", tampered))
        } finally {
            root.toFile().deleteRecursively()
            repo.toFile().deleteRecursively()
        }
    }

    @Test
    fun absentNonPrivateAndSymlinkedRootsNeverAuthorizeCacheIO() {
        val repo = privateDirectory("project-model-cache-repo")
        val source = repo.resolve("src/main/kotlin/p/A.kt").also { it.parent.createDirectories(); it.writeText("class A") }
        val build = repo.resolve("build.gradle.kts").also { it.writeText("plugins {}") }
        val artifact = repo.resolve("library.jar").also { it.writeText("artifact") }
        val model = model(repo, source, build, artifact)
        val realRoot = privateDirectory("project-model-cache-real")
        val link = realRoot.parent.resolve(realRoot.fileName.toString() + "-link")
        try {
            assertNull(PersistentProjectModelCache.load(null, repo, "key"))
            assertFalse(PersistentProjectModelCache.publish(null, repo, "key", model))
            Files.createSymbolicLink(link, realRoot)
            assertNull(PersistentProjectModelCache.load(link.toString(), repo, "key"))
            assertFalse(PersistentProjectModelCache.publish(link.toString(), repo, "key", model))
            Files.setPosixFilePermissions(realRoot, PosixFilePermission.entries.toSet())
            assertFalse(PersistentProjectModelCache.publish(realRoot.toString(), repo, "key", model))
        } finally {
            Files.deleteIfExists(link)
            realRoot.toFile().deleteRecursively()
            repo.toFile().deleteRecursively()
        }
    }

    private fun model(repo: Path, source: Path, build: Path, artifact: Path): JsonObject {
        val buildHash = sha(Files.readAllBytes(build))
        val artifactIdentity = "repo:${repo.relativize(artifact).toString().replace('\\', '/')}:${sha(Files.readAllBytes(artifact))}"
        val raw = buildJsonObject {
            put("schema", "semantic-project/0.1")
            put("jdkHomeFingerprint", sha(System.getProperty("java.home").toByteArray()))
            putJsonArray("sourceFiles") { add(JsonPrimitive(source.toString())) }
            putJsonArray("compileClasspath") { add(JsonPrimitive(artifactIdentity)) }
            putJsonArray("modelInputs") {
                add(buildJsonObject { put("path", repo.relativize(build).toString()); put("hash", buildHash) })
            }
            putJsonObject("semanticInputManifest") {
                put("schema", "kotlin-semantic-input-manifest/0.1")
                put("jdkHomeFingerprint", sha(System.getProperty("java.home").toByteArray()))
                putJsonArray("orderedCompileClasspath") { add(JsonPrimitive(artifactIdentity)) }
                putJsonArray("modelInputs") {
                    add(buildJsonObject { put("path", repo.relativize(build).toString()); put("hash", buildHash) })
                }
            }
        }
        return withSemanticInputManifestHash(raw)
    }

    private fun privateDirectory(prefix: String): Path = Files.createTempDirectory(prefix).toRealPath().also {
        Files.setPosixFilePermissions(it, setOf(PosixFilePermission.OWNER_READ, PosixFilePermission.OWNER_WRITE, PosixFilePermission.OWNER_EXECUTE))
    }

    private fun sha(bytes: ByteArray): String = MessageDigest.getInstance("SHA-256").digest(bytes)
        .joinToString("", "sha256:") { "%02x".format(it) }
}