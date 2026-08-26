package dev.semanticthread.worker

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray

class LocalCfgIndexTest {
    private fun interactive(edgeKind: String = "CFG_TRUE") = buildJsonObject {
        put("schema", "local-cfg/0.1")
        put("graphSource", "K2_FIR_CFG")
        putJsonArray("nodes") {
            add(buildJsonObject { put("id", "9"); put("kind", "ENTRY") })
            add(buildJsonObject { put("id", "2"); put("kind", "BRANCH") })
            add(buildJsonObject { put("id", "7"); put("kind", "RETURN") })
            add(buildJsonObject { put("id", "param:0"); put("kind", "PARAMETER") })
        }
        putJsonArray("edges") {
            add(buildJsonObject { put("from", "9"); put("to", "2"); put("kind", "CFG_NORMAL") })
            add(buildJsonObject { put("from", "2"); put("to", "7"); put("kind", edgeKind) })
            add(buildJsonObject { put("from", "param:0"); put("to", "9"); put("kind", "CFG_NORMAL") })
        }
    }

    @Test
    fun explicitEdgesDefineOrderInsteadOfNumericNodeIds() {
        val sealed = sealCompilerLocalCfg(
            interactive(),
            "callable:example/Product.save#jvm:()V",
            "src/main/kotlin/example/Product.kt",
            "Product.save",
        )
        assertNull(sealed.boundary)
        val graph = assertNotNull(sealed.graph)
        assertEquals(listOf(2L, 7L, 9L), graph["nodes"]!!.jsonArray.map { it.jsonObject["nodeId"]!!.jsonPrimitive.content.toLong() })
        val edges = graph["edges"]!!.jsonArray.map { edge ->
            edge.jsonObject.let { it["sourceNodeId"]!!.jsonPrimitive.content.toLong() to it["targetNodeId"]!!.jsonPrimitive.content.toLong() }
        }
        assertTrue(9L to 2L in edges)
        assertTrue(2L to 7L in edges)
        assertFalse(2L to 9L in edges)
        assertTrue(graph["graphId"]!!.jsonPrimitive.content.startsWith("sha256:"))
    }

    @Test
    fun unknownCompilerEdgeBecomesTypedUnknown() {
        val sealed = sealCompilerLocalCfg(
            interactive("NUMERIC_ID_ORDER"),
            "callable:example/Product.save#jvm:()V",
            "src/main/kotlin/example/Product.kt",
            "Product.save",
        )
        assertNull(sealed.graph)
        val boundary = assertNotNull(sealed.boundary)
        assertEquals("UNKNOWN", boundary["resolution"]!!.jsonPrimitive.content)
        assertEquals("UNSUPPORTED_LOCAL_CFG_EDGE", boundary["code"]!!.jsonPrimitive.content)
    }

    @Test
    fun snapshotIsCanonicalAndKeepsBoundariesSeparate() {
        val graph = sealCompilerLocalCfg(
            interactive(),
            "callable:example/Product.save#jvm:()V",
            "src/main/kotlin/example/Product.kt",
            "Product.save",
        )
        val boundary = unknownCompilerLocalCfg(
            "NO_SOURCE_FUNCTION",
            JsonPrimitive("raw"),
            "src/main/kotlin/example/Product.kt",
        )
        val index = attachCompilerLocalCfgSnapshot(buildJsonObject { put("schema", "semantic-index/0.1") }, listOf(boundary, graph))
        assertEquals(1, index["localCfgs"]!!.jsonArray.size)
        assertEquals(1, index["localCfgBoundaries"]!!.jsonArray.size)
        assertTrue(index["localCfgHash"]!!.jsonPrimitive.content.startsWith("sha256:"))
    }
}
