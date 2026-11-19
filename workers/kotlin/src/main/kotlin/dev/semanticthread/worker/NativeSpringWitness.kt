package dev.semanticthread.worker

import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.security.MessageDigest
import kotlinx.serialization.json.*
import kotlin.io.path.invariantSeparatorsPathString
import org.jetbrains.org.objectweb.asm.ClassReader
import org.jetbrains.org.objectweb.asm.ClassVisitor
import org.jetbrains.org.objectweb.asm.MethodVisitor
import org.jetbrains.org.objectweb.asm.Opcodes

internal val springCompilerOptions = JsonArray(listOf(JsonPrimitive("plugin:org.jetbrains.kotlin.allopen:preset=spring")))

private const val ALL_OPEN_ARTIFACT = "kotlin-allopen-compiler-plugin-embeddable-2.2.21.jar"
private const val ALL_OPEN_DIGEST = "sha256:18b6d84f93899f09daffc7b104f8c4333cd78fe0b8d8ac58434c93d9f4e77e47"
private const val ANALYZER_ALL_OPEN_DIGEST = "sha256:89f477877229a98d0c9aac7f23ce965ffdb0fb782a9af9dca9f40fcadf7ed572"

private fun nativeHash(bytes: ByteArray) = "sha256:" +
    MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

private fun nativeHash(value: JsonElement) = nativeHash(value.toString().toByteArray())

private fun nativeFileHash(path: Path): String {
    val digest = MessageDigest.getInstance("SHA-256")
    Files.newInputStream(path, NOFOLLOW_LINKS).use { stream ->
        val buffer = ByteArray(65536)
        while (true) {
            val count = stream.read(buffer)
            if (count < 0) break
            digest.update(buffer, 0, count)
        }
    }
    return "sha256:" + digest.digest().joinToString("") { "%02x".format(it) }
}

private fun nativeClassFiles(root: Path): Map<String, Path> {
    require(Files.isDirectory(root, NOFOLLOW_LINKS) && root.toRealPath() == root)
    return Files.walk(root).use { entries ->
        entries.peek { require(!Files.isSymbolicLink(it)) }
            .filter { Files.isRegularFile(it, NOFOLLOW_LINKS) && it.fileName.toString().endsWith(".class") }
            .limit(16385)
            .toList()
            .associateBy { root.relativize(it).invariantSeparatorsPathString }
            .toSortedMap()
    }.also { require(it.isNotEmpty() && it.size <= 16384) }
}

internal fun isSpringCompileReceipt(
    receipt: JsonObject,
    revision: String?,
    compileTask: String,
): Boolean = revision != null && receipt["revision"] == JsonPrimitive(revision) &&
    receipt["compileTask"] == JsonPrimitive(compileTask) && receipt["outcome"] == JsonPrimitive("EXECUTED") &&
    receipt["options"] == springCompilerOptions &&
    (receipt["requestedPluginArtifacts"] as? JsonArray)?.count {
        val artifact = it as? JsonObject
        artifact?.get("name") == JsonPrimitive(ALL_OPEN_ARTIFACT) && artifact["digest"] == JsonPrimitive(ALL_OPEN_DIGEST)
    } == 1

internal fun nativeSpringReceiptSummary(receipt: JsonObject): JsonObject = buildJsonObject {
    put("schema", "codeclew-kotlin-native-compilation/1")
    put("revision", checkNotNull(receipt["revision"]))
    put("compileTask", checkNotNull(receipt["compileTask"]))
    put("outcome", checkNotNull(receipt["outcome"]))
    put("receiptDigest", nativeHash(receipt))
    put("selectedSourcesDigest", nativeHash(checkNotNull(receipt["sourceDigests"])))
    put("generatedJavaSelectionDigest", nativeHash(checkNotNull(receipt["generatedJavaDigests"])))
    put("orderedClasspathDigest", nativeHash(checkNotNull(receipt["classpathDigests"])))
    put("nativeClassfilesDigest", nativeHash(checkNotNull(receipt["classDigests"])))
    put("requestedCompiler", "2.2.21")
    put("requestedCompilerArtifactsDigest", nativeHash(checkNotNull(receipt["compilerArtifacts"])))
    put("requestedPluginArtifactsDigest", nativeHash(checkNotNull(receipt["requestedPluginArtifacts"])))
    put("requestedAllOpenArtifactDigest", ALL_OPEN_DIGEST)
    put("orderedOptionsDigest", nativeHash(springCompilerOptions))
    put("jvmTarget", "21")
}

