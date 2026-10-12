package dev.semanticthread.worker

import kotlinx.serialization.json.*
import org.jetbrains.kotlin.com.intellij.psi.PsiElement
import org.jetbrains.kotlin.lexer.KtTokens
import org.jetbrains.kotlin.psi.*
import java.security.MessageDigest

/** Normalizes syntax and compiler occurrences; contains no transfer engine. */
internal class KotlinDocumentationData(
    private val file: String,
    private val original: String,
    private val scope: String,
    private val descriptor: JsonObject,
    private val psi: CompilerUtf16ToUtf8ByteMap,
    private val compiler: CompilerUtf16ToUtf8ByteMap?,
    facts: List<JsonObject>,
    calls: List<JsonObject>,
    private val descriptors: List<JsonObject>,
) {
    private val owner = descriptor["symbolIdentity"]!!.jsonPrimitive.content
    private val ownerStart = descriptor["start"]!!.jsonPrimitive.int
    private val ownerEnd = descriptor["end"]!!.jsonPrimitive.int
    private val sourceDigest = "sha256:" + MessageDigest.getInstance("SHA-256")
        .digest(original.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it) }
    private val records = facts.filter { it["ownerSymbolIdentity"]?.jsonPrimitive?.content == owner }
    private val callRecords = calls.mapNotNull { row ->
        val start = row["start"]?.jsonPrimitive?.intOrNull ?: return@mapNotNull null
        val end = row["end"]?.jsonPrimitive?.intOrNull ?: return@mapNotNull null
        val range = compiler?.range(start, end) ?: return@mapNotNull null
        if (range.first < ownerStart || range.last + 1 > ownerEnd) return@mapNotNull null
        (range.first to range.last + 1) to row
    }.groupBy({ it.first }, { it.second })
    private val nodes = mutableListOf<JsonObject>()
    private val variables = mutableListOf<JsonObject>()
    private val members = mutableListOf<JsonObject>()
    private val accessorAttempts = mutableListOf<JsonObject>()
    private val stringConstants = mutableListOf<JsonObject>()
    private val enumEntries = mutableListOf<JsonObject>()
    private val operations = mutableMapOf<Pair<Int, Int>, String>()
    private val boundaries = sortedSetOf<String>()
    private val variableTypes = mutableMapOf<Pair<Int, Int>, String>()
    private var depth = 0

    private class Unavailable : RuntimeException()

    fun read(function: KtNamedFunction): JsonObject? {
        val receipt = records.filter { it["recordType"]?.jsonPrimitive?.content == "DOCUMENTATION_VARIABLE_RECEIPT" }.singleOrNull()
            ?: return null
        return try {
            if (receipt["schema"]?.jsonPrimitive?.content != "kotlin-documentation-variable-receipt/1.0"
                || receipt["authority"]?.jsonPrimitive?.content != "K2_RESOLVED_VARIABLE_SYMBOL"
                || compilerRange(receipt, "ownerStart", "ownerEnd") != ownerStart to ownerEnd
            ) throw Unavailable()
            for (row in records) {
                when (row["recordType"]?.jsonPrimitive?.content) {
                    "DOCUMENTATION_ENUM_ENTRY" -> {
                        if (row["schema"]?.jsonPrimitive?.content != "kotlin-documentation-enum-entry/1.0"
                            || row["authority"]?.jsonPrimitive?.content != "K2_RESOLVED_ENUM_ENTRY_SYMBOL"
                            || row["resolution"]?.jsonPrimitive?.content != "COMPILER_EXACT"
                            || compilerRange(row, "ownerStart", "ownerEnd") != ownerStart to ownerEnd) throw Unavailable()
                        val span = compilerRange(row, "start", "end")
                        if (span.first < ownerStart || span.second > ownerEnd || span.first >= span.second) throw Unavailable()
                        enumEntries += buildJsonObject {
                            put("schema", "kotlin-documentation-enum-entry/1.0"); put("ownerSymbolIdentity", owner)
                            put("enumClassIdentity", row["enumClassIdentity"]!!); put("entryIdentity", row["entryIdentity"]!!)
                            put("entryName", row["entryName"]!!); put("authority", row["authority"]!!)
                            put("resolution", row["resolution"]!!); put("compilationScope", scope)
                            put("byteStart", span.first); put("byteEnd", span.second)
                        }
                    }
                    "DOCUMENTATION_DATA_BOUNDARY" -> {
                        val code = row["code"]!!.jsonPrimitive.content
                        if (code in setOf("DOCUMENTATION_VARIABLE_BUDGET", "DOCUMENTATION_VARIABLE_SOURCE_UNAVAILABLE")) throw Unavailable()
                        boundaries += code
                    }
                    "DOCUMENTATION_OPERATION" -> {
                        if (row["schema"]?.jsonPrimitive?.content != "kotlin-documentation-operation/1.0"
                            || row["authority"]?.jsonPrimitive?.content != "K2_RESOLVED_OPERAND_TYPES"
                            || compilerRange(row, "ownerStart", "ownerEnd") != ownerStart to ownerEnd) throw Unavailable()
                        operations[compilerRange(row, "start", "end")] = row["proof"]!!.jsonPrimitive.content
                    }
                    "DOCUMENTATION_MEMBER", "DOCUMENTATION_ACCESSOR_ATTEMPT", "DOCUMENTATION_STRING_CONSTANT" -> {
                        val constant = row["recordType"]?.jsonPrimitive?.content == "DOCUMENTATION_STRING_CONSTANT"
                        val accessor = row["recordType"]?.jsonPrimitive?.content == "DOCUMENTATION_ACCESSOR_ATTEMPT"
                        val schema = if (constant) "kotlin-documentation-string-constant/1.0" else if (accessor) "kotlin-documentation-accessor-attempt/1.0" else "kotlin-documentation-member/1.0"
                        val authority = if (constant) "K2_CONST_PROPERTY_LITERAL_STRING" else if (accessor) "K2_RESOLVED_PROPERTY_SYMBOL_NOT_ACCESSOR_DISPATCH" else "K2_RESOLVED_PROPERTY_SYMBOL"
                        if (row["schema"]?.jsonPrimitive?.content != schema
                            || row["authority"]?.jsonPrimitive?.content != authority
                            || row["resolution"]?.jsonPrimitive?.content != "COMPILER_EXACT"
                            || compilerRange(row, "ownerStart", "ownerEnd") != ownerStart to ownerEnd) throw Unavailable()
                        val property = descriptors.singleOrNull { it["symbolIdentity"] == row["propertyIdentity"]
                            && if (constant) it["documentationStringConstant"]?.jsonObject?.get("value") == row["stringConstant"]
                            else it["documentationStorage"]?.jsonObject?.get("qualified")?.jsonPrimitive?.boolean == !accessor
                            } ?: continue
                        val span = compilerRange(row, "start", "end")
                        if (span.first < ownerStart || span.second > ownerEnd || span.first >= span.second) throw Unavailable()
                        val selected = if (constant) stringConstants else if (accessor) accessorAttempts else members
                        selected += buildJsonObject {
                            put("schema", schema); put("ownerSymbolIdentity", owner)
                            if (accessor || constant) put("authority", authority)
                            if (constant) put("stringConstant", row["stringConstant"]!!)
                            put("propertyIdentity", row["propertyIdentity"]!!); put("accessMode", row["accessMode"]!!)
                            put("variableType", row["variableType"]!!); put("byteStart", span.first); put("byteEnd", span.second)
                            put("propertyByteStart", property["start"]!!); put("propertyByteEnd", property["end"]!!)
                            put("propertyFile", property["file"]!!); put("compilationScope", scope)
                            put("storageProof", property["documentationStorage"]!!)
                        }
                        variableTypes[span] = row["variableType"]!!.jsonPrimitive.content
                    }
                    "DOCUMENTATION_VARIABLE" -> {
                        if (row["schema"]?.jsonPrimitive?.content != "kotlin-documentation-variable/1.0"
                            || row["resolution"]?.jsonPrimitive?.content != "COMPILER_EXACT"
                            || row["authority"]?.jsonPrimitive?.content != "K2_RESOLVED_VARIABLE_SYMBOL"
                            || compilerRange(row, "ownerStart", "ownerEnd") != ownerStart to ownerEnd
                        ) throw Unavailable()
                        val range = compilerRange(row, "start", "end")
                        val declaration = compilerRange(row, "declarationStart", "declarationEnd")
                        for (span in listOf(range, declaration)) {
                            if (span.first < ownerStart || span.second > ownerEnd || span.first >= span.second) throw Unavailable()
                        }
                        val retained = JsonObject(row.filterKeys { it !in setOf("recordType", "file", "start", "end", "ownerStart", "ownerEnd", "declarationStart", "declarationEnd", "coordinateDomain") } + mapOf(
                            "byteStart" to JsonPrimitive(range.first), "byteEnd" to JsonPrimitive(range.second),
                            "declarationByteStart" to JsonPrimitive(declaration.first), "declarationByteEnd" to JsonPrimitive(declaration.second),
                        ))
                        variables += retained
                        row["variableType"]?.jsonPrimitive?.content?.let { variableTypes[range] = it }
                    }
                }
            }
            if (variables.size + members.size + accessorAttempts.size + stringConstants.size + enumEntries.size > 4096 || function.hasModifier(KtTokens.SUSPEND_KEYWORD) || function.receiverTypeReference != null) throw Unavailable()
            val body = function.bodyExpression ?: return null
            val root = if (function.hasBlockBody()) statement(body) else add("RETURN", body, listOf(expression(body)))
            // An opaque expression (for example an interpolated string) does
            // not expose its internal evaluation order. Keep its stop node;
            // do not promote compiler occurrences inside it as transfer nodes.
            fun activeRanges(kind: String) = nodes.filter { it["kind"]!!.jsonPrimitive.content == kind }
                .map { it["byteStart"]!!.jsonPrimitive.int to it["byteEnd"]!!.jsonPrimitive.int }.toSet()
            val activeAccessors = activeRanges("ACCESSOR")
            val activeEnums = activeRanges("ENUM_CONSTANT")
            val activeConstants = activeRanges("STRING_CONSTANT")
            buildJsonObject {
                put("schema", "codeclew-kotlin-documentation-data/1.3")
                put("authority", "KOTLIN_PSI_WITH_K2_VARIABLE_IDENTITIES")
                put("coordinateDomain", "ORIGINAL_UTF8_BYTES")
                put("ownerSymbolIdentity", owner)
                put("ownerByteStart", ownerStart); put("ownerByteEnd", ownerEnd)
                put("file", file); put("compilationScope", scope)
                put("fullCompilationSourceDigest", sourceDigest)
                put("body", root); put("nodes", JsonArray(nodes)); put("variables", JsonArray(variables)); put("members", JsonArray(members))
                put("enumEntries", JsonArray(enumEntries.filter { it["byteStart"]!!.jsonPrimitive.int to it["byteEnd"]!!.jsonPrimitive.int in activeEnums }))
                put("stringConstants", JsonArray(stringConstants.filter { it["byteStart"]!!.jsonPrimitive.int to it["byteEnd"]!!.jsonPrimitive.int in activeConstants }))
                put("accessorAttempts", JsonArray(accessorAttempts.filter { it["byteStart"]!!.jsonPrimitive.int to it["byteEnd"]!!.jsonPrimitive.int in activeAccessors }))
                put("boundaries", JsonArray(boundaries.map(::JsonPrimitive)))
            }
        } catch (_: Unavailable) {
            null // No partially normalized input is promoted.
        }
    }

    private fun compilerRange(row: JsonObject, start: String, end: String): Pair<Int, Int> {
        val range = compiler?.range(row[start]?.jsonPrimitive?.intOrNull ?: throw Unavailable(),
            row[end]?.jsonPrimitive?.intOrNull ?: throw Unavailable()) ?: throw Unavailable()
        return range.first to range.last + 1
    }
    private fun range(element: PsiElement): Pair<Int, Int> {
        val range = psi.range(element.textRange.startOffset, element.textRange.endOffset) ?: throw Unavailable()
        if (range.first < ownerStart || range.last + 1 > ownerEnd || range.isEmpty()) throw Unavailable()
        return range.first to range.last + 1
    }
    private fun add(kind: String, element: PsiElement, children: List<Int> = emptyList(),
        roles: Map<String, Int> = emptyMap(), actuals: List<JsonObject> = emptyList(), defaults: List<JsonElement> = emptyList(),
        target: String? = null, staticQualifier: JsonObject? = null): Int {
        if (nodes.size >= 4096) throw Unavailable()
        val span = range(element)
        val index = nodes.size
        nodes += buildJsonObject {
            put("kind", kind); put("byteStart", span.first); put("byteEnd", span.second)
            put("children", JsonArray(children.map(::JsonPrimitive)))
            put("roles", JsonObject(roles.mapValues { JsonPrimitive(it.value) }))
            put("actuals", JsonArray(actuals)); put("defaultArguments", JsonArray(defaults))
            target?.let { put("targetIdentity", it) }
            staticQualifier?.let { put("staticQualifier", it) }
        }
        return index
    }
    private fun bounded(block: () -> Int): Int {
        if (depth >= 128) throw Unavailable()
        depth++
        return try { block() } finally { depth-- }
    }
    private fun statement(element: KtExpression): Int = bounded {
        when (element) {
            is KtBlockExpression -> add("BLOCK", element, element.statements.map(::statement))
            is KtIfExpression -> {
                val condition = element.condition ?: return@bounded add("UNSUPPORTED", element)
                val roles = mutableMapOf("CONDITION" to expression(condition))
                element.then?.let { roles["THEN"] = statement(it) }
                element.`else`?.let { roles["ELSE"] = statement(it) }
                add("IF", element, roles.values.toList(), roles)
            }
            is KtProperty -> {
                if (element.hasDelegate() || !element.isLocal) return@bounded add("UNSUPPORTED", element)
                val roles = element.initializer?.let { mapOf("INITIALIZER" to expression(it)) }.orEmpty()
                val declarator = add("DECLARATOR", element, roles.values.toList(), roles)
                add("LOCAL", element, listOf(declarator))
            }
            is KtReturnExpression -> {
                if (element.getTargetLabel() != null) return@bounded add("UNSUPPORTED", element)
                add("RETURN", element, listOfNotNull(element.returnedExpression?.let(::expression)))
            }
            is KtBinaryExpression -> {
                if (element.operationToken == KtTokens.EQ
                    && (element.left is KtNameReferenceExpression || element.left is KtDotQualifiedExpression)
                    && element.right != null) {
                    val roles = mapOf("LEFT" to expression(element.left!!), "RIGHT" to expression(element.right!!),
                        "OPERATOR" to add("UNSUPPORTED", element.operationReference))
                    add("EXPRESSION", element, listOf(add("ASSIGNMENT", element, roles.values.toList(), roles)))
                } else add("EXPRESSION", element, listOf(expression(element)))
            }
            else -> add("EXPRESSION", element, listOf(expression(element)))
        }
    }
    private fun type(element: KtExpression): String? = when (element) {
        is KtNameReferenceExpression -> variableTypes[range(element)]
        is KtDotQualifiedExpression, is KtCallExpression -> variableTypes[range(element)]
            ?: callRecords[range(element)]?.singleOrNull()?.get("dataResultType")?.jsonPrimitive?.content
        is KtStringTemplateExpression -> if (element.entries.all { it is KtLiteralStringTemplateEntry || it is KtEscapeStringTemplateEntry }) "kotlin/String" else null
        else -> null
    }
    private fun expression(element: KtExpression): Int = bounded {
        if (stringConstants.any { it["byteStart"]!!.jsonPrimitive.int == range(element).first && it["byteEnd"]!!.jsonPrimitive.int == range(element).second }) return@bounded add("STRING_CONSTANT", element)
        if (enumEntries.any { it["byteStart"]!!.jsonPrimitive.int == range(element).first
                && it["byteEnd"]!!.jsonPrimitive.int == range(element).second }) {
            return@bounded add("ENUM_CONSTANT", element)
        }
        when (element) {
            is KtNameReferenceExpression -> {
                val span = range(element)
                if (accessorAttempts.any { it["byteStart"]!!.jsonPrimitive.int == span.first && it["byteEnd"]!!.jsonPrimitive.int == span.second }) return@bounded add("ACCESSOR", element)
                val exact = (variables + members).any { it["byteStart"]!!.jsonPrimitive.int == span.first && it["byteEnd"]!!.jsonPrimitive.int == span.second }
                add(if (exact) "VARIABLE" else "UNSUPPORTED", element)
            }
            is KtThisExpression -> add(if (element.getTargetLabel() == null) "THIS" else "UNSUPPORTED", element)
            is KtConstantExpression -> add("LITERAL", element)
            is KtStringTemplateExpression -> add(if (type(element) != null) "LITERAL" else "UNSUPPORTED", element)
            is KtParenthesizedExpression -> add("PARENTHESIZED", element, listOfNotNull(element.expression?.let(::expression)))
            is KtUnaryExpression -> {
                val operand = element.baseExpression
                if (element.operationToken != KtTokens.EXCL || operand == null || type(operand) != "kotlin/Boolean") return@bounded add("UNSUPPORTED", element)
                val roles = mapOf("OPERAND" to expression(operand), "OPERATOR" to add("UNSUPPORTED", element.operationReference))
                add("UNARY", element, roles.values.toList(), roles)
            }
            is KtBinaryExpression -> {
                val left = element.left; val right = element.right
                if (left == null || right == null) return@bounded add("UNSUPPORTED", element)
                val nullCompare = element.operationToken in setOf(KtTokens.EQEQ, KtTokens.EXCLEQ)
                    && listOf(left, right).any { it is KtConstantExpression && it.text == "null" }
                val stringConcat = element.operationToken == KtTokens.PLUS && type(left) == "kotlin/String" && type(right) == "kotlin/String"
                val intEquality = element.operationToken in setOf(KtTokens.EQEQ, KtTokens.EXCLEQ)
                    && operations[range(element)] == "NON_NULL_INT_EQUALITY"
                val intOrder = element.operationToken in setOf(KtTokens.GT, KtTokens.GTEQ, KtTokens.LT, KtTokens.LTEQ)
                    && operations[range(element)] == "NON_NULL_INT_ORDER"
                if (!nullCompare && !stringConcat && !intEquality && !intOrder) return@bounded add("UNSUPPORTED", element)
                val roles = mapOf("LEFT" to expression(left), "RIGHT" to expression(right),
                    "OPERATOR" to add("UNSUPPORTED", element.operationReference))
                add("BINARY", element, roles.values.toList(), roles)
            }
            is KtCallExpression -> call(element, element, null)
            is KtDotQualifiedExpression -> {
                val selector = element.selectorExpression as? KtCallExpression
                if (selector != null) call(element, selector, element.receiverExpression)
                else if (accessorAttempts.any { it["byteStart"]!!.jsonPrimitive.int == range(element).first && it["byteEnd"]!!.jsonPrimitive.int == range(element).second }) {
                    val receiver = expression(element.receiverExpression)
                    add("ACCESSOR", element, listOf(receiver), mapOf("RECEIVER" to receiver))
                }
                else if (members.any { it["byteStart"]!!.jsonPrimitive.int == range(element).first && it["byteEnd"]!!.jsonPrimitive.int == range(element).second }) {
                    val receiver = expression(element.receiverExpression)
                    add("MEMBER", element, listOf(receiver), mapOf("RECEIVER" to receiver))
                } else add("UNSUPPORTED", element)
            }
            else -> add("UNSUPPORTED", element)
        }
    }
    private fun call(site: KtExpression, call: KtCallExpression, receiver: KtExpression?): Int {
        if (call.lambdaArguments.isNotEmpty() || call.valueArguments.any { it.getSpreadElement() != null }) return add("UNSUPPORTED", site)
        val rows = callRecords[range(site)].orEmpty().distinct()
        val selected = rows.singleOrNull { it["resolution"]?.jsonPrimitive?.content == "COMPILER_EXACT" }
            ?: return add("UNSUPPORTED", site)
        val kind = when (selected["kind"]?.jsonPrimitive?.content) {
            "CALLS" -> "CALL"
            "CONSTRUCTS" -> "CONSTRUCT"
            else -> return add("UNSUPPORTED", site)
        }
        if (selected.containsKey("dataArgumentBoundary") || selected["dataArgumentToParameter"] !is JsonArray || selected["dataOmittedDefaultParameterIndices"] !is JsonArray) {
            return add("UNSUPPORTED", site)
        }
        val qualifier = if (receiver != null && selected["dataStaticQualifierAuthority"]?.jsonPrimitive?.content == "K2_STATIC_FUNCTION_RESOLVED_CLASS_QUALIFIER"
            && compilerRange(selected, "dataStaticQualifierStart", "dataStaticQualifierEnd") == range(receiver)) buildJsonObject {
                put("schema", "kotlin-documentation-static-qualifier/1.0"); put("authority", selected["dataStaticQualifierAuthority"]!!)
                put("classIdentity", selected["dataStaticQualifierClass"]!!)
                put("byteStart", range(receiver).first); put("byteEnd", range(receiver).second)
            } else null
        val roles = if (qualifier != null) emptyMap() else receiver?.let { mapOf("RECEIVER" to expression(it)) }.orEmpty()
        val mapping = selected["dataArgumentToParameter"]?.jsonArray.orEmpty().map { it.jsonObject }
        val actuals = call.valueArguments.map { argument ->
            val expr = argument.getArgumentExpression() ?: throw Unavailable()
            val index = expression(expr)
            val ranges = setOf(range(argument.asElement()), range(expr))
            val binding = mapping.singleOrNull { compilerRange(it, "argumentStart", "argumentEnd") in ranges }
            buildJsonObject {
                put("expression", index)
                binding?.get("parameterIndex")?.let { put("formalSlot", it) }
            }
        }
        return add(kind, site, roles.values.toList() + actuals.map { it["expression"]!!.jsonPrimitive.int }, roles,
            actuals, selected["dataOmittedDefaultParameterIndices"]?.jsonArray.orEmpty(), selected["target"]?.jsonPrimitive?.content, qualifier)
    }
}
