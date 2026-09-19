package dev.semanticthread.worker

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/** Keep operational compiler timing and checkout location out of graph identity. */
internal fun compilerCfgIdentityRow(raw: JsonObject, normalizedFile: String): JsonObject =
    buildJsonObject {
        raw.forEach { (key, value) ->
            when (key) {
                "firExtractionMicros" -> Unit
                "file" -> put("file", normalizedFile)
                else -> put(key, value)
            }
        }
    }

internal fun canonicalCompilerRowDigest(raw: JsonObject, normalizedFile: String?): String {
    if (normalizedFile == null) return stableBoundaryDigest(raw)
    val normalized = buildJsonObject {
        raw.entries.sortedBy { it.key }.forEach { (key, value) ->
            if (key == "file") put("file", normalizedFile) else put(key, value)
        }
    }
    return stableBoundaryDigest(normalized)
}
