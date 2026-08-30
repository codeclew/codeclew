package dev.semanticthread.worker

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull

class DeclarationRelationCoordinateNormalizationTest {
    private fun utf8Offset(source: String, utf16Offset: Int): Int =
        source.substring(0, utf16Offset).toByteArray(Charsets.UTF_8).size

    @Test
    fun callCoordinatesUseUtf8BytesAfterCyrillicAndEmojiPrefix() {
        val source = "// Привет\nfun run() { target(\"😀\", имя) }\n"
        val callStart = source.indexOf("target")
        val callEnd = source.indexOf(')', callStart) + 1
        val firstArgumentStart = source.indexOf("\"😀\"")
        val secondArgumentStart = source.indexOf("имя", callStart)
        val raw = buildJsonObject {
            put("schema", "declaration-relation/0.1")
            put("kind", "CALLS")
            put("start", callStart)
            put("end", callEnd)
            put("orderKey", callStart)
            putJsonArray("argumentToParameter") {
                add(buildJsonObject {
                    put("argumentStart", firstArgumentStart)
                    put("parameterIndex", 0)
                    put("parameterType", "kotlin/String")
                })
                add(buildJsonObject {
                    put("argumentStart", secondArgumentStart)
                    put("parameterIndex", 1)
                    put("parameterType", "kotlin/String")
                })
            }
        }

        val normalized = assertNotNull(normalizeDeclarationRelationAttributeCoordinatesToUtf8(source, raw))

        assertEquals("declaration-relation/0.1", normalized["schema"]?.jsonPrimitive?.content)
        assertEquals(callStart, normalized["start"]?.jsonPrimitive?.content?.toInt())
        assertEquals(callEnd, normalized["end"]?.jsonPrimitive?.content?.toInt())
        assertEquals(utf8Offset(source, callStart), normalized["orderKey"]?.jsonPrimitive?.content?.toInt())
        val arguments = normalized["argumentToParameter"]!!.jsonArray
        assertEquals(
            utf8Offset(source, firstArgumentStart),
            arguments[0].jsonObject["argumentStart"]?.jsonPrimitive?.content?.toInt(),
        )
        assertEquals(
            utf8Offset(source, secondArgumentStart),
            arguments[1].jsonObject["argumentStart"]?.jsonPrimitive?.content?.toInt(),
        )
        assertEquals(1, arguments[1].jsonObject["parameterIndex"]?.jsonPrimitive?.content?.toInt())
    }

    @Test
    fun orderedRelationsIncludingReadSurviveWithoutAnOptionalArgumentMap() {
        val source = "// 😀 чтение\nfun run() = state\n"
        val relationStart = source.indexOf("state")
        val relationEnd = relationStart + "state".length
        for (kind in listOf("CALLS", "CONSTRUCTS", "READS", "WRITES", "INITIALIZES")) {
            val raw = buildJsonObject {
                put("schema", "declaration-relation/0.1")
                put("kind", kind)
                put("start", relationStart)
                put("end", relationEnd)
                put("orderKey", relationStart)
            }

            val normalized = assertNotNull(normalizeDeclarationRelationAttributeCoordinatesToUtf8(source, raw))

            assertNull(normalized["argumentToParameter"])
            assertEquals(
                utf8Offset(source, relationStart),
                normalized["orderKey"]?.jsonPrimitive?.content?.toInt(),
            )
        }
    }

    @Test
    fun nullCoalescingConvertsEveryNestedCoordinate() {
        val source = "// 😀 Префикс\nfun choose(a: String?, b: String) = a ?: b\n"
        val mergeStart = source.indexOf("a ?:")
        val mergeEnd = source.indexOf('\n', mergeStart)
        val sourceStart = mergeStart
        val sourceEnd = sourceStart + 1
        val fallbackStart = source.lastIndexOf('b')
        val fallbackEnd = fallbackStart + 1
        val raw = buildJsonObject {
            put("schema", "declaration-relation/0.1")
            put("kind", "NULL_COALESCES")
            put("start", mergeStart)
            put("end", mergeEnd)
            put("orderKey", mergeStart)
            put("sourceOccurrence", occurrence(sourceStart, sourceEnd, "kotlin/String?", true))
            put("fallbackOccurrence", occurrence(fallbackStart, fallbackEnd, "kotlin/String", false))
            put("mergedOccurrence", occurrence(mergeStart, mergeEnd, "kotlin/String", false))
            put("branchProvenance", buildJsonObject {
                put("kind", "FIR_ELVIS_EXPRESSION")
                put("nullableBranchStart", sourceStart)
                put("fallbackBranchStart", fallbackStart)
                put("mergeStart", mergeStart)
                put("mergeEnd", mergeEnd)
            })
        }

        val normalized = assertNotNull(normalizeDeclarationRelationAttributeCoordinatesToUtf8(source, raw))

        assertOccurrence(source, normalized, "sourceOccurrence", sourceStart, sourceEnd)
        assertOccurrence(source, normalized, "fallbackOccurrence", fallbackStart, fallbackEnd)
        assertOccurrence(source, normalized, "mergedOccurrence", mergeStart, mergeEnd)
        val branch = normalized["branchProvenance"]!!.jsonObject
        assertEquals(utf8Offset(source, sourceStart), branch["nullableBranchStart"]?.jsonPrimitive?.content?.toInt())
        assertEquals(utf8Offset(source, fallbackStart), branch["fallbackBranchStart"]?.jsonPrimitive?.content?.toInt())
        assertEquals(utf8Offset(source, mergeStart), branch["mergeStart"]?.jsonPrimitive?.content?.toInt())
        assertEquals(utf8Offset(source, mergeEnd), branch["mergeEnd"]?.jsonPrimitive?.content?.toInt())
        assertEquals(utf8Offset(source, mergeStart), normalized["orderKey"]?.jsonPrimitive?.content?.toInt())
    }

