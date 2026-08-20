package dev.semanticthread.worker

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray

class ProjectModelCacheCanonicalPublicationTest {
    @Test
    fun existingManifestIsPreservedAndOwnsTheDigest() {
        val manifest = buildJsonObject {
            put("schema", "kotlin-semantic-input-manifest/0.1")
            put("jdkHomeFingerprint", "sha256:${"a".repeat(64)}")
            putJsonArray("modelInputs") {}
        }
        val first = withSemanticInputManifestHash(modelWith(manifest, "first"))
        val second = withSemanticInputManifestHash(modelWith(manifest, "second"))

        assertEquals(manifest, first["semanticInputManifest"])
        assertEquals(manifest, second["semanticInputManifest"])
        assertEquals(first["semanticInputManifestHash"], second["semanticInputManifestHash"])
    }

    @Test
    fun missingManifestIsNotSynthesizedFromAnUnnormalizedBuildModel() {
        val raw = buildJsonObject { put("transientExtractionField", "raw") }

        val hashed = withSemanticInputManifestHash(raw)

        assertNull(hashed["semanticInputManifest"])
    }

    private fun modelWith(manifest: JsonObject, transient: String) = buildJsonObject {
        put("semanticInputManifest", manifest)
        put("transientExtractionField", transient)
    }
}