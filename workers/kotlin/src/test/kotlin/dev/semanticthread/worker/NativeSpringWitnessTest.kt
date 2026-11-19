package dev.semanticthread.worker

import java.nio.file.Files
import java.security.MessageDigest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFails
import kotlin.test.assertNotEquals
import kotlinx.serialization.json.*
import org.jetbrains.org.objectweb.asm.ClassWriter
import org.jetbrains.org.objectweb.asm.Opcodes

class NativeSpringWitnessTest {
    @Test
    fun refusesMutatedGeneratedSourceAndNativeOutput() {
        val repo = Files.createTempDirectory("spring-witness").toRealPath()
        try {
            val output = Files.createDirectory(repo.resolve("classes"))
            val generated = repo.resolve("Generated.java")
            val compiled = output.resolve("Service.class")
            Files.writeString(generated, "class Generated {}")
            Files.write(compiled, classBytes(Opcodes.ACC_PUBLIC))
            val generatedDigest = JsonPrimitive(digest(Files.readAllBytes(generated)))
            val selected = JsonObject(mapOf("Generated.java" to generatedDigest))
            val model = buildJsonObject {
                putJsonArray("sourceFiles") {}
                putJsonArray("javaSourceFiles") { add(generated.toString()) }
                putJsonArray("classpath") {}
            }
            val receipt = buildJsonObject {
                put("outputRoot", "classes")
                putJsonObject("sourceDigests") {
                    putJsonObject("kotlin") {}
                    put("java", selected)
                }
                put("generatedJavaDigests", selected)
                putJsonArray("classpathDigests") {}
                putJsonObject("classDigests") { put("Service.class", digest(Files.readAllBytes(compiled))) }
            }
            assertEquals(output, verifyNativeSpringInputs(repo, model, receipt))
            val opened = springClassShape(output)
            Files.write(compiled, classBytes(Opcodes.ACC_PUBLIC or Opcodes.ACC_FINAL))
            assertNotEquals(opened, springClassShape(output))
            assertFails { verifyNativeSpringInputs(repo, model, receipt) }
            Files.write(compiled, classBytes(Opcodes.ACC_PUBLIC))
            Files.writeString(generated, "class Other {}")
            assertFails { verifyNativeSpringInputs(repo, model, receipt) }
            Files.delete(generated)
            assertFails { verifyNativeSpringInputs(repo, model, receipt) }
        } finally {
            repo.toFile().deleteRecursively()
        }
    }

    private fun digest(bytes: ByteArray): String = "sha256:" +
        MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

    private fun classBytes(flags: Int): ByteArray = ClassWriter(0).run {
        visit(Opcodes.V21, flags, "Service", null, "java/lang/Object", null)
        visitEnd()
        toByteArray()
    }
}
