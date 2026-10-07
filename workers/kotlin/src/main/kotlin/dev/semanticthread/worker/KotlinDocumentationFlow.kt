package dev.semanticthread.worker

import kotlinx.serialization.json.*
import org.jetbrains.kotlin.com.intellij.psi.PsiElement
import org.jetbrains.kotlin.com.intellij.psi.util.PsiTreeUtil
import org.jetbrains.kotlin.lexer.KtTokens
import org.jetbrains.kotlin.psi.*
import java.security.MessageDigest

/** Syntax structure is attached only to a matching retained compiler descriptor. */
internal class KotlinDocumentationSource(
    private val file: String,
    private val source: KtFile,
    private val originalSource: String,
    private val compilationScope: String,
    descriptors: List<JsonObject>,
    relations: List<JsonObject>,
    private val dataFacts: List<JsonObject> = emptyList(),
) {
    private val dataCalls = relations
    private val sourceDigest = "sha256:" + MessageDigest.getInstance("SHA-256")
        .digest(originalSource.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it) }
    // Kotlin PSI expects LF line separators. Use a documentation-only view;
    // original parser/edit offsets and retained source bytes keep their domain.
    private val documentationSource = if ('\r' in source.text) {
        KtPsiFactory(source.project, markGenerated = false).createFile(source.name, compilerLineNormalizedText(originalSource))
    } else source
    private val inputCoordinates = CompilerUtf16ToUtf8ByteMap.forSourceText(originalSource, source.text)
    private val psiCoordinates = CompilerUtf16ToUtf8ByteMap.forSourceText(originalSource, documentationSource.text)
    private val functions = PsiTreeUtil.collectElementsOfType(documentationSource, KtNamedFunction::class.java)
        .groupBy { function -> psiCoordinates?.range(function.textRange.startOffset, function.textRange.endOffset)?.let { it.first to it.last + 1 } }
    private val compilerCoordinates = CompilerUtf16ToUtf8ByteMap.fromCompilerInput(originalSource)
    private val declarations = descriptors.groupBy { it["start"]?.jsonPrimitive?.intOrNull to it["end"]?.jsonPrimitive?.intOrNull }
    private val calls = relations.mapNotNull { row ->
        val start = row["start"]?.jsonPrimitive?.intOrNull ?: return@mapNotNull null
        val end = row["end"]?.jsonPrimitive?.intOrNull ?: return@mapNotNull null
        val range = compilerCoordinates?.range(start, end) ?: return@mapNotNull null
        JsonObject(row + mapOf("start" to JsonPrimitive(range.first), "end" to JsonPrimitive(range.last + 1)))
    }.groupBy { it["owner"]?.jsonPrimitive?.content }

    fun enrich(declaration: KtNamedDeclaration, row: JsonObject): JsonObject {
        if (declaration !is KtNamedFunction || declaration.bodyExpression == null) return row
        val coordinates = psiCoordinates ?: return row
        val range = inputCoordinates?.range(declaration.textRange.startOffset, declaration.textRange.endOffset) ?: return row
        val function = functions[range.first to range.last + 1]?.singleOrNull() ?: return row
        val descriptor = declarations[range.first to range.last + 1]?.singleOrNull {
            it["declarationKind"]?.jsonPrimitive?.content == "FUNCTION"
        } ?: return row
        val identity = descriptor["symbolIdentity"]?.jsonPrimitive?.content ?: return row
        val flow = KotlinDocumentationFlow(file, documentationSource, sourceDigest, compilationScope, descriptor, coordinates, calls[descriptor["compilerCallableId"]?.jsonPrimitive?.content].orEmpty().filter {
            (it["start"]!!.jsonPrimitive.int >= range.first) && (it["end"]!!.jsonPrimitive.int <= range.last + 1)
        }).read(function, descriptor)
        val data = KotlinDocumentationData(file, originalSource, compilationScope, descriptor,
            coordinates, compilerCoordinates, dataFacts, dataCalls).read(function)
        val documented = if (data == null) flow else JsonObject(flow + mapOf("dataInput" to data))
        return JsonObject(row + mapOf("documentationSymbol" to JsonPrimitive(identity), "documentation" to documented))
    }
}

