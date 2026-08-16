package dev.semanticthread.worker

import java.io.ByteArrayOutputStream
import java.nio.file.Files
import java.security.MessageDigest
import kotlin.io.path.readBytes
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class TransportProjectModelCommandTest {
    @Test
    fun largeIndexBodySpillsToOneVerifiedPrivateCasObject() {
        val root = Files.createTempDirectory("worker-response-transport").toRealPath()
        try {
            val payload = """{"indexHash":"index","projectModelHash":"project","compilation":":/main","files":[],"partial":false,"padding":"${"x".repeat(4096)}"}"""
            val encoded = typedResponsePayload(12, payload, root, inlineLimitBytes = 256, maximumBlobBytes = 8192)
            val fields = Proto.fields(encoded)
            assertContentEquals(byteArrayOf(), fields.single { it.number == 2 }.bytes)
            val blob = Proto.fields(fields.single { it.number == 3 }.bytes)
            val expectedHash = "sha256:" + MessageDigest.getInstance("SHA-256").digest(payload.toByteArray()).joinToString("") { "%02x".format(it) }
            assertEquals(expectedHash, blob.single { it.number == 1 }.bytes.decodeToString())
            val relative = blob.single { it.number == 2 }.bytes.decodeToString()
            assertFalse(java.nio.file.Path.of(relative).isAbsolute)
            assertEquals(payload.toByteArray().size.toLong(), blob.single { it.number == 3 }.value)
            assertContentEquals(payload.toByteArray(), root.resolve(relative).readBytes())

            val inline = typedResponsePayload(12, payload, root, inlineLimitBytes = payload.toByteArray().size, maximumBlobBytes = 8192)
            val inlineFields = Proto.fields(inline)
            assertContentEquals(payload.toByteArray(), inlineFields.single { it.number == 2 }.bytes)
            assertTrue(inlineFields.none { it.number == 3 })

            Files.writeString(root.resolve(relative), "tampered")
            assertFailsWith<WorkerFailure> { typedResponsePayload(12, payload, root, inlineLimitBytes = 256, maximumBlobBytes = 8192) }
        } finally {
            root.toFile().deleteRecursively()
        }
    }

    @Test
    fun frameWriterRejectsOversizedPayloadBeforeWriting() {
        val output = ByteArrayOutputStream()
        Proto.writeFrame(output, ByteArray(8), maximumFrameBytes = 8)
        assertEquals(12, output.size())
        assertFailsWith<IllegalArgumentException> { Proto.writeFrame(ByteArrayOutputStream(), ByteArray(9), maximumFrameBytes = 8) }
    }
}
