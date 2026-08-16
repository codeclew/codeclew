package dev.semanticthread.worker

import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class SemanticCoreProjectModelCommandTest {
    @Test
    fun optionalTypeEvidenceKeepsOnlyCompilerProvenCore() {
        assertEquals(true, isOptionalDescriptorAttributeBoundary("UNRESOLVED_DESCRIPTOR_TYPE"))
        assertEquals(true, isOptionalDescriptorAttributeBoundary("UNKNOWN_EFFECTIVE_VISIBILITY"))
        assertEquals(false, isOptionalDescriptorAttributeBoundary("INVALID_DESCRIPTOR_IDENTITY"))
        assertEquals(true, isRetainedCallTopologyKind("CALLS"))
        assertEquals(false, isRetainedCallTopologyKind("RETURNS_VALUE_FROM"))
        val sourceRowHash = "sha256:" + "0".repeat(64)
        val partialDescriptor = descriptorCorePayload(buildJsonObject {
            put("recordType", "DECLARATION_DESCRIPTOR")
            put("schema", "declaration-descriptor/0.1")
            put("symbolIdentity", "callable:p/answer#jvm:()I")
            put("compilerCallableId", "p/answer")
            put("declarationKind", "FUNCTION")
            put("ownerIdentity", "package:p")
            putJsonArray("containment") {}
            put("resolution", "PROVEN")
            put("provider", "K2_FIR")
            put("visibility", "public")
            put("returnType", "<ERROR TYPE>")
        }, sourceRowHash)
        assertEquals("callable:p/answer#jvm:()I", partialDescriptor["symbolIdentity"]?.jsonPrimitive?.content)
        assertEquals("PARTIAL", partialDescriptor["attributeCoverage"]?.jsonPrimitive?.content)
        assertEquals(sourceRowHash, partialDescriptor["sourceRowHash"]?.jsonPrimitive?.content)
        assertNull(partialDescriptor["returnType"])
        assertNull(partialDescriptor["visibility"])

        val relation = relationCorePayload(buildJsonObject {
            put("recordType", "DECLARATION_RELATION")
            put("kind", "CALLS")
            put("owner", "callable:p/owner#jvm:()V")
            put("target", "callable:p/answer#jvm:()I")
            put("resultType", "<ERROR TYPE>")
            putJsonArray("argumentToParameter") {}
        }, sourceRowHash)
        assertEquals("CALLS", relation["kind"]?.jsonPrimitive?.content)
        assertEquals("PARTIAL", relation["attributeCoverage"]?.jsonPrimitive?.content)
        assertEquals(sourceRowHash, relation["sourceRowHash"]?.jsonPrimitive?.content)
        assertEquals("callable:p/owner#jvm:()V", relation["owner"]?.jsonPrimitive?.content)
        assertEquals("callable:p/answer#jvm:()I", relation["target"]?.jsonPrimitive?.content)
        assertNull(relation["resultType"])
        assertNull(relation["argumentToParameter"])
    }
}
