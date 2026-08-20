package dev.semanticthread.worker

import java.security.MessageDigest
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

internal fun withSemanticInputManifestHash(model: JsonObject): JsonObject {
    val unhashed = buildJsonObject {
        model.forEach { (key, value) ->
            if (key != "semanticInputManifestHash") put(key, value)
        }
    }
    val manifest = unhashed["semanticInputManifest"] ?: unhashed
    val digest = MessageDigest.getInstance("SHA-256")
        .digest(canonicalSemanticJson(manifest).toByteArray())
        .joinToString(separator = "", prefix = "sha256:") { byte -> "%02x".format(byte) }
    return buildJsonObject {
        unhashed.forEach(::put)
        put("semanticInputManifestHash", digest)
    }
}

private fun canonicalSemanticJson(value: JsonElement): String = when (value) {
    is JsonObject -> value.entries
        .sortedBy { it.key }
        .joinToString(separator = ",", prefix = "{", postfix = "}") { (key, element) ->
            "${JsonPrimitive(key)}:${canonicalSemanticJson(element)}"
        }
    is JsonArray -> value.joinToString(separator = ",", prefix = "[", postfix = "]") { canonicalSemanticJson(it) }
    else -> value.toString()
}