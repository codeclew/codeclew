package dev.semanticthread.worker

import java.nio.file.Files
import java.nio.file.Path
import java.security.MessageDigest
import java.util.zip.ZipFile
import kotlinx.serialization.json.*
import org.jetbrains.org.objectweb.asm.ClassReader
import org.jetbrains.org.objectweb.asm.ClassVisitor
import org.jetbrains.org.objectweb.asm.MethodVisitor
import org.jetbrains.org.objectweb.asm.Opcodes

private fun artifactFailure(message: String): Nothing =
    throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", message)

internal fun libraryArtifactDigest(path: Path): String {
    if (!Files.isRegularFile(path) || Files.isSymbolicLink(path)) artifactFailure("library artifact is not a regular admitted file")
    val digest = MessageDigest.getInstance("SHA-256")
    Files.newInputStream(path).use { input ->
        val buffer = ByteArray(65536)
        while (true) {
            val size = input.read(buffer)
            if (size < 0) break
            digest.update(buffer, 0, size)
        }
    }
    return "sha256:" + digest.digest().joinToString("") { "%02x".format(it) }
}

/** Coordinates are retained only when Gradle associated them with this exact chosen classpath slot. */
internal fun resolvedLibraryArtifacts(model: JsonObject, normalizedClasspath: List<String>): JsonArray {
    val classpath = model["classpath"]?.jsonArray.orEmpty().map { Path.of(it.jsonPrimitive.content).toAbsolutePath().normalize() }
    if (classpath.size != normalizedClasspath.size) artifactFailure("library classpath identities differ from selected compiler model")
    val seen = mutableSetOf<Int>()
    return JsonArray(model["dependencyArtifacts"]?.jsonArray.orEmpty().map { raw ->
        val row = raw.jsonObject
        val index = row["classpathIndex"]?.jsonPrimitive?.intOrNull
            ?: artifactFailure("resolved library artifact lacks classpath index")
        if (index !in classpath.indices || !seen.add(index)) artifactFailure("resolved library artifact classpath index is absent or repeated")
        val path = Path.of(row["file"]!!.jsonPrimitive.content).toAbsolutePath().normalize()
        if (path != classpath[index]) artifactFailure("resolved coordinates do not belong to the selected classpath slot")
        val digest = libraryArtifactDigest(path)
        if (!normalizedClasspath[index].endsWith(":$digest")) artifactFailure("resolved library artifact changed after compiler model capture")
        buildJsonObject {
            put("id", "classpath:$index:$digest")
            put("classpathIndex", index)
            put("fileName", path.fileName.toString())
            put("contentDigest", digest)
            put("size", Files.size(path))
            put("coordinates", row["coordinates"] ?: JsonNull)
            put("variant", row["variant"] ?: buildJsonObject {})
            put("authority", "GRADLE_RESOLVED_COMPONENT_ARTIFACT")
            put("classpathAuthority", model["classpathAuthority"]?.jsonObject?.get("chosen") ?: JsonNull)
        }
    })
}

private data class BinaryTarget(val identity: String, val owner: String, val method: String, val descriptor: String)