internal fun verifyNativeSpringInputs(repo: Path, model: JsonObject, receipt: JsonObject): Path {
    fun sourceDigests(key: String) = JsonObject(model[key]!!.jsonArray.associate { value ->
        val path = Path.of(value.jsonPrimitive.content)
        require(path.startsWith(repo) && path.toRealPath() == path && Files.isRegularFile(path, NOFOLLOW_LINKS))
        repo.relativize(path).invariantSeparatorsPathString to JsonPrimitive(nativeFileHash(path))
    })
    val sources = receipt["sourceDigests"]!!.jsonObject
    require(sourceDigests("sourceFiles") == sources["kotlin"] && sourceDigests("javaSourceFiles") == sources["java"])
    val generated = receipt["generatedJavaDigests"]!!.jsonObject
    require(generated.isNotEmpty() && generated.all { (path, digest) -> sources["java"]!!.jsonObject[path] == digest })
    val classpath = model["classpath"]!!.jsonArray.map { value ->
        val path = Path.of(value.jsonPrimitive.content)
        if (Files.isRegularFile(path, NOFOLLOW_LINKS)) {
            JsonPrimitive(nativeFileHash(path))
        } else {
            require(Files.isDirectory(path, NOFOLLOW_LINKS) && path.toRealPath() == path)
            JsonObject(Files.walk(path).use { entries ->
                entries.peek { require(!Files.isSymbolicLink(it)) }.filter { Files.isRegularFile(it, NOFOLLOW_LINKS) }
                    .toList().associate { path.relativize(it).invariantSeparatorsPathString to JsonPrimitive(nativeFileHash(it)) }
            })
        }
    }
    require(JsonArray(classpath) == receipt["classpathDigests"])
    val output = repo.resolve(receipt["outputRoot"]!!.jsonPrimitive.content).normalize()
    require(output.startsWith(repo) && output != repo)
    require(JsonObject(nativeClassFiles(output).mapValues { JsonPrimitive(nativeFileHash(it.value)) }) == receipt["classDigests"])
    return output
}

/**
 * Compare the classes and methods that Spring can open, the inheritance and
 * JVM descriptors, and direct static/super calls. Other bytecode differences
 * between supported compiler patches are outside this witness.
 */
internal fun springClassShape(root: Path): Map<String, List<String>> = nativeClassFiles(root).mapValues { (_, path) ->
    require(Files.size(path) <= 16 * 1024 * 1024)
    val entries = mutableListOf<String>()
    val visibility = Opcodes.ACC_PUBLIC or Opcodes.ACC_PRIVATE or Opcodes.ACC_PROTECTED
    val shape = visibility or Opcodes.ACC_FINAL or Opcodes.ACC_ABSTRACT or Opcodes.ACC_STATIC
    val reader = ClassReader(Files.readAllBytes(path))
    reader.accept(object : ClassVisitor(Opcodes.ASM9) {
        override fun visit(version: Int, access: Int, name: String, sig: String?, parent: String?, interfaces: Array<out String>?) {
            entries += "class:${version}:${access and shape}:$name:$parent:${interfaces.orEmpty().joinToString(",")}"
        }

        override fun visitMethod(access: Int, name: String, descriptor: String, signature: String?, exceptions: Array<out String>?): MethodVisitor? {
            if (access and Opcodes.ACC_SYNTHETIC != 0) return null
            val method = "method:${access and shape}:$name:$descriptor"
            entries += method
            var ordinal = 0
            return object : MethodVisitor(Opcodes.ASM9) {
                override fun visitMethodInsn(opcode: Int, owner: String, name: String, descriptor: String, isInterface: Boolean) {
                    if (opcode == Opcodes.INVOKESTATIC || opcode == Opcodes.INVOKESPECIAL) {
                        entries += "$method:call:${ordinal++}:$opcode:$owner:$name:$descriptor:$isInterface"
                    }
                }
            }
        }
    }, ClassReader.SKIP_DEBUG or ClassReader.SKIP_FRAMES)
    entries.sorted()
}

internal fun sealSpringReplay(
    receipt: JsonObject,
    nativeShape: Map<String, List<String>>,
    replayRoot: Path,
): JsonObject? {
    if (nativeShape != springClassShape(replayRoot)) return null
    return buildJsonObject {
        nativeSpringReceiptSummary(receipt).forEach(::put)
        put("comparison", "CLASS_FLAGS_INHERITANCE_METHODS_AND_STATIC_SUPER_CALLS_MATCH")
        put("comparisonDigest", nativeHash(JsonObject(nativeShape.mapValues { (_, value) ->
            JsonArray(value.map(::JsonPrimitive))
        })))
        put("analyzerCompiler", WORKER_COMPILER_VERSION)
        put("effectiveAllOpenArtifactDigest", ANALYZER_ALL_OPEN_DIGEST)
    }
}

internal fun isSpringReplayPlugin(plugin: Path): Boolean =
    plugin.fileName.toString() == "kotlin-allopen-compiler-plugin-embeddable-2.4.10.jar" &&
        nativeFileHash(plugin) == ANALYZER_ALL_OPEN_DIGEST
