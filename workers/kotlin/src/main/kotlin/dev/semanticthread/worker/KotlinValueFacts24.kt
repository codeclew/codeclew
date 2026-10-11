@file:OptIn(org.jetbrains.kotlin.fir.symbols.SymbolInternals::class)

package dev.semanticthread.worker

import kotlinx.serialization.json.*
import org.jetbrains.kotlin.KtRealSourceElementKind
import org.jetbrains.kotlin.KtSourceElement
import org.jetbrains.kotlin.fir.declarations.FirNamedFunction
import org.jetbrains.kotlin.fir.declarations.FirProperty
import org.jetbrains.kotlin.fir.expressions.*
import org.jetbrains.kotlin.fir.references.FirResolvedNamedReference
import org.jetbrains.kotlin.fir.symbols.FirBasedSymbol
import java.util.IdentityHashMap

/** K2 2.4 only: linear immutable locals, literal nullness and null-only Elvis. */
internal fun kotlinValueProjection24(owner: FirNamedFunction, file: String, identity: String): JsonObject? {
    val source = owner.source ?: return null
    val definitions = IdentityHashMap<FirBasedSymbol<*>, String>()
    val nodes = sortedMapOf<String, JsonObject>()
    class Unsupported : RuntimeException()
    var depth = 0
    fun operation(kind: String, operands: Map<String, String> = emptyMap()) = buildJsonObject {
        put("kind", kind)
        operands.forEach { (key, value) -> put(key, value) }
    }
    fun add(span: KtSourceElement?, op: JsonObject, root: Boolean = false): String {
        if (span == null || span.kind !== KtRealSourceElementKind || span.startOffset < source.startOffset
            || span.endOffset > source.endOffset || span.startOffset >= span.endOffset || nodes.size >= 128) throw Unsupported()
        val id = if (root) "result" else "v${span.startOffset}_${span.endOffset}"
        val row = buildJsonObject {
            put("id", id); put("operation", op)
            put("guards", JsonArray(if (op["kind"]?.jsonPrimitive?.content == "COALESCE") listOf(op["left"]!!) else emptyList()))
            put("start", span.startOffset); put("end", span.endOffset)
        }
        if (nodes.putIfAbsent(id, row)?.let { it != row } == true) throw Unsupported()
        return id
    }
    fun expression(expr: FirExpression): String {
        if (depth >= 64) throw Unsupported()
        depth++
        try {
            // Preserve the resolved original occurrence. Smart-cast type facts
            // do not refine this nullness profile or create new definitions.
            if (expr is FirSmartCastExpression) return expression(expr.originalExpression)
            val op = when (expr) {
                is FirLiteralExpression -> operation(if (expr.value == null) "NULL" else "NON_NULL")
                is FirElvisExpression -> operation("COALESCE", mapOf("left" to expression(expr.lhs), "right" to expression(expr.rhs)))
                is FirPropertyAccessExpression -> {
                    val symbol = (expr.calleeReference as? FirResolvedNamedReference)?.resolvedSymbol
                    val definition = symbol?.let(definitions::get)
                    if (definition == null || expr.explicitReceiver != null) operation("UNKNOWN", mapOf("boundary" to "KOTLIN_VALUE_EXPRESSION_UNSUPPORTED"))
                    else operation("COPY", mapOf("value" to definition))
                }
                else -> operation("UNKNOWN", mapOf("boundary" to "KOTLIN_VALUE_EXPRESSION_UNSUPPORTED"))
            }
            return add(expr.source, op)
        } finally { depth-- }
    }
    try {
        owner.valueParameters.forEach { parameter ->
            // A default expression is not a fixed incoming argument.
            definitions[parameter.symbol] = add(parameter.source, operation("INPUT"))
        }
        val statements = owner.body?.statements ?: throw Unsupported()
        if (statements.isEmpty() || statements.size > 128) throw Unsupported()
        statements.dropLast(1).forEach { statement ->
            val local = statement as? FirProperty ?: throw Unsupported()
            if (!local.isLocal || local.isVar || local.delegate != null) throw Unsupported()
            val initializer = local.initializer ?: throw Unsupported()
            val value = expression(initializer)
            definitions[local.symbol] = add(local.source, operation("COPY", mapOf("value" to value)))
        }
        val returned = statements.last() as? FirReturnExpression ?: throw Unsupported()
        if (returned.target.labeledElement !== owner) throw Unsupported()
        val value = expression(returned.result)
        // Expression-bodied returns have a synthetic return source; bind the
        // result occurrence to the real returned expression instead.
        var returnedExpression = returned.result
        while (returnedExpression is FirSmartCastExpression) returnedExpression = returnedExpression.originalExpression
        add(returnedExpression.source, operation("COPY", mapOf("value" to value)), root = true)
    } catch (_: Unsupported) {
        nodes.clear()
        add(source, operation("UNKNOWN", mapOf("boundary" to "KOTLIN_VALUE_BODY_OR_BUDGET_UNSUPPORTED")), root = true)
    }
    return buildJsonObject {
        put("recordType", "PORTABLE_VALUE_PROJECTION")
        put("schema", "k2-value-projection-raw/0.1"); put("compilerVersion", "2.4.10")
        put("authority", "K2_RESOLVED_IMMUTABLE_LOCAL_VALUES")
        put("coordinateDomain", "COMPILER_UTF16_OFFSETS")
        put("file", file); put("owner", identity)
        put("ownerStart", source.startOffset); put("ownerEnd", source.endOffset)
        put("nodes", JsonArray(nodes.values.toList()))
        put("globalBoundaries", JsonArray(listOf(JsonPrimitive("KOTLIN_VALUE_EFFECT_EXCEPTION_MODEL_UNAVAILABLE"))))
    }
}
