package dev.semanticthread.worker

import dev.semanticthread.worker.typedResponsePayload
import java.nio.file.Path
import java.security.MessageDigest
import kotlin.io.path.readBytes
import kotlinx.serialization.json.*
import org.jetbrains.kotlin.config.KotlinCompilerVersion

internal const val PROTOCOL_MAJOR = 1L
internal const val PROTOCOL_MINOR = 0L
internal const val WORKER_VERSION = "0.1.0"
internal val WORKER_COMPILER_VERSION: String = KotlinCompilerVersion.VERSION

fun main() {
    Worker().use { worker ->
        Proto.writeFrame(System.out, response(0, capabilities = true))
        while (true) {
            val frame = Proto.readFrame(System.`in`) ?: break
            val fields = Proto.fields(frame)
            val requestId = fields.firstOrNull { it.number == 1 }?.value ?: 0
            val payloadField = fields.firstOrNull { it.number in 10..17 }
                ?: fields.firstOrNull { it.number == 18 }
                ?: throw IllegalArgumentException("request has no payload or contains an unknown oneof value")
            require(fields.any { it.number == 2 } && fields.any { it.number == 3 }) { "request requires protocol version and snapshot" }
            if (payloadField.number == 18) {
                val batch = Proto.fields(payloadField.bytes)
                require(batch.firstOrNull { it.number == 1 }?.bytes?.let(::validSchemaVersion) == true) { "unsupported batch schema version" }
                val responses = batch.filter { it.number == 2 }.map { processRequest(worker, it.bytes) }
                Proto.writeFrame(System.out, batchResponse(requestId, responses))
                continue
            }
            val kind = payloadField.number - 8
            if (kind == 9) {
                Proto.writeFrame(System.out, response(requestId, payload = "{\"shutdown\":true}", responseField = 19))
                break
            }
            val encoded = try {
                response(requestId, payload = worker.handle(kind, decodeRequest(kind, payloadField.bytes)), responseField = kind + 9)
            } catch (e: WorkerFailure) {
                response(requestId, errorCode = e.code, errorMessage = e.message)
            } catch (e: Throwable) {
                response(requestId, errorCode = "INCOMPLETE_SEMANTIC_ANALYSIS", errorMessage = e.message ?: e::class.simpleName.orEmpty())
            }
            Proto.writeFrame(System.out, encoded)
        }
    }
}

private fun processRequest(worker: Worker, frame: ByteArray): ByteArray {
    val fields = Proto.fields(frame)
    val requestId = fields.firstOrNull { it.number == 1 }?.value ?: 0
    val payload = fields.firstOrNull { it.number in 10..16 }
        ?: return response(requestId, errorCode = "WORKER_PROTOCOL_MISMATCH", errorMessage = "unknown or unsupported batch item")
    val kind = payload.number - 8
    return try {
        response(requestId, payload = worker.handle(kind, decodeRequest(kind, payload.bytes)), responseField = kind + 9)
    } catch (e: WorkerFailure) {
        response(requestId, errorCode = e.code, errorMessage = e.message)
    } catch (e: Throwable) {
        response(requestId, errorCode = "INCOMPLETE_SEMANTIC_ANALYSIS", errorMessage = e.message ?: e::class.simpleName.orEmpty())
    }
}

private fun version() = Proto.message(Proto.uint(1, PROTOCOL_MAJOR), Proto.uint(2, PROTOCOL_MINOR))
private fun schemaVersion() = Proto.message(Proto.uint(1, 1), Proto.uint(2, 0))
private fun validSchemaVersion(bytes: ByteArray): Boolean = Proto.fields(bytes).firstOrNull { it.number == 1 }?.value == 1L

