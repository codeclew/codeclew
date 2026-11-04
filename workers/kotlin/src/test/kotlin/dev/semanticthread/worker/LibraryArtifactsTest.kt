package dev.semanticthread.worker

import java.nio.file.Files
import java.nio.file.Path
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream
import kotlin.test.*
import kotlinx.serialization.json.*
import org.jetbrains.org.objectweb.asm.ClassWriter
import org.jetbrains.org.objectweb.asm.Opcodes

class LibraryArtifactsTest {
    private val target = "callable:owned/library/Policy.decide#jvm:(I)Ljava/lang/String;"

    private fun jar(directory: Path, name: String, body: String, method: String = "decide"): Path {
        val writer = ClassWriter(0)
        writer.visit(Opcodes.V17, Opcodes.ACC_PUBLIC, "owned/library/Policy", null, "java/lang/Object", null)
        writer.visitMethod(Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC, method, "(I)Ljava/lang/String;", null, null).apply {
            visitCode(); visitLdcInsn(body); visitInsn(Opcodes.ARETURN); visitMaxs(1, 1); visitEnd()
        }
        writer.visitEnd()
        val file = directory.resolve(name)
        ZipOutputStream(Files.newOutputStream(file)).use { zip ->
            zip.putNextEntry(ZipEntry("owned/library/Policy.class").apply { time = 0 })
            zip.write(writer.toByteArray()); zip.closeEntry()
        }
        return file
    }

    private fun model(files: List<Path>): JsonObject = buildJsonObject {
        putJsonArray("classpath") { files.forEach { add(it.toString()) } }
        putJsonObject("classpathAuthority") { put("chosen", "KOTLIN_TASK_LIBRARIES") }
        putJsonArray("dependencyArtifacts") {
            files.forEachIndexed { index, file -> add(buildJsonObject {
                put("classpathIndex", index); put("file", file.toString())
                putJsonObject("coordinates") { put("group", "owned"); put("name", "library"); put("version", "1.0"); put("classifier", JsonNull); put("extension", "jar"); put("scope", "compileClasspath") }
                putJsonObject("variant") { put("org.gradle.usage", "java-api") }
            }) }
        }
    }

    private fun calls() = listOf(buildJsonObject {
        put("resolution", "COMPILER_EXACT"); put("target", target); put("targetJvmOwner", "owned/library/Policy")
    })
    private fun captureArtifacts(model: JsonObject, files: List<Path>) = resolvedLibraryArtifacts(model, files.map { "artifact:${it.fileName}:${libraryArtifactDigest(it)}" })

    @Test fun retainsSelectedArtifactAndVariantAndChecksTheActualClassMethod() {
        val root = Files.createTempDirectory("owned-library-")
        try {
            val file = jar(root, "library.jar", "v1"); val model = model(listOf(file)); val artifacts = captureArtifacts(model, listOf(file))
            assertEquals("java-api", artifacts[0].jsonObject["variant"]!!.jsonObject["org.gradle.usage"]!!.jsonPrimitive.content)
            assertEquals(libraryArtifactDigest(file), artifacts[0].jsonObject["contentDigest"]!!.jsonPrimitive.content)
            val result = libraryBinaryTargets(model, artifacts, calls(), emptySet()).single().jsonObject
            assertEquals("UNIQUE_CLASSPATH_OWNER", result["status"]!!.jsonPrimitive.content)
            assertEquals(0, result["owners"]!!.jsonArray.single().jsonObject["classpathIndex"]!!.jsonPrimitive.int)
            assertFalse(result.toString().contains(root.toString()))
        } finally { root.toFile().deleteRecursively() }
    }

    @Test fun duplicateOwnersNeverSelectAnImplementationBySignatureOrOrder() {
        val root = Files.createTempDirectory("owned-library-")
        try {
            val files = listOf(jar(root, "first.jar", "v1"), jar(root, "lookalike.jar", "other"))
            val model = model(files)
            val result = libraryBinaryTargets(model, captureArtifacts(model, files), calls(), emptySet()).single().jsonObject
            assertEquals("AMBIGUOUS_CLASSPATH_OWNER", result["status"]!!.jsonPrimitive.content)
            assertEquals(2, result["owners"]!!.jsonArray.size)
        } finally { root.toFile().deleteRecursively() }
    }

    @Test fun republishedSameCoordinatesAndSignatureCannotKeepTheOldArtifactIdentity() {
        val root = Files.createTempDirectory("owned-library-")
        try {
            val file = jar(root, "library.jar", "v1"); val model = model(listOf(file)); val artifacts = captureArtifacts(model, listOf(file)); val old = libraryArtifactDigest(file)
            jar(root, "library.jar", "republished-v1")
            assertNotEquals(old, libraryArtifactDigest(file))
            assertFailsWith<WorkerFailure> { libraryBinaryTargets(model, artifacts, calls(), emptySet()) }
        } finally { root.toFile().deleteRecursively() }
    }

    @Test fun wrongMethodAndUnboundCoordinatesAndLocalSymbolsRemainDistinct() {
        val root = Files.createTempDirectory("owned-library-")
        try {
            val file = jar(root, "library.jar", "v1", "anotherMethod"); val model = model(listOf(file)); val artifacts = captureArtifacts(model, listOf(file))
            assertEquals("METHOD_NOT_IN_BINARY_OWNER", libraryBinaryTargets(model, artifacts, calls(), emptySet()).single().jsonObject["status"]!!.jsonPrimitive.content)
            assertEquals("RESOLVED_ARTIFACT_METADATA_UNAVAILABLE", libraryBinaryTargets(model, JsonArray(emptyList()), calls(), emptySet()).single().jsonObject["status"]!!.jsonPrimitive.content)
            assertTrue(libraryBinaryTargets(model, artifacts, calls(), setOf(target)).isEmpty())
            val wrong = JsonObject(model.toMutableMap().apply { put("dependencyArtifacts", JsonArray(listOf(JsonObject(model["dependencyArtifacts"]!!.jsonArray.single().jsonObject.toMutableMap().apply { put("file", JsonPrimitive(root.resolve("other.jar").toString())) })))) })
            assertFailsWith<WorkerFailure> { captureArtifacts(wrong, listOf(file)) }
        } finally { root.toFile().deleteRecursively() }
    }
}
