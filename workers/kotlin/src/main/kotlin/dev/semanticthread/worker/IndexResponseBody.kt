package dev.semanticthread.worker

import java.io.ByteArrayOutputStream
import java.io.OutputStream
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.nio.file.StandardCopyOption.ATOMIC_MOVE
import java.nio.file.AtomicMoveNotSupportedException
import java.nio.file.FileAlreadyExistsException
import java.security.DigestInputStream
import java.security.DigestOutputStream
import java.security.MessageDigest
import java.util.HexFormat
import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.encodeToStream

private class BoundedIndexOutput(private val output: OutputStream, private val limit: Int) : OutputStream() {
    var size = 0
        private set

    private fun reserve(count: Int) {
        if (count > limit - size) {
            throw WorkerFailure("INCOMPLETE_SEMANTIC_ANALYSIS", "IndexFiles response exceeds the bounded transport body limit")
        }
        size += count
    }

    override fun write(value: Int) {
        reserve(1)
        output.write(value)
    }

    override fun write(bytes: ByteArray, offset: Int, length: Int) {
        reserve(length)
        output.write(bytes, offset, length)
    }
}

@OptIn(ExperimentalSerializationApi::class)
internal fun indexResponseBody(value: JsonObject, transportRoot: Path?, inlineLimit: Int, blobLimit: Int): ByteArray {
    if (transportRoot == null) {
        val inline = ByteArrayOutputStream()
        Json.encodeToStream(JsonObject.serializer(), value, BoundedIndexOutput(inline, inlineLimit))
        return Proto.bytes(2, inline.toByteArray())
    }
    val root = transportRoot.toAbsolutePath().normalize()
    if (Files.isSymbolicLink(root) || !Files.isDirectory(root, NOFOLLOW_LINKS) || root.toRealPath() != root) {
        throw WorkerFailure("INCOMPLETE_SEMANTIC_ANALYSIS", "worker transport root is not a canonical real directory")
    }
    val temporary = Files.createTempFile(root, ".index-response-", ".tmp")
    try {
        val digest = MessageDigest.getInstance("SHA-256")
        val size = Files.newOutputStream(temporary).buffered().use { stream ->
            val bounded = BoundedIndexOutput(DigestOutputStream(stream, digest), blobLimit)
            Json.encodeToStream(JsonObject.serializer(), value, bounded)
            bounded.size
        }
        if (size <= inlineLimit) return Proto.bytes(2, Files.readAllBytes(temporary))

        val digestHex = HexFormat.of().formatHex(digest.digest())
        val directory = root.resolve("sha256")
        if (!Files.exists(directory, NOFOLLOW_LINKS)) Files.createDirectory(directory)
        if (Files.isSymbolicLink(directory) || !Files.isDirectory(directory, NOFOLLOW_LINKS) || directory.toRealPath().parent != root) {
            throw WorkerFailure("INCOMPLETE_SEMANTIC_ANALYSIS", "worker transport CAS directory is unsafe")
        }
        val relative = "sha256/$digestHex"
        val target = root.resolve(relative)
        if (!Files.exists(target, NOFOLLOW_LINKS)) {
            try {
                try {
                    Files.move(temporary, target, ATOMIC_MOVE)
                } catch (_: AtomicMoveNotSupportedException) {
                    Files.move(temporary, target)
                }
            } catch (_: FileAlreadyExistsException) {
                // Verify the pre-existing content-addressed object below.
            }
        }
        if (!Files.isRegularFile(target, NOFOLLOW_LINKS) || Files.size(target) != size.toLong()) {
            throw WorkerFailure("INCOMPLETE_SEMANTIC_ANALYSIS", "worker transport CAS object differs from its content authority")
        }
        val verification = MessageDigest.getInstance("SHA-256")
        DigestInputStream(Files.newInputStream(target, NOFOLLOW_LINKS), verification).use { stream ->
            // Verify the bytes read, including EOF, without retaining a second full body.
            val buffer = ByteArray(64 * 1024)
            var remaining = size
            while (remaining > 0) {
                val read = stream.read(buffer, 0, minOf(remaining, buffer.size))
                if (read < 0) break
                remaining -= read
            }
            if (remaining != 0 || stream.read() != -1 || HexFormat.of().formatHex(verification.digest()) != digestHex) {
                throw WorkerFailure("INCOMPLETE_SEMANTIC_ANALYSIS", "worker transport CAS object differs from its content authority")
            }
        }
        return Proto.bytes(2, byteArrayOf()) + Proto.bytes(3, Proto.message(
            Proto.string(1, "sha256:$digestHex"), Proto.string(2, relative), Proto.uint(3, size.toLong()),
        ))
    } finally {
        Files.deleteIfExists(temporary)
    }
}
