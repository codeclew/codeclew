package dev.semanticthread.worker

import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class PartialDescriptorCoreProjectModelCommandTest {
    @Test
    fun partialDescriptorContainsIdentityCoreButNoDispatchAttributes() {
        val sourceRowHash = "sha256:" + "0".repeat(64)
        val partial = descriptorCorePayload(buildJsonObject {
            put("recordType", "DECLARATION_DESCRIPTOR")
            put("schema", "declaration-descriptor/0.1")
            put("symbolIdentity", "callable:p/answer#jvm:()I")
            put("compilerCallableId", "p/answer")
            put("declarationKind", "FUNCTION")
            put("ownerIdentity", "package:p")
            putJsonArray("containment") {}
            put("resolution", "PROVEN")
            put("provider", "K2_FIR")
            put("returnType", "<ERROR TYPE>")
            put("visibility", "public")
            put("isOverride", true)
            put("isPrimary", true)
        }, sourceRowHash)
        assertEquals(
            setOf("schema", "symbolIdentity", "compilerCallableId", "declarationKind", "ownerIdentity", "containment", "resolution", "provider", "attributeCoverage", "sourceRowHash"),
            partial.keys,
        )
        assertEquals(sourceRowHash, partial["sourceRowHash"]?.toString()?.trim('"'))
        assertNull(partial["returnType"])
        assertNull(partial["visibility"])
        assertNull(partial["isOverride"])
        assertNull(partial["isPrimary"])
    }
}