/** Inspect only compiler-named owners, opening each classpath archive once. Duplicate owners never resolve by guess. */
internal fun libraryBinaryTargets(
    model: JsonObject,
    artifacts: JsonArray,
    calls: List<JsonObject>,
    localSymbols: Set<String>,
): JsonArray {
    val groups = calls.filter { it["resolution"]?.jsonPrimitive?.contentOrNull == "COMPILER_EXACT" }
        .groupBy { it["target"]!!.jsonPrimitive.content }.filterKeys { it !in localSymbols }.toSortedMap()
    val targets = linkedMapOf<String, BinaryTarget>()
    val results = linkedMapOf<String, JsonObject>()
    fun boundary(identity: String, code: String) = buildJsonObject {
        put("targetIdentity", identity); put("status", code); putJsonArray("owners") {}
    }
    groups.entries.forEachIndexed { ordinal, (identity, rows) ->
        val owners = rows.mapNotNull { it["targetJvmOwner"]?.jsonPrimitive?.contentOrNull }.distinct()
        if (ordinal >= 2048) { results[identity] = boundary(identity, "BINARY_OWNERSHIP_TARGET_LIMIT"); return@forEachIndexed }
        if (owners.size != 1 || rows.any { it["targetJvmOwner"] == null }) {
            results[identity] = boundary(identity, "JVM_OWNER_UNAVAILABLE_OR_AMBIGUOUS"); return@forEachIndexed
        }
        val owner = owners.single()
        if (owner.length > 1024 || owner.split('/').any { it.isEmpty() || it == "." || it == ".." } || owner.any { !(it.isLetterOrDigit() || it in "_$/") }) {
            results[identity] = boundary(identity, "JVM_OWNER_INVALID"); return@forEachIndexed
        }
        val descriptor = identity.substringAfter("#jvm:", "")
        if (descriptor.isEmpty()) { results[identity] = boundary(identity, "JVM_DESCRIPTOR_UNAVAILABLE"); return@forEachIndexed }
        val method = if (identity.startsWith("constructor:")) "<init>" else identity.substringBefore("#jvm:").substringAfterLast('.').substringAfterLast('/')
        targets[identity] = BinaryTarget(identity, owner, method, descriptor)
    }
    val byOwner = targets.values.groupBy { it.owner }
    val found = targets.keys.associateWith { mutableListOf<JsonObject>() }
    val byIndex = artifacts.associateBy { it.jsonObject["classpathIndex"]!!.jsonPrimitive.int }
    model["classpath"]?.jsonArray.orEmpty().forEachIndexed { index, raw ->
        val path = Path.of(raw.jsonPrimitive.content)
        if (Files.isSymbolicLink(path)) artifactFailure("binary owner classpath entry is a symbolic link")
        if (Files.isDirectory(path)) {
            byOwner.forEach { (owner, ownerTargets) ->
                if (Files.isRegularFile(path.resolve("$owner.class"))) ownerTargets.forEach { target ->
                    found.getValue(target.identity).add(buildJsonObject { put("classpathIndex", index); put("kind", "DIRECTORY"); put("classEntry", "$owner.class") })
                }
            }
        } else if (Files.isRegularFile(path) && path.fileName.toString().endsWith(".jar")) {
            val matches = mutableListOf<Pair<BinaryTarget, JsonObject>>()
            ZipFile(path.toFile()).use { zip ->
                byOwner.forEach { (owner, ownerTargets) ->
                    val entry = zip.getEntry("$owner.class") ?: return@forEach
                    if (entry.size < 0 || entry.size > 4 * 1024 * 1024) artifactFailure("binary owner class exceeds bounded classfile size")
                    val bytes = zip.getInputStream(entry).use { it.readNBytes(4 * 1024 * 1024 + 1) }
                    if (bytes.size > 4 * 1024 * 1024) artifactFailure("binary owner class exceeds bounded classfile size")
                    val methods = mutableSetOf<Pair<String, String>>()
                    val reader = ClassReader(bytes)
                    if (reader.className != owner) artifactFailure("binary owner class entry differs from declared JVM class")
                    reader.accept(object : ClassVisitor(Opcodes.ASM9) {
                        override fun visitMethod(access: Int, name: String, descriptor: String, signature: String?, exceptions: Array<out String>?): MethodVisitor? {
                            methods.add(name to descriptor); return null
                        }
                    }, ClassReader.SKIP_CODE or ClassReader.SKIP_DEBUG or ClassReader.SKIP_FRAMES)
                    ownerTargets.forEach { target ->
                        matches.add(target to buildJsonObject {
                            put("classpathIndex", index); put("kind", "JAR"); put("classEntry", "$owner.class")
                            put("methodPresent", (target.method to target.descriptor) in methods)
                            put("artifact", byIndex[index] ?: JsonNull)
                        })
                    }
                }
            }
            if (matches.isNotEmpty()) {
                val actual = libraryArtifactDigest(path)
                val expected = byIndex[index]?.jsonObject?.get("contentDigest")?.jsonPrimitive?.contentOrNull
                if (expected != null && expected != actual) artifactFailure("binary owner artifact changed after compiler model capture")
                matches.forEach { (target, owner) -> found.getValue(target.identity).add(owner) }
            }
        }
    }
    targets.forEach { (identity, target) ->
        val owners = found.getValue(identity)
        val status = when {
            owners.size > 1 -> "AMBIGUOUS_CLASSPATH_OWNER"
            owners.isEmpty() -> "BINARY_OWNER_UNAVAILABLE"
            owners.single()["kind"]?.jsonPrimitive?.content != "JAR" -> "DIRECTORY_CLASSPATH_OWNER_UNSUPPORTED"
            owners.single()["artifact"] == JsonNull -> "RESOLVED_ARTIFACT_METADATA_UNAVAILABLE"
            owners.single()["methodPresent"]?.jsonPrimitive?.boolean != true -> "METHOD_NOT_IN_BINARY_OWNER"
            else -> "UNIQUE_CLASSPATH_OWNER"
        }
        results[identity] = buildJsonObject {
            put("targetIdentity", identity); put("ownerInternalName", target.owner)
            put("jvmDescriptor", target.descriptor); put("methodName", target.method)
            put("status", status); put("owners", JsonArray(owners))
            put("authority", "COMPILER_TARGET_AND_UNIQUE_SELECTED_CLASSPATH_CLASSFILE_NOT_SOURCE_EQUIVALENCE")
        }
    }
    return JsonArray(results.toSortedMap().values.toList())
}