private fun capabilities(): ByteArray {
    val supported = listOf(
        "kotlin.project.inspect", "kotlin.index.declarations", "kotlin.resolve.symbols",
        "kotlin.resolve.expressions", "kotlin.cfg.local", "kotlin.edit.replace_expression",
        "kotlin.edit.replace_function_body", "kotlin.validate.copied_file", "kotlin.batch"
    )
    val features = listOf("functions", "locals", "assignments", "if", "when", "loops", "return", "throw", "calls", "safe_calls", "elvis")
    val unsupported = listOf("android", "multiplatform", "scripts", "expect_actual", "reflection", "compiler_plugins", "precise_coroutine_state_machine")
    return Proto.message(
        Proto.string(1, "kotlin"), Proto.string(2, WORKER_VERSION), Proto.string(3, WORKER_COMPILER_VERSION), Proto.bytes(4, version()),
        *supported.map { Proto.string(5, it) }.toTypedArray(), *features.map { Proto.string(6, it) }.toTypedArray(),
        *unsupported.map { Proto.string(7, it) }.toTypedArray()
    )
}

private fun response(requestId: Long, payload: String? = null, capabilities: Boolean = false, errorCode: String? = null, errorMessage: String = "", responseField: Int = 0): ByteArray {
    val fields = mutableListOf(Proto.uint(1, requestId), Proto.bytes(2, version()))
    if (capabilities) fields += Proto.bytes(10, capabilities())
    if (payload != null) fields += Proto.bytes(responseField, typedResponsePayload(responseField, payload))
    if (errorCode != null) fields += Proto.bytes(18, Proto.message(Proto.string(1, errorCode), Proto.string(2, errorMessage), Proto.uint(3, 0)))
    return Proto.message(*fields.toTypedArray())
}

