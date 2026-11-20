package dev.semanticthread.worker

import java.io.OutputStream
import java.security.DigestOutputStream
import java.security.MessageDigest
import java.util.HexFormat
import java.util.TreeMap
import kotlinx.serialization.json.*

internal fun distinctCanonicalIndexRows(rows: List<JsonObject>): List<JsonObject> {
    val sorted = TreeMap<String, JsonObject>()
    for (row in rows) sorted.putIfAbsent(canonicalJson(row), row)
    return sorted.values.toList()
}

// Hash canonical JSON without retaining a second tree, a full string, or a full UTF-8 array.
internal fun canonicalIndexJsonDigest(value: JsonElement): String {
    val digest = MessageDigest.getInstance("SHA-256")
    DigestOutputStream(OutputStream.nullOutputStream(), digest).bufferedWriter(Charsets.UTF_8).use {
        appendIndexJson(value, it)
    }
    return "sha256:" + HexFormat.of().formatHex(digest.digest())
}

private fun appendIndexJson(value: JsonElement, output: Appendable) {
    when (value) {
        is JsonObject -> {
            output.append('{')
            value.entries.sortedBy { it.key }.forEachIndexed { index, (key, child) ->
                if (index != 0) output.append(',')
                output.append(JsonPrimitive(key).toString()).append(':')
                appendIndexJson(child, output)
            }
            output.append('}')
        }
        is JsonArray -> {
            output.append('[')
            value.forEachIndexed { index, child ->
                if (index != 0) output.append(',')
                appendIndexJson(child, output)
            }
            output.append(']')
        }
        else -> output.append(value.toString())
    }
}

/** Per-observation sharing of repetitive immutable JSON fields. Large or novel values pass through. */
internal class CompilerJsonPool {
    private val keys = HashMap<String, String>()
    private val literals = HashMap<JsonPrimitive, JsonPrimitive>()

    fun row(value: JsonObject): JsonObject {
        val fields = LinkedHashMap<String, JsonElement>(value.size)
        for ((key, child) in value) {
            val shared = keys[key] ?: key.also { if (key.length <= 64 && keys.size < 4096) keys[key] = it }
            fields[shared] = intern(child)
        }
        return JsonObject(fields)
    }

    private fun intern(value: JsonElement): JsonElement = when (value) {
        is JsonObject -> row(value)
        is JsonArray -> JsonArray(value.map(::intern))
        is JsonPrimitive -> literals[value] ?: value.also {
            if (value.content.length <= 256 && literals.size < 65_536) literals[value] = it
        }
    }
}
