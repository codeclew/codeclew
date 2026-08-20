package dev.semanticthread.worker

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject

internal fun deduplicateCanonicalK2FactLines21(lines: List<String>): List<String> {
    val seen = linkedSetOf<String>()
    return lines.filter { line ->
        val row = Json.parseToJsonElement(line).jsonObject
        seen.add(canonicalK2Json21(row))
    }
}