internal fun typedResponsePayload(
    responseField: Int,
    payload: String,
    transportRoot: java.nio.file.Path? = System.getenv("CODECLEW_WORKER_TRANSPORT_ROOT")?.takeIf(String::isNotBlank)?.let(java.nio.file.Path::of),
    inlineLimitBytes: Int = 32 * 1024 * 1024,
    maximumBlobBytes: Int = 256 * 1024 * 1024,
): ByteArray {
    require(inlineLimitBytes in 1 until 64 * 1024 * 1024)
    require(maximumBlobBytes >= inlineLimitBytes)
    val value = Json.parseToJsonElement(payload).jsonObject
    val payloadBytes = payload.toByteArray()
    val fields = mutableListOf(Proto.bytes(1, schemaVersion()))
    if (responseField == 12 && payloadBytes.size > inlineLimitBytes) {
        if (payloadBytes.size > maximumBlobBytes) throw WorkerFailure("INCOMPLETE_SEMANTIC_ANALYSIS", "IndexFiles response exceeds the bounded transport body limit")
        val requestedRoot = transportRoot?.toAbsolutePath()?.normalize()
            ?: throw WorkerFailure("INCOMPLETE_SEMANTIC_ANALYSIS", "large IndexFiles response has no private transport root")
        if (!requestedRoot.isAbsolute || java.nio.file.Files.isSymbolicLink(requestedRoot) || !java.nio.file.Files.isDirectory(requestedRoot, java.nio.file.LinkOption.NOFOLLOW_LINKS)) {
            throw WorkerFailure("INCOMPLETE_SEMANTIC_ANALYSIS", "worker transport root is not a real directory")
        }
        val canonicalRoot = requestedRoot.toRealPath()
        if (canonicalRoot != requestedRoot) throw WorkerFailure("INCOMPLETE_SEMANTIC_ANALYSIS", "worker transport root identity changed")
        val digestHex = MessageDigest.getInstance("SHA-256").digest(payloadBytes).joinToString("") { "%02x".format(it) }
        val contentHash = "sha256:$digestHex"
        val directory = canonicalRoot.resolve("sha256")
        if (!java.nio.file.Files.exists(directory, java.nio.file.LinkOption.NOFOLLOW_LINKS)) java.nio.file.Files.createDirectory(directory)
        if (java.nio.file.Files.isSymbolicLink(directory) || !java.nio.file.Files.isDirectory(directory, java.nio.file.LinkOption.NOFOLLOW_LINKS) || directory.toRealPath().parent != canonicalRoot) {
            throw WorkerFailure("INCOMPLETE_SEMANTIC_ANALYSIS", "worker transport CAS directory is unsafe")
        }
        val relative = "sha256/$digestHex"
        val target = canonicalRoot.resolve(relative)
        if (!java.nio.file.Files.exists(target, java.nio.file.LinkOption.NOFOLLOW_LINKS)) {
            val temporary = java.nio.file.Files.createTempFile(directory, ".response-", ".tmp")
            try {
                java.nio.file.Files.write(temporary, payloadBytes)
                try {
                    java.nio.file.Files.move(temporary, target, java.nio.file.StandardCopyOption.ATOMIC_MOVE)
                } catch (_: java.nio.file.AtomicMoveNotSupportedException) {
                    try { java.nio.file.Files.move(temporary, target) } catch (_: java.nio.file.FileAlreadyExistsException) { }
                } catch (_: java.nio.file.FileAlreadyExistsException) { }
            } finally {
                java.nio.file.Files.deleteIfExists(temporary)
            }
        }
        if (java.nio.file.Files.isSymbolicLink(target) || !java.nio.file.Files.isRegularFile(target, java.nio.file.LinkOption.NOFOLLOW_LINKS) || java.nio.file.Files.size(target) != payloadBytes.size.toLong() || !java.nio.file.Files.readAllBytes(target).contentEquals(payloadBytes)) {
            throw WorkerFailure("INCOMPLETE_SEMANTIC_ANALYSIS", "worker transport CAS object differs from its content authority")
        }
        fields += Proto.bytes(2, byteArrayOf())
        fields += Proto.bytes(3, Proto.message(Proto.string(1, contentHash), Proto.string(2, relative), Proto.uint(3, payloadBytes.size.toLong())))
    } else {
        fields += Proto.bytes(2, payloadBytes)
    }
    fun string(number: Int, pointer: String) { value[pointer]?.jsonPrimitive?.contentOrNull?.let { fields += Proto.string(number, it) } }
    value["sourceBlob"]?.jsonObject?.let { blob ->
        fields += Proto.bytes(3, Proto.message(
            Proto.string(1, blob["contentHash"]!!.jsonPrimitive.content),
            Proto.string(2, blob["relativePath"]!!.jsonPrimitive.content),
            Proto.uint(3, blob["sizeBytes"]!!.jsonPrimitive.long)
        ))
    }
    when (responseField) {
        11 -> { string(4, "projectModelHash"); string(5, "compilerVersion"); string(6, "compilation") }
        12 -> {
            string(4, "indexHash"); string(5, "projectModelHash"); string(6, "compilation")
            fields += Proto.uint(7, value["files"]?.jsonArray?.size?.toLong() ?: 0); fields += Proto.uint(8, if (value["partial"]?.jsonPrimitive?.booleanOrNull == true) 1 else 0)
        }
        13 -> value["declaration"]?.jsonObject?.get("symbolId")?.jsonPrimitive?.contentOrNull?.let { fields += Proto.string(4, it) }
        14 -> value["anchor"]?.jsonObject?.get("anchorId")?.jsonPrimitive?.contentOrNull?.let { fields += Proto.string(4, it) }
        15 -> {
            string(4, "symbol"); fields += Proto.uint(5, value["nodes"]?.jsonArray?.size?.toLong() ?: 0); fields += Proto.uint(6, value["edges"]?.jsonArray?.size?.toLong() ?: 0)
        }
        16 -> { string(4, "originalHash"); string(5, "candidateHash"); fields += Proto.uint(6, if (value["k2Validated"]?.jsonPrimitive?.booleanOrNull == true) 1 else 0) }
        17 -> { fields += Proto.uint(4, if (value["valid"]?.jsonPrimitive?.booleanOrNull == true) 1 else 0); fields += Proto.uint(5, value["diagnostics"]?.jsonArray?.size?.toLong() ?: 0) }
    }
    return Proto.message(*fields.toTypedArray())
}

private fun batchResponse(requestId: Long, responses: List<ByteArray>): ByteArray = Proto.message(
    Proto.uint(1, requestId), Proto.bytes(2, version()), Proto.bytes(20, Proto.message(
        Proto.bytes(1, schemaVersion()), *responses.map { Proto.bytes(2, it) }.toTypedArray()
    ))
)

