package dev.semanticthread.worker

import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlin.test.Test
import kotlin.test.assertEquals

class SemanticInputManifestAuthorityTest {
    @Test
    fun canonical_manifest_hash_replaces_stale_value_and_is_idempotent() {
        val manifest = buildJsonObject {
            put("schema", "semantic-input-manifest/0.1")
            put("configuration", "stable")
        }
        val model = buildJsonObject {
            put("semanticInputManifest", manifest)
            put("semanticInputManifestHash", "sha256:stale")
        }

        val bound = withSemanticInputManifestHash(model)

        assertEquals(
            JsonPrimitive("sha256:e22cee7cffd96cc51f4d2e00d27b9aee247d932fc8b56811fbc47454125ca596"),
            bound["semanticInputManifestHash"],
        )
        assertEquals(bound, withSemanticInputManifestHash(bound))
    }

    @Test
    fun canonical_model_is_the_fail_closed_authority_before_manifest_projection() {
        val model = buildJsonObject { put("buildSystem", "GRADLE") }

        val bound = withSemanticInputManifestHash(model)

        assertEquals(
            JsonPrimitive("sha256:e758cefb8fdd5472e8ca0d59c2b280b28030072d6deee3abf079797989389375"),
            bound["semanticInputManifestHash"],
        )
        assertEquals(bound, withSemanticInputManifestHash(bound))
    }
}