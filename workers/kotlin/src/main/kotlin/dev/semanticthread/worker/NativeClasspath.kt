package dev.semanticthread.worker

import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.NoSuchFileException
import java.nio.file.Path
import java.nio.file.attribute.BasicFileAttributes
import kotlinx.serialization.json.*
import kotlin.io.path.invariantSeparatorsPathString

private const val ABSENT_DIRECTORY = "ABSENT_DECLARED_DIRECTORY"

internal fun JsonElement.isAbsentDirectorySlot(): Boolean =
    this is JsonObject && this["kind"] == JsonPrimitive(ABSENT_DIRECTORY)

internal fun absentNativeClasspathIdentity(
    repo: Path,
    path: Path,
    index: Int,
    compileTask: JsonElement?,
    slot: JsonObject,
): String {
    require(slot.keys == setOf("kind", "index", "path", "producerTask", "outputProperty"))
    require(slot.isAbsentDirectorySlot() && slot["index"] == JsonPrimitive(index))
    require(path.isAbsolute && path.normalize() == path && path.startsWith(repo) && path != repo)
    val relative = repo.relativize(path).invariantSeparatorsPathString
    require(slot["path"] == JsonPrimitive(relative))
    val producer = (slot["producerTask"] as? JsonPrimitive)?.takeIf { it.isString }?.content
    require(producer != null && producer.startsWith(":") && producer.length > 1 && slot["producerTask"] != compileTask)
    val outputProperty = (slot["outputProperty"] as? JsonPrimitive)?.takeIf { it.isString }?.content
    require(!outputProperty.isNullOrBlank())

    // A denied read, a dangling link, and a missing directory are distinct states.
    var ancestor = path
    while (true) {
        val attributes = try {
            Files.readAttributes(ancestor, BasicFileAttributes::class.java, NOFOLLOW_LINKS)
        } catch (_: NoSuchFileException) {
            require(ancestor != repo)
            ancestor = ancestor.parent
            continue
        }
        require(ancestor != path && attributes.isDirectory && ancestor.toRealPath() == ancestor &&
            Files.isReadable(ancestor) && Files.isExecutable(ancestor))
        break
    }
    return "repo:$relative:absent-directory:${nativeHash(slot)}"
}