private fun decodeRequest(kind: Int, bytes: ByteArray): ByteArray {
    val fields = Proto.fields(bytes)
    require(fields.firstOrNull { it.number == 1 }?.bytes?.let(::validSchemaVersion) == true) { "unsupported request schema version" }
    fun string(number: Int) = fields.firstOrNull { it.number == number }?.bytes?.decodeToString().orEmpty()
    fun optionalString(number: Int) = fields.firstOrNull { it.number == number }?.bytes?.decodeToString()
    val request = when (kind) {
        2, 3 -> buildJsonObject {
            put("repo", string(2)); optionalString(3)?.let { put("compilation", it) }
            if (kind == 3) put("syntaxOnly", (fields.firstOrNull { it.number == 4 }?.value ?: 0) != 0L)
            if (kind == 3) putJsonArray("files") { fields.filter { it.number == 5 }.map { it.bytes.decodeToString() }.sorted().forEach(::add) }
        }
        4 -> buildJsonObject {
            put("repo", string(2)); put("symbol", string(3)); optionalString(4)?.let { put("compilation", it) }
        }
        5 -> buildJsonObject {
            put("repo", string(2)); put("file", string(3)); put("offset", fields.firstOrNull { it.number == 4 }?.value ?: 0); optionalString(5)?.let { put("compilation", it) }
        }
        6 -> buildJsonObject {
            put("repo", string(2)); put("symbol", string(3)); optionalString(4)?.let { put("compilation", it) }
        }
        7 -> buildJsonObject {
            val repo = string(2); put("repo", repo); put("file", string(3))
            val inline = fields.firstOrNull { it.number == 4 }?.bytes ?: byteArrayOf()
            val encodedBlob = fields.firstOrNull { it.number == 5 }?.bytes
            if (inline.isNotEmpty() || encodedBlob != null) {
                val source = if (inline.isNotEmpty()) inline else readBlob(repo, encodedBlob!!)
                put("source", source.decodeToString())
            }
            put("ownerSymbolId", string(6)); put("exactTextHash", string(7)); put("syntaxKind", string(8)); put("normalizedTokenHash", string(9))
            put("ancestorPathHash", string(10)); fields.firstOrNull { it.number == 11 }?.value?.let { put("localOrdinal", it) }; put("leftContextHash", string(12)); put("rightContextHash", string(13))
            put("kind", string(14)); put("replacement", string(15))
            fields.firstOrNull { it.number == 16 }?.bytes?.takeIf { it.isNotEmpty() }?.let { put("preconditions", Json.parseToJsonElement(it.decodeToString())) }
            fields.firstOrNull { it.number == 17 }?.bytes?.takeIf { it.isNotEmpty() }?.let { put("postconditions", Json.parseToJsonElement(it.decodeToString())) }
            optionalString(18)?.let { put("compilation", it) }
            fields.firstOrNull { it.number == 19 }?.value?.let { put("deferSemanticValidation", it != 0L) }
            fields.firstOrNull { it.number == 20 }?.bytes?.takeIf { it.isNotEmpty() }?.let { put("semanticOperation", Json.parseToJsonElement(it.decodeToString())) }
        }
        8 -> buildJsonObject {
            val repo = string(2); put("repo", repo); put("file", string(3))
            val inline = fields.firstOrNull { it.number == 4 }?.bytes ?: byteArrayOf()
            val source = if (inline.isNotEmpty()) inline else fields.firstOrNull { it.number == 5 }?.bytes?.let { readBlob(repo, it) } ?: byteArrayOf()
            put("source", source.decodeToString())
        }
        else -> error("unknown request kind $kind")
    }
    return request.toString().toByteArray()
}

private fun readBlob(repo: String, encoded: ByteArray): ByteArray {
    val fields = Proto.fields(encoded)
    val expected = fields.firstOrNull { it.number == 1 }?.bytes?.decodeToString() ?: error("blob hash missing")
    val relative = fields.firstOrNull { it.number == 2 }?.bytes?.decodeToString() ?: error("blob path missing")
    require(!relative.startsWith('/') && relative.split('/').none { it == ".." }) { "blob path escapes repository" }
    val bytes = Path.of(repo).resolve(relative).normalize().readBytes()
    val actual = "sha256:" + MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
    require(actual == expected) { "blob content hash mismatch" }
    return bytes
}
