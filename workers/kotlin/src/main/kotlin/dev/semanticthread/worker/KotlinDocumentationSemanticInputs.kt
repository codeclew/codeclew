package dev.semanticthread.worker

import kotlinx.serialization.json.*

/** Documentation metadata only: neither a transfer engine nor executable inlining admission. */
internal fun normalizeKotlinDocumentationSemanticInputs(
    facts: List<JsonObject>, calls: List<JsonObject>, descriptor: JsonObject,
    file: String, scope: String, coordinates: CompilerUtf16ToUtf8ByteMap?, digest: String,
): JsonObject {
    val owner = descriptor["symbolIdentity"]!!.jsonPrimitive.content
    val ownerStart = descriptor["start"]!!.jsonPrimitive.int
    val ownerEnd = descriptor["end"]!!.jsonPrimitive.int
    fun result(status: String, variables: List<JsonObject>, callRows: List<JsonObject>, returns: List<JsonObject>, boundaries: Set<String>) = buildJsonObject {
        put("schema", "codeclew-kotlin-documentation-semantic-inputs/1.0")
        put("resolution", status); put("authority", "K2_LEXICAL_AND_ARGUMENT_SYMBOLS_AND_RETURN_TARGETS")
        put("coordinateDomain", "ORIGINAL_UTF8_BYTES")
        put("ownerSymbolIdentity", owner); put("file", file); put("compilationScope", scope)
        put("ownerByteStart", ownerStart); put("ownerByteEnd", ownerEnd); put("fullCompilationSourceDigest", digest)
        put("variables", JsonArray(variables)); put("calls", JsonArray(callRows)); put("returns", JsonArray(returns))
        put("boundaries", JsonArray(boundaries.sorted().map(::JsonPrimitive)))
        put("cfgAndExecutionProof", "SEPARATE_NOT_ASSERTED")
    }
    class Invalid : RuntimeException()
    fun unavailable(code: String) = result("UNKNOWN", emptyList(), emptyList(), emptyList(), setOf(code))
    if (coordinates == null) return unavailable("SEMANTIC_INPUT_COORDINATES_UNAVAILABLE")
    val selected = facts.filter { it["ownerSymbolIdentity"]?.jsonPrimitive?.content == owner }
    val receipt = selected.singleOrNull { it["recordType"]?.jsonPrimitive?.content == "DOCUMENTATION_LEXICAL_RECEIPT" }
        ?: return unavailable("SEMANTIC_INPUT_LEXICAL_RECEIPT_UNAVAILABLE")
    return try {
        fun range(row: JsonObject, start: String = "start", end: String = "end"): Pair<Int,Int> {
            val a = row[start]?.jsonPrimitive?.intOrNull ?: throw Invalid()
            val b = row[end]?.jsonPrimitive?.intOrNull ?: throw Invalid()
            val span = coordinates.range(a,b) ?: throw Invalid()
            if (span.isEmpty() || span.first < ownerStart || span.last + 1 > ownerEnd) throw Invalid()
            return span.first to span.last + 1
        }
        fun region(raw: JsonElement?): JsonObject {
            val row = raw as? JsonObject ?: throw Invalid()
            val kind = row["kind"]?.jsonPrimitive?.content ?: throw Invalid()
            if (kind !in setOf("NAMED_FUNCTION", "ANONYMOUS_FUNCTION", "LOCAL_FUNCTION") || row["ownerSymbolIdentity"]?.jsonPrimitive?.content != owner) throw Invalid()
            val expected = if (kind == "NAMED_FUNCTION") owner else "$owner/lexical/$kind/${row["start"]!!.jsonPrimitive.int}:${row["end"]!!.jsonPrimitive.int}"
            if (row["identity"]?.jsonPrimitive?.content != expected) throw Invalid()
            val (start,end) = range(row)
            if (kind == "NAMED_FUNCTION" && (start != ownerStart || end != ownerEnd)) throw Invalid()
            if (row["execution"]?.jsonPrimitive?.content != if (kind == "NAMED_FUNCTION") "OWNER_BODY" else "DEFERRED") throw Invalid()
            return JsonObject(row.filterKeys { it !in setOf("start", "end") } + mapOf(
                "byteStart" to JsonPrimitive(start), "byteEnd" to JsonPrimitive(end),
                "identityCoordinateDomain" to JsonPrimitive("OPAQUE_COMPILER_SYMBOL_AND_UTF16_REGION"),
            ))
        }
        if (receipt["schema"]?.jsonPrimitive?.content != "kotlin-documentation-lexical-receipt/1.0"
            || receipt["authority"]?.jsonPrimitive?.content != "K2_RESOLVED_LEXICAL_SYMBOLS"
            || receipt["coordinateDomain"]?.jsonPrimitive?.content != "COMPILER_UTF16_OFFSETS"
            || range(receipt,"ownerStart","ownerEnd") != ownerStart to ownerEnd) throw Invalid()
        val boundaries = receipt["boundaries"]!!.jsonArray.map { it.jsonPrimitive.content }.toMutableSet()
        val variables = selected.filter { it["recordType"]?.jsonPrimitive?.content == "DOCUMENTATION_LEXICAL_VARIABLE" }.map { row ->
            if (row["schema"]?.jsonPrimitive?.content != "kotlin-documentation-lexical-variable/1.0"
                || row["resolution"]?.jsonPrimitive?.content != "COMPILER_EXACT"
                || row["authority"]?.jsonPrimitive?.content != "K2_RESOLVED_VARIABLE_SYMBOL"
                || row["coordinateDomain"]?.jsonPrimitive?.content != "COMPILER_UTF16_OFFSETS"
                || range(row,"ownerStart","ownerEnd") != ownerStart to ownerEnd) throw Invalid()
            val (start,end) = range(row); val (declStart,declEnd) = range(row,"declarationStart","declarationEnd")
            val lexical = region(row["lexicalRegion"]); val declared = region(row["declarationRegion"])
            if (start < lexical["byteStart"]!!.jsonPrimitive.int || end > lexical["byteEnd"]!!.jsonPrimitive.int
                || declStart < declared["byteStart"]!!.jsonPrimitive.int || declEnd > declared["byteEnd"]!!.jsonPrimitive.int) throw Invalid()
            val expected = when (row["variableKind"]?.jsonPrimitive?.content) {
                "PARAMETER" -> "${declared["identity"]!!.jsonPrimitive.content}/parameter/${row["parameterIndex"]!!.jsonPrimitive.int}"
                "LOCAL_VARIABLE" -> "${declared["identity"]!!.jsonPrimitive.content}/local-source/${row["declarationStart"]!!.jsonPrimitive.int}:${row["declarationEnd"]!!.jsonPrimitive.int}"
                else -> throw Invalid()
            }
            if (row["variableIdentity"]?.jsonPrimitive?.content != expected
                || row["captured"]?.jsonPrimitive?.boolean != (lexical["identity"] != declared["identity"])) throw Invalid()
            val initializer = if (row.containsKey("initializerStart")) range(row,"initializerStart","initializerEnd") else null
            if (initializer != null && (initializer.first < declStart || initializer.second > declEnd)) throw Invalid()
            JsonObject(row.filterKeys { it !in setOf("recordType","file","ownerStart","ownerEnd","start","end","declarationStart","declarationEnd","initializerStart","initializerEnd","coordinateDomain") } + mapOf(
                "byteStart" to JsonPrimitive(start), "byteEnd" to JsonPrimitive(end),
                "declarationByteStart" to JsonPrimitive(declStart), "declarationByteEnd" to JsonPrimitive(declEnd),
                "lexicalRegion" to lexical, "declarationRegion" to declared,
            ) + if (initializer == null) emptyMap() else mapOf("initializerSpan" to buildJsonObject {
                put("byteStart", initializer.first); put("byteEnd", initializer.second); put("meaning", "SEPARATE_INITIALIZER_NOT_ACTUAL_FORMAL_BINDING")
            }))
        }
        if (variables.size > 4096) throw Invalid()
        val declarations = variables.filter { it["kind"]?.jsonPrimitive?.content == "VARIABLE_DECLARATION" }.groupBy { it["variableIdentity"] }
        if (declarations.any { it.value.size != 1 }) throw Invalid()
        variables.filter { it["kind"]?.jsonPrimitive?.content == "VARIABLE_ACCESS" }.forEach { row ->
            val declaration = declarations[row["variableIdentity"]]?.singleOrNull() ?: throw Invalid()
            if (declaration["declarationRegion"] != row["declarationRegion"] || declaration["declarationByteStart"] != row["declarationByteStart"] || declaration["declarationByteEnd"] != row["declarationByteEnd"]) throw Invalid()
        }
        val callRows = calls.filter { it["owner"] == descriptor["compilerCallableId"] }.mapNotNull { row ->
            val rawStart = row["start"]?.jsonPrimitive?.intOrNull ?: return@mapNotNull null
            val rawEnd = row["end"]?.jsonPrimitive?.intOrNull ?: return@mapNotNull null
            val location = coordinates.range(rawStart,rawEnd) ?: throw Invalid()
            if (location.first < ownerStart || location.last + 1 > ownerEnd) return@mapNotNull null
            val lexical = row["lexicalRegion"]?.let(::region)
            if (lexical != null && (location.first < lexical["byteStart"]!!.jsonPrimitive.int || location.last + 1 > lexical["byteEnd"]!!.jsonPrimitive.int)) throw Invalid()
            val target = row["target"]?.jsonPrimitive?.content
            val mapping = row["dataArgumentToParameter"] as? JsonArray
            val omitted = row["dataOmittedDefaultParameterIndices"] as? JsonArray
            val code = row["dataArgumentBoundary"]?.jsonPrimitive?.content
                ?: row["dataSemanticBoundary"]?.jsonPrimitive?.content
                ?: if (lexical == null) "COMPILER_LEXICAL_REGION_UNAVAILABLE" else if (target == null || mapping == null) "COMPILER_ARGUMENT_MAPPING_UNAVAILABLE"
                else if (mapping.any { it.jsonObject["actualStart"]?.jsonPrimitive?.intOrNull == null || it.jsonObject["actualEnd"]?.jsonPrimitive?.intOrNull == null }) "COMPILER_ACTUAL_SOURCE_WRAPPER_UNSUPPORTED" else null
            val arguments = if (code != null) emptyList() else mapping!!.mapIndexed { index, element ->
                val argument = element.jsonObject; val syntactic = range(argument,"argumentStart","argumentEnd")
                val actual = range(argument,"actualStart","actualEnd")
                val slot = argument["parameterIndex"]!!.jsonPrimitive.int
                if (slot < 0 || argument["evaluationIndex"]?.jsonPrimitive?.intOrNull != index
                    || actual.first < syntactic.first || actual.second > syntactic.second
                    || syntactic.first < location.first || syntactic.second > location.last + 1) throw Invalid()
                val references = variables.filter { it["kind"]?.jsonPrimitive?.content == "VARIABLE_ACCESS" && it["accessMode"]?.jsonPrimitive?.content == "READ"
                    && it["byteStart"]!!.jsonPrimitive.int == actual.first && it["byteEnd"]!!.jsonPrimitive.int == actual.second && it["lexicalRegion"] == lexical }
                if (references.size > 1) throw Invalid()
                buildJsonObject {
                    put("evaluationIndex", index); put("parameterIndex", slot); put("formalIdentity", "$target/parameter/$slot")
                    put("parameterName", argument["parameter"]!!)
                    put("argumentByteStart", syntactic.first); put("argumentByteEnd", syntactic.second)
                    put("actualByteStart", actual.first); put("actualByteEnd", actual.second)
                    put("resolution", "COMPILER_EXACT")
                    val reference = references.singleOrNull()
                    if (reference != null) {
                        put("actualVariableIdentity", reference["variableIdentity"]!!); put("declarationRegion",reference["declarationRegion"]!!)
                        put("declarationByteStart", reference["declarationByteStart"]!!); put("declarationByteEnd", reference["declarationByteEnd"]!!)
                        reference["initializerSpan"]?.let { put("initializerSpan", it) }
                        put("captured", reference["captured"]!!)
                    } else put("actualKind", "EXPRESSION_OCCURRENCE_NOT_VARIABLE_BINDING")
                }
            }
            if (arguments.zipWithNext().any { (a,b) -> a["argumentByteStart"]!!.jsonPrimitive.int >= b["argumentByteStart"]!!.jsonPrimitive.int }) throw Invalid()
            buildJsonObject {
                put("byteStart", location.first); put("byteEnd", location.last + 1)
                put("resolution", if (code == null) "COMPILER_EXACT" else "UNKNOWN")
                target?.let { put("targetIdentity", it) }; lexical?.let { put("lexicalRegion", it) }
                put("arguments", JsonArray(arguments)); put("receiverObjectIdentity", "UNKNOWN")
                put("evaluationOrderAuthority", "COMPILER_ARGUMENT_SOURCE_OCCURRENCES_NOT_CALLBACK_INVOCATION_ORDER")
                put("omittedDefaultParameterIndices", omitted ?: JsonArray(emptyList()))
                put("boundaries", JsonArray(listOfNotNull(code, if (!omitted.isNullOrEmpty()) "DEFAULT_ARGUMENT_EVALUATION_UNSUPPORTED" else null).map(::JsonPrimitive)))
            }
        }.distinct().groupBy { it["byteStart"] to it["byteEnd"] }.values.map { occurrences ->
            if (occurrences.size == 1) occurrences.single() else buildJsonObject {
                // FIR lowers iteration into several calls sharing one source range.
                // No arbitrary target can own that occurrence, but other calls and
                // compiler-selected returns remain independently available.
                put("byteStart", occurrences.first()["byteStart"]!!); put("byteEnd", occurrences.first()["byteEnd"]!!)
                put("resolution", "UNKNOWN"); put("arguments", JsonArray(emptyList()))
                put("receiverObjectIdentity", "UNKNOWN")
                put("evaluationOrderAuthority", "COMPILER_ARGUMENT_SOURCE_OCCURRENCES_NOT_CALLBACK_INVOCATION_ORDER")
                put("omittedDefaultParameterIndices", JsonArray(emptyList()))
                put("boundaries", JsonArray(listOf(JsonPrimitive("COMPILER_CALL_SOURCE_OCCURRENCE_AMBIGUOUS"))))
                put("candidateTargetIdentities", JsonArray(occurrences.mapNotNull { it["targetIdentity"] }.distinct().sortedBy { it.jsonPrimitive.content }))
            }
        }.sortedBy { it["byteStart"]!!.jsonPrimitive.int }
        val returns = selected.filter { it["recordType"]?.jsonPrimitive?.content == "DOCUMENTATION_RETURN_CONTROL" }.map { row ->
            if (row["schema"]?.jsonPrimitive?.content != "kotlin-documentation-return-control/1.0" || range(row,"ownerStart","ownerEnd") != ownerStart to ownerEnd) throw Invalid()
            val location = range(row)
            buildJsonObject {
                put("byteStart", location.first); put("byteEnd", location.second)
                put("resolution", row["resolution"]!!); put("returnValueProof", "SEPARATE_NOT_ASSERTED")
                if (row["resolution"]?.jsonPrimitive?.content == "COMPILER_EXACT") {
                    if (row["authority"]?.jsonPrimitive?.content != "K2_SELECTED_RETURN_TARGET") throw Invalid()
                    val lexical = region(row["lexicalRegion"]); val target = region(row["targetRegion"])
                    if (listOf(lexical,target).any { location.first < it["byteStart"]!!.jsonPrimitive.int || location.second > it["byteEnd"]!!.jsonPrimitive.int }) throw Invalid()
                    put("lexicalRegion", lexical); put("targetRegion", target); put("authority", row["authority"]!!)
                    put("nonLocal", lexical["identity"] != target["identity"])
                } else put("code", row["code"] ?: JsonPrimitive("COMPILER_RETURN_TARGET_UNAVAILABLE"))
            }
        }.distinct().sortedBy { it["byteStart"]!!.jsonPrimitive.int }
        if (callRows.size > 512 || returns.size > 512 || callRows.map { it["byteStart"] to it["byteEnd"] }.toSet().size != callRows.size) throw Invalid()
        val normalized = result("COMPILER_METADATA_NOT_EXECUTION_PROOF", variables, callRows, returns, boundaries)
        if (normalized.toString().toByteArray(Charsets.UTF_8).size > 524288) unavailable("SEMANTIC_INPUT_BUDGET") else normalized
    } catch (_: Invalid) { unavailable("SEMANTIC_INPUT_INVALID_SOURCE_OR_SYMBOL_BINDING") }
}

internal fun attachKotlinSemanticInputs(flow: JsonObject, semantic: JsonObject): JsonObject {
    val calls = semantic["calls"]!!.jsonArray.map { it.jsonObject }.groupBy { it["byteStart"] to it["byteEnd"] }
    val returns = semantic["returns"]!!.jsonArray.map { it.jsonObject }.groupBy { it["byteStart"] to it["byteEnd"] }
    val events = flow["events"]!!.jsonArray.map { raw ->
        val event = raw.jsonObject; val span = event["sourceSpan"]?.jsonObject
        val rows = if (event["kind"]?.jsonPrimitive?.content == "RETURN") returns else calls
        val selected = rows[span?.get("byteStart") to span?.get("byteEnd")]?.singleOrNull()
        if (selected == null || event["kind"]?.jsonPrimitive?.content !in setOf("RETURN","CALL","CONSTRUCT")) event
        else JsonObject(event + mapOf("semanticInput" to selected))
    }
    return JsonObject(flow + mapOf("events" to JsonArray(events), "semanticInputs" to semantic))
}
