package dev.semanticthread.worker

import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlin.test.Test
import kotlin.test.assertEquals

class SemanticCoreHardeningProjectModelCommandTest {
    @Test
    fun structuralFailuresWinAndPartialCoreUsesExactFields() {
        val source = "fun answer() = 42"
        fun descriptor() = buildJsonObject {
            put("recordType", "DECLARATION_DESCRIPTOR")
            put("file", "/repo/A.kt")
            put("start", 0)
            put("end", source.length)
            put("symbolIdentity", "callable:p/answer#jvm:()I")
            put("ownerIdentity", "package:p")
            put("declarationKind", "FUNCTION")
            put("visibility", "package")
            put("effectiveVisibility", "public")
            put("modality", "FINAL")
            put("returnType", "kotlin/Int")
        }
        assertEquals("DESCRIPTOR_SOURCE_NOT_IN_COMPILATION", descriptorUnsupportedReason(descriptor(), "A.kt", null))
        assertEquals("INVALID_DESCRIPTOR_SOURCE_RANGE", descriptorUnsupportedReason(buildJsonObject {
            descriptor().forEach(::put)
            put("end", source.length + 1)
        }, "A.kt", source))

        val sourceRowHash = "sha256:" + "0".repeat(64)
        val relationCore = relationCorePayload(buildJsonObject {
            put("schema", "declaration-relation/0.1")
            put("kind", "CALLS")
            put("owner", "p/owner")
            put("target", "p/target")
            put("resolution", "PROVEN")
            put("provider", "K2_FIR")
            put("receiverType", "<ERROR TYPE>")
            put("orderKey", 7)
            put("futureMapping", true)
        }, sourceRowHash)
        assertEquals(
            setOf("schema", "kind", "owner", "target", "resolution", "provider", "attributeCoverage", "sourceRowHash"),
            relationCore.keys,
        )
        val firstRow = buildJsonObject {
            put("file", "/checkout-one/src/A.kt")
            put("start", 1)
            put("end", 2)
        }
        val secondRow = buildJsonObject {
            put("file", "/checkout-two/src/A.kt")
            put("start", 1)
            put("end", 2)
        }
        assertEquals(
            canonicalCompilerRowDigest(firstRow, "src/A.kt"),
            canonicalCompilerRowDigest(secondRow, "src/A.kt"),
        )
    }
}
