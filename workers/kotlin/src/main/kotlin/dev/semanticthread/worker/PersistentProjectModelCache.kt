package dev.semanticthread.worker

import dev.semanticthread.worker.ProjectModelInvalidReason
import dev.semanticthread.worker.ProjectModelPublishResult
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.nio.file.StandardOpenOption
import java.nio.file.attribute.PosixFilePermission
import java.nio.file.attribute.PosixFilePermissions
import java.security.MessageDigest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

internal object PersistentProjectModelCache {
    private const val SCHEMA = "persistent-project-model-cache/0.1"
    private const val MAX_RECORD_BYTES = 64L * 1024L * 1024L
    private val resourceIdentity = Regex("^repo:([^:]+):sha256:([0-9a-f]{64})$")
    private val artifactIdentity = Regex("^artifact:([^/:\\\\]+):sha256:([0-9a-f]{64})$")
    private val digestText = Regex("^sha256:[0-9a-f]{64}$")
    private val runtimeAuthority: String by lazy(::computeRuntimeAuthority)
private val extractorAuthority: String by lazy(::computeExtractorAuthority)

enum class PublishOutcome {
    PUBLISHED,
    INVALID_MODEL,
    ROOT_UNAVAILABLE,
    WRITE_FAILED,
}

    fun load(configuredRoot: String?, repo: Path, key: String): JsonObject? {
        return try {
        val canonicalRepo = repo.toRealPath()
        val directory = cacheDirectory(configuredRoot, key) ?: return null
        withLock(directory) {
            val record = directory.resolve("model.json")
            if (!Files.isRegularFile(record, LinkOption.NOFOLLOW_LINKS) || Files.isSymbolicLink(record)) return@withLock null
            val size = Files.size(record)
            if (size <= 0 || size > MAX_RECORD_BYTES) return@withLock null
            val envelope = Json.parseToJsonElement(Files.readString(record)).jsonObject
            if (envelope.keys != setOf("schema", "key", "runtimeAuthority", "extractorAuthority", "model", "payloadHash")) return@withLock null
            if (envelope.string("schema") != SCHEMA || envelope.string("key") != key) return@withLock null
            if (envelope.string("runtimeAuthority") != runtimeAuthority || envelope.string("extractorAuthority") != extractorAuthority) return@withLock null
            val model = envelope["model"] as? JsonObject ?: return@withLock null
            val payload = cachePayload(key, model)
            if (envelope.string("payloadHash") != sha(canonical(payload).toByteArray())) return@withLock null
            if (!validateModel(canonicalRepo, model)) return@withLock null
            model
        }
        } catch (error: InterruptedException) {
            Thread.currentThread().interrupt()
            throw error
        } catch (_: Exception) {
            null
        }
    }

    fun publish(configuredRoot: String?, repo: Path, key: String, model: JsonObject): Boolean =
        publishWithOutcome(configuredRoot, repo, key, model) == PublishOutcome.PUBLISHED

    fun publishWithOutcome(configuredRoot: String?, repo: Path, key: String, model: JsonObject): PublishOutcome =
        publishWithResult(configuredRoot, repo, key, model).outcome

