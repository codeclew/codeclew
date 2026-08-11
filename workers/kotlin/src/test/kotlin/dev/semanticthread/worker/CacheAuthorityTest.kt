package dev.semanticthread.worker

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertNotEquals
import kotlin.test.assertTrue

class CacheAuthorityTest {
    private fun authority(
        extractorSchema: String = FIR_FACTS_EXTRACTOR_SCHEMA,
        pluginDigest: String = "sha256:plugin-a",
    ): JsonObject = buildJsonObject {
        put("extractorSchema", extractorSchema)
        put("pluginArtifactFingerprint", pluginDigest)
        put("workerCompilerVersion", "2.3.0")
        put("workerVersion", "0.1.0")
        put("workerProtocolVersion", "1.0")
    }

    private fun cacheMetadata(authority: JsonObject, schema: String = SEMANTIC_K2_CACHE_SCHEMA) =
        buildJsonObject {
            put("schema", schema)
            put("authority", SEMANTIC_K2_DISK_CACHE_AUTHORITY)
            authority.forEach { (key, value) -> put(key, value) }
            put("valid", true)
            put("facts", kotlinx.serialization.json.JsonArray(emptyList()))
            put("diagnostics", kotlinx.serialization.json.JsonArray(emptyList()))
        }

    @Test
    fun extractor_identity_participates_in_key_and_stale_metadata_is_rejected() {
        val original = authority()
        val changedPlugin = authority(pluginDigest = "sha256:plugin-b")
        val sameSemanticInput = "same-source-and-project"

        assertNotEquals(
            semanticK2CacheKey(original, sameSemanticInput),
            semanticK2CacheKey(changedPlugin, sameSemanticInput),
        )
        assertTrue(cacheMatchesExtractorAuthority(cacheMetadata(original), original))
        assertFalse(cacheMatchesExtractorAuthority(cacheMetadata(original), changedPlugin))
        assertFalse(
            cacheMatchesExtractorAuthority(
                cacheMetadata(original, schema = "semantic-k2-cache/0.1"),
                original,
            )
        )
        assertFalse(
            cacheMatchesExtractorAuthority(
                cacheMetadata(authority(extractorSchema = "fir-facts-extractor/0.1")),
                original,
            )
        )
    }

    @Test
    fun semantic_payload_integrity_rejects_changed_facts_with_preserved_metadata() {
        val payload = cacheMetadata(authority())
        val stored = buildJsonObject {
            payload.forEach { (key, value) -> put(key, value) }
            put("payloadIntegrity", semanticK2CachePayloadIntegrity(payload))
        }
        assertTrue(cachePayloadIntegrityMatches(stored))

        val forged = buildJsonObject {
            stored.forEach { (key, value) -> put(key, value) }
            put("facts", kotlinx.serialization.json.JsonArray(listOf(buildJsonObject {
                put("kind", "OVERRIDES")
                put("target", "forged/Decoy.read")
            })))
        }
        assertFalse(cachePayloadIntegrityMatches(forged))
        assertTrue(stored["authority"] == JsonPrimitive("NON_AUTHORITATIVE"))
    }
}
