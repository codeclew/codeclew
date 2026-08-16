package dev.semanticthread.worker

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

internal fun canonicalCompilerRowDigest(raw: JsonObject, normalizedFile: String?): String {
    if (normalizedFile == null) return stableBoundaryDigest(raw)
    val normalized = buildJsonObject {
        raw.entries.sortedBy { it.key }.forEach { (key, value) ->
            if (key == "file") put("file", normalizedFile) else put(key, value)
        }
    }
    return stableBoundaryDigest(normalized)
}