    @Test
    fun returnValueConvertsEveryNestedCoordinate() {
        val source = "// 😀 возврат\nfun answer() { return target() }\n"
        val returnStart = source.indexOf("return")
        val returnEnd = source.indexOf('}', returnStart) - 1
        val resultStart = source.indexOf("target", returnStart)
        val resultEnd = source.indexOf(')', resultStart) + 1
        val raw = buildJsonObject {
            put("schema", "declaration-relation/0.1")
            put("kind", "RETURNS_VALUE_FROM")
            put("start", returnStart)
            put("end", returnEnd)
            put("orderKey", resultStart)
            put("sourceOccurrence", cfgOccurrence(resultStart, resultEnd, 7))
            put("returnOccurrence", cfgOccurrence(returnStart, returnEnd, 8))
            put("resultOccurrence", cfgOccurrence(resultStart, resultEnd, 7))
        }

        val normalized = assertNotNull(normalizeDeclarationRelationAttributeCoordinatesToUtf8(source, raw))

        assertCfgOccurrence(source, normalized, "sourceOccurrence", resultStart, resultEnd, 7)
        assertCfgOccurrence(source, normalized, "returnOccurrence", returnStart, returnEnd, 8)
        assertCfgOccurrence(source, normalized, "resultOccurrence", resultStart, resultEnd, 7)
        assertEquals(utf8Offset(source, resultStart), normalized["orderKey"]?.jsonPrimitive?.content?.toInt())
    }

    @Test
    fun invalidMissingAndSurrogateInteriorCoordinatesFailClosed() {
        val source = "// Префикс 😀\nfun run() { target(value) }\n"
        val callStart = source.indexOf("target")
        val callEnd = source.indexOf(')', callStart) + 1
        val argumentStart = source.indexOf("value", callStart)
        val emojiInterior = source.indexOf("😀") + 1

        fun call(orderKey: Int? = callStart, mappedStart: Int? = argumentStart): JsonObject =
            buildJsonObject {
                put("schema", "declaration-relation/0.1")
                put("kind", "CALLS")
                put("start", callStart)
                put("end", callEnd)
                orderKey?.let { put("orderKey", it) }
                putJsonArray("argumentToParameter") {
                    add(buildJsonObject {
                        mappedStart?.let { put("argumentStart", it) }
                        put("parameterIndex", 0)
                        put("parameterType", "kotlin/String")
                    })
                }
            }

        assertNull(normalizeDeclarationRelationAttributeCoordinatesToUtf8(source, call(orderKey = null)))
        assertNull(normalizeDeclarationRelationAttributeCoordinatesToUtf8(source, call(mappedStart = null)))
        assertNull(normalizeDeclarationRelationAttributeCoordinatesToUtf8(source, call(mappedStart = -1)))
        assertNull(normalizeDeclarationRelationAttributeCoordinatesToUtf8(source, call(mappedStart = source.length + 1)))
        assertNull(normalizeDeclarationRelationAttributeCoordinatesToUtf8(source, call(mappedStart = emojiInterior)))
        assertNull(normalizeDeclarationRelationAttributeCoordinatesToUtf8(source + "\uD83D", call()))
        assertNull(normalizeDeclarationRelationAttributeCoordinatesToUtf8(source, buildJsonObject {
            put("schema", "declaration-relation/0.1")
            put("kind", "READS")
            put("start", callStart)
            put("end", callEnd)
            put("orderKey", callStart)
            put("argumentToParameter", "not-an-array")
        }))
    }

    @Test
    fun partialCallCoreWithoutCoordinatesRemainsUnchanged() {
        val source = "// 😀\nfun run() = target()\n"
        val partial = buildJsonObject {
            put("schema", "declaration-relation/0.1")
            put("kind", "CALLS")
            put("attributeCoverage", "PARTIAL")
            put("sourceRowHash", "sha256:" + "0".repeat(64))
        }

        assertEquals(partial, normalizeDeclarationRelationAttributeCoordinatesToUtf8(source, partial))
    }

    private fun occurrence(start: Int, end: Int, type: String, nullable: Boolean) =
        buildJsonObject {
            put("start", start)
            put("end", end)
            put("type", type)
            put("nullable", nullable)
        }

    private fun cfgOccurrence(start: Int, end: Int, cfgNodeId: Int) =
        buildJsonObject {
            put("start", start)
            put("end", end)
            put("cfgNodeId", cfgNodeId)
        }

    private fun assertOccurrence(
        source: String,
        relation: JsonObject,
        field: String,
        start: Int,
        end: Int,
    ) {
        val occurrence = relation[field]!!.jsonObject
        assertEquals(utf8Offset(source, start), occurrence["start"]?.jsonPrimitive?.content?.toInt())
        assertEquals(utf8Offset(source, end), occurrence["end"]?.jsonPrimitive?.content?.toInt())
    }

    private fun assertCfgOccurrence(
        source: String,
        relation: JsonObject,
        field: String,
        start: Int,
        end: Int,
        cfgNodeId: Int,
    ) {
        assertOccurrence(source, relation, field, start, end)
        assertEquals(cfgNodeId, relation[field]!!.jsonObject["cfgNodeId"]?.jsonPrimitive?.content?.toInt())
    }
}