/** A bounded source outline, with exact FIR call targets; never a runtime trace. */
private class KotlinDocumentationFlow(
    private val file: String,
    private val source: KtFile,
    private val sourceDigest: String,
    private val compilationScope: String,
    private val descriptor: JsonObject,
    private val coordinates: CompilerUtf16ToUtf8ByteMap,
    relations: List<JsonObject>,
) : KtTreeVisitorVoid() {
    private val events = mutableListOf<JsonObject>()
    private val boundaries = sortedSetOf<String>()
    private val calls = relations.groupBy { it["start"]?.jsonPrimitive?.intOrNull to it["end"]?.jsonPrimitive?.intOrNull }
    private val lineStarts = listOf(0) + source.text.indices.filter { source.text[it] == '\n' }.map { it + 1 }
    private var depth = 0

    fun read(function: KtNamedFunction, descriptor: JsonObject): JsonObject {
        scan(function.bodyExpression)
        if (!function.hasBlockBody()) event("RETURN", function.bodyExpression ?: function)
        if (function.hasModifier(KtTokens.SUSPEND_KEYWORD)) boundaries += "COROUTINE_SCHEDULING_NOT_ESTABLISHED"
        return buildJsonObject {
            put("schema", "codeclew-kotlin-documentation-flow/1.0")
            put("authority", "KOTLIN_PSI_WITH_K2_CALL_TARGETS")
            put("parameterTypes", JsonArray(descriptor["parameterTypes"]?.jsonArray.orEmpty().map { parameter ->
                JsonPrimitive(parameter.jsonObject["type"]!!.jsonPrimitive.content.replace('/', '.'))
            }))
            put("events", JsonArray(events))
            put("boundaries", JsonArray(boundaries.map(::JsonPrimitive)))
        }
    }

    private fun scan(element: PsiElement?) {
        if (element == null) return
        if (depth >= 64 || events.size >= 512) {
            boundaries += "DOCUMENTATION_FLOW_EVENT_BUDGET"
            return
        }
        depth++
        element.accept(this)
        depth--
    }

    override fun visitElement(element: PsiElement) {
        element.children.forEach(::scan)
    }

    private fun line(offset: Int): Int {
        val index = lineStarts.binarySearch(offset)
        return if (index >= 0) index + 1 else -index - 1
    }

    private fun event(kind: String, element: PsiElement, attributes: Map<String, JsonElement> = emptyMap()) {
        if (events.size >= 512) {
            boundaries += "DOCUMENTATION_FLOW_EVENT_BUDGET"
            return
        }
        events += buildJsonObject {
            put("kind", kind)
            put("file", file)
            put("startLine", line(element.textRange.startOffset))
            put("endLine", line((element.textRange.endOffset - 1).coerceAtLeast(element.textRange.startOffset)))
            val range = coordinates.range(element.textRange.startOffset, element.textRange.endOffset)
            if (range != null && !range.isEmpty()) {
                put("sourceSpan", buildJsonObject {
                    put("schema", "codeclew-documentation-source-span/1.0")
                    put("coordinateDomain", "ORIGINAL_UTF8_BYTES")
                    put("ownerSymbolIdentity", descriptor["symbolIdentity"]!!)
                    put("compilationScope", compilationScope)
                    put("file", file)
                    put("ordinal", events.size)
                    put("ownerByteStart", descriptor["start"]!!)
                    put("ownerByteEnd", descriptor["end"]!!)
                    put("byteStart", range.first)
                    put("byteEnd", range.last + 1)
                    put("fullCompilationSourceDigest", sourceDigest)
                })
            }
            attributes.forEach { (key, value) -> put(key, value) }
        }
    }

    private fun boundary(code: String, element: PsiElement) {
        boundaries += code
        event("BOUNDARY", element, mapOf("code" to JsonPrimitive(code)))
    }

    override fun visitNamedFunction(function: KtNamedFunction) = boundary("LOCAL_FUNCTION_EXECUTION_NOT_EXPANDED", function)
    override fun visitClassOrObject(classOrObject: KtClassOrObject) = boundary("LOCAL_CLASS_BODY_NOT_EXPANDED", classOrObject)
    override fun visitLambdaExpression(lambdaExpression: KtLambdaExpression) {
        boundaries += "CALLBACK_INVOCATION_ORDER_AND_COUNT_NOT_ESTABLISHED"
        event("DEFERRED", lambdaExpression, mapOf("condition" to JsonPrimitive("If this callback is invoked"), "invocation" to JsonPrimitive("NOT_ESTABLISHED")))
        scan(lambdaExpression.bodyExpression)
        event("END", lambdaExpression)
    }

    override fun visitTryExpression(expression: KtTryExpression) {
        boundaries += "EXCEPTION_TYPES_AND_DISPATCH_NOT_ESTABLISHED"
        event("TRY", expression.tryBlock, mapOf("condition" to JsonPrimitive("Normal processing or a caught failure")))
        scan(expression.tryBlock)
        expression.catchClauses.forEach { clause ->
            event("ELSE", clause, mapOf("condition" to JsonPrimitive("catch ${clause.catchParameter?.typeReference?.text.orEmpty()}")))
            scan(clause.catchBody)
        }
        event("END", expression)
        expression.finallyBlock?.finalExpression?.let { block ->
            event("FINALLY", block)
            scan(block)
        }
    }

    override fun visitBreakExpression(expression: KtBreakExpression) = event("BREAK", expression)
    override fun visitContinueExpression(expression: KtContinueExpression) = event("CONTINUE", expression)

    override fun visitIfExpression(expression: KtIfExpression) {
        scan(expression.condition)
        event("IF", expression.condition ?: expression, mapOf("condition" to JsonPrimitive(expression.condition?.text.orEmpty())))
        scan(expression.then)
        expression.`else`?.let { event("ELSE", it); scan(it) }
        event("END", expression)
    }

    override fun visitWhenExpression(expression: KtWhenExpression) {
        scan(expression.subjectExpression)
        expression.entries.forEachIndexed { index, entry ->
            val condition = if (entry.isElse) "otherwise" else entry.conditions.joinToString(", ") { it.text }
            event(if (index == 0) "IF" else "ELSE", entry, mapOf("condition" to JsonPrimitive(condition), "subject" to JsonPrimitive(expression.subjectExpression?.text.orEmpty())))
            scan(entry.expression)
        }
        if (expression.entries.isNotEmpty()) event("END", expression)
    }

    override fun visitForExpression(expression: KtForExpression) {
        scan(expression.loopRange)
        event("LOOP", expression, mapOf("condition" to JsonPrimitive("for each ${expression.loopParameter?.text.orEmpty()} in ${expression.loopRange?.text.orEmpty()}")))
        scan(expression.body)
        event("END", expression)
    }

    override fun visitWhileExpression(expression: KtWhileExpression) {
        event("LOOP", expression.condition ?: expression, mapOf("condition" to JsonPrimitive(expression.condition?.text.orEmpty())))
        scan(expression.condition)
        scan(expression.body)
        event("END", expression)
    }

    override fun visitDoWhileExpression(expression: KtDoWhileExpression) {
        event("LOOP", expression, mapOf("condition" to JsonPrimitive("do-while ${expression.condition?.text.orEmpty()}")))
        scan(expression.body)
        scan(expression.condition)
        event("END", expression)
    }

    override fun visitBinaryExpression(expression: KtBinaryExpression) {
        if (expression.operationToken in setOf(KtTokens.ELVIS, KtTokens.ANDAND, KtTokens.OROR)) {
            scan(expression.left)
            val condition = when (expression.operationToken) {
                KtTokens.ELVIS -> "${expression.left?.text.orEmpty()} is null"
                KtTokens.ANDAND -> "${expression.left?.text.orEmpty()} is true"
                else -> "${expression.left?.text.orEmpty()} is false"
            }
            event("IF", expression, mapOf("condition" to JsonPrimitive(condition)))
            scan(expression.right)
            event("END", expression)
        } else {
            super.visitBinaryExpression(expression)
            // Retain the assignment's source occurrence, without deriving
            // storage identity, value flow or an effect from PSI syntax.
            if (KtTokens.ALL_ASSIGNMENTS.contains(expression.operationToken)) event("STATEMENT", expression)
        }
    }

    override fun visitSafeQualifiedExpression(expression: KtSafeQualifiedExpression) {
        scan(expression.receiverExpression)
        event("IF", expression, mapOf("condition" to JsonPrimitive("${expression.receiverExpression.text} is not null")))
        scan(expression.selectorExpression)
        event("END", expression)
    }

    override fun visitCallExpression(expression: KtCallExpression) {
        expression.valueArguments.mapNotNull { it.getArgumentExpression() }.filterNot { it is KtLambdaExpression }.forEach(::scan)
        val qualified = (expression.parent as? KtQualifiedExpression)?.takeIf { it.selectorExpression === expression }
        val candidates = listOfNotNull(qualified, expression).flatMap { element ->
            val range = coordinates.range(element.textRange.startOffset, element.textRange.endOffset)
            calls[range?.first to range?.let { it.last + 1 }].orEmpty()
        }.distinct()
        val target = candidates.mapNotNull { it["target"]?.jsonPrimitive?.content }.distinct().singleOrNull()
        if (target == null) boundaries += "DOCUMENTATION_CALL_TARGET_UNRESOLVED"
        event(if (candidates.any { it["kind"]?.jsonPrimitive?.content == "CONSTRUCTS" }) "CONSTRUCT" else "CALL", qualified ?: expression,
            if (target == null) emptyMap() else mapOf("target" to JsonPrimitive(target), "resolution" to JsonPrimitive("COMPILER_EXACT")) + transport(expression, target))
        (expression.valueArguments.mapNotNull { it.getArgumentExpression() as? KtLambdaExpression } + expression.lambdaArguments.mapNotNull { it.getLambdaExpression() }).distinct().forEach(::scan)
    }

    private fun transport(expression: KtCallExpression, target: String): Map<String, JsonElement> {
        val callable = target.substringAfter(':').substringBefore("#jvm:").replace('/', '.')
        if (callable !in setOf("org.springframework.kafka.core.KafkaTemplate.send", "org.springframework.kafka.core.KafkaOperations.send")) return emptyMap()
        val argument = expression.valueArguments.firstOrNull()?.getArgumentExpression() ?: return emptyMap()
        val literal = (argument as? KtStringTemplateExpression)?.entries
            ?.takeIf { entries -> entries.all { it is KtLiteralStringTemplateEntry } }
            ?.joinToString("") { it.text }
        return mapOf("kafka" to buildJsonObject {
            put("adapter", "SPRING_KAFKA_LITERAL_TOPIC/1.0")
            put("topicExpression", argument.text)
            literal?.let { put("topic", it) }
        })
    }

    override fun visitReturnExpression(expression: KtReturnExpression) {
        scan(expression.returnedExpression)
        event("RETURN", expression)
    }

    override fun visitThrowExpression(expression: KtThrowExpression) {
        scan(expression.thrownExpression)
        event("THROW", expression)
    }

    override fun visitProperty(property: KtProperty) {
        scan(property.initializer)
        event("LOCAL", property)
    }
}