    fun publishWithResult(configuredRoot: String?, repo: Path, key: String, model: JsonObject): ProjectModelPublishResult {
        return try {
            val canonicalRepo = repo.toRealPath()
            if (!validateModel(canonicalRepo, model)) {
                val manifestHash = model.string("semanticInputManifestHash")
                    ?: return ProjectModelPublishResult(PublishOutcome.INVALID_MODEL, ProjectModelInvalidReason.MISSING_SEMANTIC_INPUT_MANIFEST_HASH)
                if (!digestText.matches(manifestHash)) return ProjectModelPublishResult(PublishOutcome.INVALID_MODEL, ProjectModelInvalidReason.INVALID_SEMANTIC_INPUT_MANIFEST_HASH)
                if (withSemanticInputManifestHash(model).string("semanticInputManifestHash") != manifestHash) {
                    return ProjectModelPublishResult(PublishOutcome.INVALID_MODEL, ProjectModelInvalidReason.SEMANTIC_INPUT_MANIFEST_HASH_MISMATCH)
                }
                val manifest = model["semanticInputManifest"] as? JsonObject
                    ?: return ProjectModelPublishResult(PublishOutcome.INVALID_MODEL, ProjectModelInvalidReason.MISSING_SEMANTIC_INPUT_MANIFEST)
                if (manifest["modelInputs"] != model["modelInputs"]) return ProjectModelPublishResult(PublishOutcome.INVALID_MODEL, ProjectModelInvalidReason.MODEL_INPUTS_MANIFEST_MISMATCH)
                if (manifest["jdkHomeFingerprint"] != model["jdkHomeFingerprint"]) return ProjectModelPublishResult(PublishOutcome.INVALID_MODEL, ProjectModelInvalidReason.JDK_FINGERPRINT_MANIFEST_MISMATCH)
                if (!validateModelInputs(canonicalRepo, model)) return ProjectModelPublishResult(PublishOutcome.INVALID_MODEL, ProjectModelInvalidReason.MODEL_INPUTS_INVALID)
                if (!validateResourceIdentities(canonicalRepo, model)) return ProjectModelPublishResult(PublishOutcome.INVALID_MODEL, ProjectModelInvalidReason.RESOURCE_IDENTITIES_INVALID)
                val modelJdk = model.string("jdkHome")
                if (modelJdk != null) {
                    val configured = attempt { Path.of(modelJdk).toRealPath() }
                        ?: return ProjectModelPublishResult(PublishOutcome.INVALID_MODEL, ProjectModelInvalidReason.JDK_HOME_INVALID)
                    if (configured != Path.of(System.getProperty("java.home")).toRealPath()) return ProjectModelPublishResult(PublishOutcome.INVALID_MODEL, ProjectModelInvalidReason.JDK_HOME_MISMATCH)
                }
                val jdkFingerprint = model.string("jdkHomeFingerprint")
                    ?: return ProjectModelPublishResult(PublishOutcome.INVALID_MODEL, ProjectModelInvalidReason.JDK_FINGERPRINT_MISSING)
                if (!digestText.matches(jdkFingerprint)) return ProjectModelPublishResult(PublishOutcome.INVALID_MODEL, ProjectModelInvalidReason.JDK_FINGERPRINT_INVALID)
                error("project model validation failed without a typed reason")
            }
            val directory = cacheDirectory(configuredRoot, key)
                ?: return ProjectModelPublishResult(PublishOutcome.ROOT_UNAVAILABLE)
            withLock(directory) {
                val payload = cachePayload(key, model)
                val envelope = buildJsonObject {
                    payload.forEach(::put)
                    put("payloadHash", sha(canonical(payload).toByteArray()))
                }
                atomicWrite(directory, directory.resolve("model.json"), (canonical(envelope) + "\n").toByteArray())
            }
            ProjectModelPublishResult(PublishOutcome.PUBLISHED)
        } catch (error: InterruptedException) {
            Thread.currentThread().interrupt()
            throw error
        } catch (_: Exception) {
            ProjectModelPublishResult(PublishOutcome.WRITE_FAILED)
        }
    }

    private fun cachePayload(key: String, model: JsonObject): JsonObject = buildJsonObject {
        put("schema", SCHEMA)
        put("key", key)
        put("runtimeAuthority", runtimeAuthority)
        put("extractorAuthority", extractorAuthority)
        put("model", model)
    }

    private fun validateModel(repo: Path, model: JsonObject): Boolean {
        val manifestHash = model.string("semanticInputManifestHash") ?: return false
        if (!digestText.matches(manifestHash)) return false
        if (withSemanticInputManifestHash(model).string("semanticInputManifestHash") != manifestHash) return false
        val manifest = model["semanticInputManifest"] as? JsonObject ?: return false
        if (manifest["modelInputs"] != model["modelInputs"]) return false
        if (manifest["jdkHomeFingerprint"] != model["jdkHomeFingerprint"]) return false
        if (!validateModelInputs(repo, model)) return false
        if (!validateResourceIdentities(repo, model)) return false
        val modelJdk = model.string("jdkHome")
        if (modelJdk != null) {
            val configured = attempt { Path.of(modelJdk).toRealPath() } ?: return false
            if (configured != Path.of(System.getProperty("java.home")).toRealPath()) return false
        }
        val jdkFingerprint = model.string("jdkHomeFingerprint") ?: return false
        return digestText.matches(jdkFingerprint)
    }

