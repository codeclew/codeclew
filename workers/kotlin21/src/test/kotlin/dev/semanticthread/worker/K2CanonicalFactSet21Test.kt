package dev.semanticthread.worker

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlin.test.Test
import kotlin.test.assertEquals

class K2CanonicalFactSet21Test {
    @Test
    fun keyOrderEquivalentRowsAreIdempotentAndDistinctRowsRemain() {
        val first = """{"recordType":"DECLARATION","file":"A.kt","value":"first"}"""
        val reordered = """{"value":"first","file":"A.kt","recordType":"DECLARATION"}"""
        val second = """{"recordType":"DECLARATION","file":"A.kt","value":"second"}"""

        val normalized = deduplicateCanonicalK2FactLines21(
            listOf(first, first, reordered, second),
        )

        assertEquals(2, normalized.size)
        assertEquals(
            listOf(first, second).map { canonicalK2Json21(Json.parseToJsonElement(it).jsonObject) },
            normalized.map { canonicalK2Json21(Json.parseToJsonElement(it).jsonObject) },
        )
    }
}