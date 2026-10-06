package dev.semanticthread.worker

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
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
        }
        putJsonArray("edges") {
            add(buildJsonObject {
                put("from", "9"); put("to", "2"); put("kind", "CFG_NORMAL"); put("label", "NormalPath")
            })
            add(buildJsonObject {
                put("from", "2"); put("to", "7"); put("kind", edgeKind); put("label", "TruePath")
            })
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
        assertEquals(setOf("NormalPath", "TruePath"), graph["edges"]!!.jsonArray.map { it.jsonObject["label"]!!.jsonPrimitive.content }.toSet())
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
    fun nonNumericCompilerIdsAreRejectedInsteadOfSilentlyDropped() {
        val malformed = buildJsonObject {
            interactive().forEach { (key, value) ->
                if (key == "nodes") putJsonArray("nodes") {
                    value.jsonArray.forEachIndexed { index, node ->
                        if (index == 0) add(buildJsonObject {
                            put("id", "fir:9"); put("kind", "ENTRY")
                        }) else add(node)
                    }
                } else put(key, value)
            }
        }
        val sealed = sealCompilerLocalCfg(
            malformed,
            "callable:example/Product.save#jvm:()V",
            "src/main/kotlin/example/Product.kt",
            "Product.save",
        )
        assertNull(sealed.graph)
        assertEquals("UNSUPPORTED_LOCAL_CFG_NODE", sealed.boundary!!["code"]!!.jsonPrimitive.content)
    }

    @Test
    fun retainedRawFirRangesMapUtf16OffsetsToUtf8AndKeepCompilerLabels() {
        val source = "// 😀 prefix\nfun next(value: Int): Int = value + 1\n"
        val functionStart = source.indexOf("fun next")
        val functionEnd = source.indexOf('\n', functionStart)
        val entryId = 11
        val operationId = 4
        val exitId = 18
        val raw = buildJsonObject {
            put("symbol", "example/Product.next")
            put("jvmDescriptor", "(I)I")
            put("file", "src/main/kotlin/example/Product.kt")
            put("start", functionStart)
            put("end", functionEnd)
            put("name", "Product.next")
            putJsonArray("nodes") {
                add(buildJsonObject {
                    put("id", entryId); put("kind", "FunctionEnterNode"); put("dead", false)
                    put("start", functionStart); put("end", functionStart + 3)
                })
                add(buildJsonObject {
                    put("id", operationId); put("kind", "QualifiedAccessNode"); put("dead", false)
                    val start = source.indexOf("value + 1")
                    put("start", start); put("end", start + "value".length)
                })
                add(buildJsonObject {
                    put("id", exitId); put("kind", "FunctionExitNode"); put("dead", false)
                    put("start", functionEnd - 1); put("end", functionEnd)
                })
            }
            putJsonArray("edges") {
                add(buildJsonObject {
                    put("from", entryId); put("to", operationId); put("label", "NormalPath"); put("edgeKind", "CfgForward")
                })
                add(buildJsonObject {
                    put("from", operationId); put("to", exitId); put("label", "NormalPath"); put("edgeKind", "CfgForward")
                })
            }
        }
        val normalized = normalizeRetainedCompilerFirCfg(
            raw,
            "callable:example/Product.next#jvm:(I)I",
            "src/main/kotlin/example/Product.kt",
            functionStart,
            functionEnd,
            source,
        )
        val sealed = sealCompilerLocalCfg(
            normalized,
            "callable:example/Product.next#jvm:(I)I",
            "src/main/kotlin/example/Product.kt",
            "Product.next",
        )
        val graph = assertNotNull(sealed.graph, sealed.boundary.toString())
        val sourceBytes = source.toByteArray(Charsets.UTF_8)
        val operation = graph["nodes"]!!.jsonArray.single { it.jsonObject["nodeId"]!!.jsonPrimitive.content.toInt() == operationId }
        val range = operation.jsonObject["source"]!!.jsonObject
        val byteStart = range["start"]!!.jsonPrimitive.content.toInt()
        val byteEnd = range["end"]!!.jsonPrimitive.content.toInt()
        assertEquals("value", sourceBytes.copyOfRange(byteStart, byteEnd).decodeToString())
        assertEquals(setOf("NormalPath"), graph["edges"]!!.jsonArray.map { it.jsonObject["label"]!!.jsonPrimitive.content }.toSet())
    }

    @Test
    fun retainedRawFirRequiresExactOwnerAndFunctionBounds() {
        val source = "fun next(value: Int): Int = value + 1\n"
        val functionEnd = source.indexOf('\n')
        val raw = buildJsonObject {
            put("symbol", "example/Product.next"); put("jvmDescriptor", "(I)I")
            put("file", "src/main/kotlin/example/Product.kt")
            put("start", 0); put("end", functionEnd); put("name", "Product.next")
            putJsonArray("nodes") {
                add(buildJsonObject { put("id", 1); put("kind", "FunctionEnterNode"); put("dead", false) })
                add(buildJsonObject { put("id", 2); put("kind", "FunctionExitNode"); put("dead", false) })
            }
            putJsonArray("edges") {
                add(buildJsonObject { put("from", 1); put("to", 2); put("label", "NormalPath"); put("edgeKind", "CfgForward") })
            }
        }
        assertFailsWith<IllegalStateException> {
            normalizeRetainedCompilerFirCfg(
                raw, "callable:example/Other.next#jvm:(I)I", "src/main/kotlin/example/Product.kt", 0, functionEnd, source,
            )
        }
        assertFailsWith<IllegalStateException> {
            normalizeRetainedCompilerFirCfg(
                raw, "callable:example/Product.next#jvm:(I)I", "src/main/kotlin/example/Product.kt", 1, functionEnd, source,
            )
        }

        fun replaceFirstNode(update: (JsonObject) -> JsonObject): JsonObject = buildJsonObject {
            raw.forEach { (key, value) ->
                if (key == "nodes") putJsonArray("nodes") {
                    value.jsonArray.forEachIndexed { index, node ->
                        add(if (index == 0) update(node.jsonObject) else node)
                    }
                } else put(key, value)
            }
        }
        val malformedRange = replaceFirstNode { node -> buildJsonObject {
            node.forEach { (key, value) -> if (key != "start" && key != "end") put(key, value) }
            put("start", "invalid"); put("end", "invalid")
        } }
        assertFailsWith<IllegalStateException> {
            normalizeRetainedCompilerFirCfg(
                malformedRange, "callable:example/Product.next#jvm:(I)I", "src/main/kotlin/example/Product.kt", 0, functionEnd, source,
            )
        }
        val missingDeadFlag = replaceFirstNode { node -> buildJsonObject {
            node.forEach { (key, value) -> if (key != "dead") put(key, value) }
        } }
        assertFailsWith<IllegalStateException> {
            normalizeRetainedCompilerFirCfg(
                missingDeadFlag, "callable:example/Product.next#jvm:(I)I", "src/main/kotlin/example/Product.kt", 0, functionEnd, source,
            )
        }
    }

    @Test
    fun retainedRawFirDropsDataOnlyEdgesAndKeepsJumpAsOperation() {
        val source = "fun next(): Int = 1\n"
        val functionEnd = source.indexOf('\n')
        val raw = buildJsonObject {
            put("symbol", "example/Product.next"); put("jvmDescriptor", "()I")
            put("file", "src/main/kotlin/example/Product.kt")
            put("start", 0); put("end", functionEnd); put("name", "Product.next")
            putJsonArray("nodes") {
                add(buildJsonObject { put("id", 1); put("kind", "FunctionEnterNode"); put("dead", false) })
                add(buildJsonObject { put("id", 2); put("kind", "JumpNode"); put("dead", false) })
                add(buildJsonObject { put("id", 3); put("kind", "FunctionExitNode"); put("dead", false) })
            }
            putJsonArray("edges") {
                add(buildJsonObject { put("from", 1); put("to", 2); put("label", "NormalPath"); put("edgeKind", "DfgForward") })
                add(buildJsonObject { put("from", 2); put("to", 3); put("label", "NormalPath"); put("edgeKind", "Forward") })
            }
        }
        val normalized = normalizeRetainedCompilerFirCfg(
            raw,
            "callable:example/Product.next#jvm:()I",
            "src/main/kotlin/example/Product.kt",
            0,
            functionEnd,
            source,
        )
        assertEquals("OPERATION", normalized["nodes"]!!.jsonArray[1].jsonObject["kind"]!!.jsonPrimitive.content)
        val retainedEdges = normalized["edges"]!!.jsonArray.map { it.jsonObject }
        assertEquals(1, retainedEdges.size)
        assertEquals(2, retainedEdges.single()["from"]!!.jsonPrimitive.content.toInt())
        assertEquals(3, retainedEdges.single()["to"]!!.jsonPrimitive.content.toInt())
        val sealed = sealCompilerLocalCfg(
            normalized,
            "callable:example/Product.next#jvm:()I",
            "src/main/kotlin/example/Product.kt",
            "Product.next",
        )
        assertNull(sealed.graph)
        assertEquals("INVALID_LOCAL_CFG_TOPOLOGY", sealed.boundary!!["code"]!!.jsonPrimitive.content)

        val unknownLabel = buildJsonObject {
            raw.forEach { (key, value) ->
                if (key == "edges") putJsonArray("edges") {
                    value.jsonArray.forEachIndexed { index, edge ->
                        if (index == 0) add(buildJsonObject {
                            edge.jsonObject.forEach { (edgeKey, edgeValue) ->
                                if (edgeKey == "edgeKind") put("edgeKind", "CfgForward")
                                else put(edgeKey, edgeValue)
                            }
                            put("label", "CompilerInventedLabel")
                        }) else add(edge)
                    }
                } else put(key, value)
            }
        }
        assertFailsWith<IllegalStateException> {
            normalizeRetainedCompilerFirCfg(
                unknownLabel,
                "callable:example/Product.next#jvm:()I",
                "src/main/kotlin/example/Product.kt",
                0,
                functionEnd,
                source,
            )
        }
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
