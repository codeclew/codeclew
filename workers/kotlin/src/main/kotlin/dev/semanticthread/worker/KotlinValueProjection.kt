package dev.semanticthread.worker

import kotlinx.serialization.json.*

/** Attach only one exact owner projection, translating retained source coordinates. */
internal fun normalizeKotlinValueProjection(
    rawFacts: List<JsonObject>, descriptor: JsonObject, file: String, original: String,
    coordinates: CompilerUtf16ToUtf8ByteMap?, sourceDigest: String,
): JsonObject? {
    val owner = descriptor["symbolIdentity"] ?: return null
    val raw = rawFacts.filter { it["recordType"]?.jsonPrimitive?.content == "PORTABLE_VALUE_PROJECTION" && it["owner"] == owner }.singleOrNull() ?: return null
    val map = coordinates ?: return null
    val ownerStart = descriptor["start"]?.jsonPrimitive?.intOrNull ?: return null
    val ownerEnd = descriptor["end"]?.jsonPrimitive?.intOrNull ?: return null
    if (raw["schema"]?.jsonPrimitive?.content != "k2-value-projection-raw/0.1"
        || raw["compilerVersion"]?.jsonPrimitive?.content != "2.4.10"
        || raw["authority"]?.jsonPrimitive?.content != "K2_RESOLVED_IMMUTABLE_LOCAL_VALUES"
        || raw["coordinateDomain"]?.jsonPrimitive?.content != "COMPILER_UTF16_OFFSETS") return null
    val enclosing = map.range(raw["ownerStart"]?.jsonPrimitive?.intOrNull ?: return null,
        raw["ownerEnd"]?.jsonPrimitive?.intOrNull ?: return null) ?: return null
    if (enclosing.first != ownerStart || enclosing.last + 1 != ownerEnd) return null
    val rawNodes = raw["nodes"] as? JsonArray ?: return null
    if (rawNodes.isEmpty() || rawNodes.size > 128 || raw.toString().toByteArray().size > 32768) return null
    val bytes = original.toByteArray(Charsets.UTF_8)
    fun line(offset: Int): Int = 1 + bytes.take(offset).count { it == 10.toByte() }
    val nodes = rawNodes.map { element ->
        val node = element as? JsonObject ?: return null
        val start = node["start"]?.jsonPrimitive?.intOrNull ?: return null
        val end = node["end"]?.jsonPrimitive?.intOrNull ?: return null
        val span = map.range(start, end) ?: return null
        if (span.isEmpty() || span.first < ownerStart || span.last + 1 > ownerEnd) return null
        JsonObject(node + mapOf("file" to JsonPrimitive(file),
            "byteStart" to JsonPrimitive(span.first), "byteEnd" to JsonPrimitive(span.last + 1),
            "startLine" to JsonPrimitive(line(span.first)), "endLine" to JsonPrimitive(line(span.last))))
    }
    val projection = buildJsonObject {
        put("schema", "k2-value-projection/0.1"); put("compilerVersion", "2.4.10")
        put("owner", owner); put("ownerByteStart", ownerStart); put("ownerByteEnd", ownerEnd)
        put("fullCompilationSourceDigest", sourceDigest)
        put("nodes", JsonArray(nodes)); put("globalBoundaries", raw["globalBoundaries"]!!)
    }
    if (projection.toString().toByteArray(Charsets.UTF_8).size <= 32768) return projection
    // Adding original-byte evidence can exceed the bound even when raw FIR
    // equations fit. Collapse the complete model, never clip its dependencies.
    val unknown = buildJsonObject {
        put("id", "result"); put("operation", buildJsonObject {
            put("kind", "UNKNOWN"); put("boundary", "KOTLIN_VALUE_BODY_OR_BUDGET_UNSUPPORTED")
        }); put("guards", JsonArray(emptyList()))
        put("file", file); put("start", raw["ownerStart"]!!); put("end", raw["ownerEnd"]!!)
        put("byteStart", ownerStart); put("byteEnd", ownerEnd)
        put("startLine", line(ownerStart)); put("endLine", line(ownerEnd - 1))
    }
    val collapsed = JsonObject(projection + mapOf("nodes" to JsonArray(listOf(unknown))))
    return collapsed.takeIf { it.toString().toByteArray(Charsets.UTF_8).size <= 32768 }
}