    private fun validateModelInputs(repo: Path, model: JsonObject): Boolean {
        val inputs = model["modelInputs"] as? JsonArray ?: return false
        if (inputs.isEmpty()) return false
        val paths = linkedSetOf<String>()
        for (element in inputs) {
            val row = element as? JsonObject ?: return false
            if (row.keys != setOf("path", "hash")) return false
            val relative = row.string("path") ?: return false
            val expected = row.string("hash") ?: return false
            if (!canonicalRelative(relative) || !digestText.matches(expected) || !paths.add(relative)) return false
            val file = attempt { repositorySourceFile(repo, relative) } ?: return false
            if (sha(Files.readAllBytes(file)) != expected) return false
        }
        return true
    }

    internal fun validateResourceIdentities(repo: Path, model: JsonObject): Boolean {
        val repositoryIdentities = linkedMapOf<String, String>()
        val artifactIdentities = linkedMapOf<String, String>()
        var unsupported = false
        fun visit(value: JsonElement) {
            when (value) {
                is JsonObject -> value.values.forEach(::visit)
                is JsonArray -> value.forEach(::visit)
                is JsonPrimitive -> if (value.isString) {
                    val text = value.contentOrNull ?: return
                    val repositoryMatch = resourceIdentity.matchEntire(text)
                    val artifactMatch = artifactIdentity.matchEntire(text)
                    when {
                        repositoryMatch != null -> {
                            val relative = repositoryMatch.groupValues[1]
                            val expected = "sha256:${repositoryMatch.groupValues[2]}"
                            if (!canonicalRelative(relative)) { unsupported = true; return }
                            val previous = repositoryIdentities.putIfAbsent(relative, expected)
                            if (previous != null && previous != expected) unsupported = true
                        }
                        artifactMatch != null -> {
                            val name = artifactMatch.groupValues[1]
                            val expected = "sha256:${artifactMatch.groupValues[2]}"
                            val previous = artifactIdentities.putIfAbsent(name, expected)
                            if (previous != null && previous != expected) unsupported = true
                        }
                        ":sha256:" in text -> unsupported = true
                    }
                }
            }
        }
        visit(model)
        if (unsupported) return false
        for ((relative, expected) in repositoryIdentities) {
            val file = attempt { repositorySourceFile(repo, relative) } ?: return false
            if (sha(Files.readAllBytes(file)) != expected) return false
        }
        if (artifactIdentities.isEmpty()) return true
        val classpathArtifacts = linkedMapOf<String, MutableSet<String>>()
        for (entry in System.getProperty("java.class.path").split(java.io.File.pathSeparatorChar)) {
            val path = attempt { Path.of(entry).toAbsolutePath().normalize() } ?: return false
            val name = path.fileName?.toString() ?: continue
            if (name !in artifactIdentities) continue
            if (Files.isSymbolicLink(path) || !Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)) return false
            val canonical = attempt { path.toRealPath() } ?: return false
            if (canonical != path) return false
            val digest = attempt { sha(Files.readAllBytes(path)) } ?: return false
            classpathArtifacts.getOrPut(name) { linkedSetOf() }.add(digest)
        }
        for ((name, expected) in artifactIdentities) {
            if (classpathArtifacts[name] != setOf(expected)) return false
        }
        return true
    }

    private fun canonicalRelative(value: String): Boolean {
        if (value.isBlank() || '\\' in value) return false
        val parsed = attempt { Path.of(value) } ?: return false
        return !parsed.isAbsolute && parsed.normalize() == parsed && parsed.none { it.toString() == ".." }
    }

    private fun cacheDirectory(configuredRoot: String?, key: String): Path? {
        val root = privateRoot(configuredRoot) ?: return null
        val parent = secureDirectory(root, "project-model-cache") ?: return null
        val authorityKey = sha((SCHEMA + "\n" + key + "\n" + runtimeAuthority + "\n" + extractorAuthority).toByteArray()).removePrefix("sha256:")
        return secureDirectory(parent, authorityKey)
    }

    private fun privateRoot(raw: String?): Path? {
        if (raw.isNullOrBlank()) return null
        val parsed = attempt { Path.of(raw) } ?: return null
        if (!parsed.isAbsolute || parsed.normalize() != parsed || Files.isSymbolicLink(parsed)) return null
        if (!Files.isDirectory(parsed, LinkOption.NOFOLLOW_LINKS)) return null
        val canonical = attempt { parsed.toRealPath() } ?: return null
        if (canonical != parsed) return null
        val permissions = attempt { Files.getPosixFilePermissions(canonical) } ?: return null
        if (permissions.any { it in nonOwnerPermissions }) return null
        return canonical
    }

    private fun secureDirectory(parent: Path, name: String): Path? {
        val path = parent.resolve(name)
        if (!Files.exists(path, LinkOption.NOFOLLOW_LINKS)) {
            attempt { Files.createDirectory(path, PosixFilePermissions.asFileAttribute(ownerDirectoryPermissions)) } ?: return null
        }
        if (Files.isSymbolicLink(path) || !Files.isDirectory(path, LinkOption.NOFOLLOW_LINKS)) return null
        return path.takeIf { attempt { it.toRealPath() == it } == true }
    }

    private fun <T> withLock(directory: Path, block: () -> T): T {
        val lock = directory.resolve("LOCK")
        FileChannel.open(lock, StandardOpenOption.CREATE, StandardOpenOption.WRITE, LinkOption.NOFOLLOW_LINKS).use { channel ->
            channel.lock().use { return block() }
        }
    }

    private fun atomicWrite(directory: Path, destination: Path, bytes: ByteArray) {
        val temporary = Files.createTempFile(directory, ".model-", ".json")
        try {
            FileChannel.open(temporary, StandardOpenOption.WRITE).use { channel ->
                var buffer = ByteBuffer.wrap(bytes)
                while (buffer.hasRemaining()) channel.write(buffer)
                channel.force(true)
            }
            Files.move(temporary, destination, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
            FileChannel.open(directory, StandardOpenOption.READ).use { it.force(true) }
        } finally {
            Files.deleteIfExists(temporary)
        }
    }

    private fun computeRuntimeAuthority(): String {
        val home = Path.of(System.getProperty("java.home")).toRealPath()
        val files = listOf(home.resolve("release"), home.resolve("bin/java"), home.resolve("lib/modules"))
        if (files.take(2).any { !Files.isRegularFile(it, LinkOption.NOFOLLOW_LINKS) || Files.isSymbolicLink(it) }) error("JDK authority files are unavailable")
        val digest = MessageDigest.getInstance("SHA-256")
        digest.update(home.toString().toByteArray())
        for (file in files.filter { Files.isRegularFile(it, LinkOption.NOFOLLOW_LINKS) && !Files.isSymbolicLink(it) }) {
            digest.update(home.relativize(file).toString().toByteArray())
            Files.newInputStream(file).use { stream ->
                val buffer = ByteArray(1024 * 1024)
                while (true) { val read = stream.read(buffer); if (read < 0) break; digest.update(buffer, 0, read) }
            }
        }
        return digest.digest().joinToString("", "sha256:") { "%02x".format(it) }
    }

    private fun computeExtractorAuthority(): String {
        val digest = MessageDigest.getInstance("SHA-256")
        val resources = listOf(
            "/dev/semanticthread/worker/PersistentProjectModelCache.class",
            "/dev/semanticthread/worker/Worker.class",
            "/dev/semanticthread/worker/SemanticInputManifestAuthorityKt.class",
            "/semantic-thread-model.init.gradle",
        )
        for (resource in resources) {
            digest.update(resource.toByteArray())
            val bytes = PersistentProjectModelCache::class.java.getResourceAsStream(resource)?.use { it.readBytes() }
                ?: error("project-model extractor authority resource is absent: $resource")
            digest.update(bytes)
        }
        return digest.digest().joinToString("", "sha256:") { "%02x".format(it) }
    }

    private fun canonical(value: JsonElement): String = when (value) {
        is JsonObject -> value.entries.sortedBy { it.key }.joinToString(",", "{", "}") { (key, element) -> "${JsonPrimitive(key)}:${canonical(element)}" }
        is JsonArray -> value.joinToString(",", "[", "]") { canonical(it) }
        else -> value.toString()
    }

    private fun sha(bytes: ByteArray): String = MessageDigest.getInstance("SHA-256")
        .digest(bytes).joinToString("", "sha256:") { "%02x".format(it) }

    private fun JsonObject.string(key: String): String? = this[key]?.jsonPrimitive?.contentOrNull

    private inline fun <T> attempt(block: () -> T): T? = try {
        block()
    } catch (error: InterruptedException) {
        Thread.currentThread().interrupt()
        throw error
    } catch (_: Exception) {
        null
    }

    private val ownerDirectoryPermissions = setOf(
        PosixFilePermission.OWNER_READ, PosixFilePermission.OWNER_WRITE, PosixFilePermission.OWNER_EXECUTE,
    )
    private val nonOwnerPermissions = PosixFilePermission.entries.filterNot { it in ownerDirectoryPermissions }.toSet()
}