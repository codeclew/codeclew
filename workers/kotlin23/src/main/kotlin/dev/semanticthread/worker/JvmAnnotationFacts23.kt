@file:OptIn(
    org.jetbrains.kotlin.fir.symbols.SymbolInternals::class,
    org.jetbrains.kotlin.fir.declarations.DirectDeclarationsAccess::class,
)

package dev.semanticthread.worker

import kotlinx.serialization.json.*
import org.jetbrains.kotlin.fir.FirSession
import org.jetbrains.kotlin.fir.analysis.checkers.context.CheckerContext
import org.jetbrains.kotlin.fir.declarations.*
import org.jetbrains.kotlin.fir.expressions.*
import org.jetbrains.kotlin.fir.references.FirResolvedNamedReference
import org.jetbrains.kotlin.fir.resolve.providers.getRegularClassSymbolByClassId
import org.jetbrains.kotlin.fir.scopes.getDirectOverriddenSafe
import org.jetbrains.kotlin.fir.scopes.unsubstitutedScope
import org.jetbrains.kotlin.fir.symbols.impl.FirPropertySymbol
import org.jetbrains.kotlin.fir.types.*

internal fun jvmAnnotationFacts23(function: FirSimpleFunction, context: CheckerContext): JsonObject =
    JvmAnnotationReader23(context).read(function)

internal fun jvmClassAnnotationFacts23(owner: FirRegularClass, context: CheckerContext): JsonObject =
    JvmAnnotationReader23(context).readInherited(owner)

/** Compiler observations only; no framework names, aliases or registration rules. */
private class JvmAnnotationReader23(private val context: CheckerContext) {
    private val session: FirSession = context.session
    private val definitions = sortedMapOf<String, JsonElement>()
    private val boundaries = sortedSetOf<String>()
    private val collecting = mutableSetOf<String>()
    private var annotationDepth = 0
    private var visits = 0

    private fun identity(function: FirSimpleFunction): String? = compilerJvmMethodDescriptor23(function)
        ?.let { "callable:${function.symbol.callableId}#jvm:$it" }

    fun read(function: FirSimpleFunction): JsonObject {
        val id = identity(function) ?: "callable:${function.symbol.callableId}"
        val owner = function.symbol.dispatchReceiverType?.classId?.let(session::getRegularClassSymbolByClassId)?.fir
        val callable = callable(function, owner, false)
        return finish(id, listOf(callable), owner)
    }

    fun readInherited(owner: FirRegularClass): JsonObject {
        val callables = mutableListOf<JsonElement>()
        if (owner.status.modality != org.jetbrains.kotlin.descriptors.Modality.ABSTRACT &&
            owner.classKind != org.jetbrains.kotlin.descriptors.ClassKind.ANNOTATION_CLASS) {
            val scope = owner.symbol.unsubstitutedScope(session, context.scopeSession, false, FirResolvePhase.BODY_RESOLVE)
            val seen = mutableSetOf<String>()
            var members = 0
            scope.getCallableNames().sortedBy { it.asString() }.forEach { name ->
                scope.processFunctionsByName(name) { symbol ->
                    if (++members > 4096) { boundaries += "INHERITED_MEMBER_LIMIT"; return@processFunctionsByName }
                    if (symbol.callableId.classId == owner.symbol.classId && symbol.fir.origin.fromSource) return@processFunctionsByName
                    var target = symbol
                    val visited = mutableSetOf<org.jetbrains.kotlin.fir.symbols.impl.FirNamedFunctionSymbol>()
                    while (!target.fir.origin.fromSource && target.callableId.classId == owner.symbol.classId && visited.add(target)) {
                        val bases = scope.getDirectOverriddenSafe(target)
                        if (bases.size != 1) break
                        target = bases.single() as? org.jetbrains.kotlin.fir.symbols.impl.FirNamedFunctionSymbol ?: break
                    }
                    // Kotlin/JVM's implicit Object methods add no source callable.
                    if (target.callableId.classId?.asSingleFqName()?.asString() in setOf("kotlin.Any", "java.lang.Object")) return@processFunctionsByName
                    val id = identity(target.fir)
                    if (id == null || target.callableId.classId == owner.symbol.classId) {
                        boundaries += "INHERITED_CALLABLE_IDENTITY_UNRESOLVED"; return@processFunctionsByName
                    }
                    if (seen.add(id)) callables += callable(target.fir, owner, true)
                }
            }
        }
        return finish("class:${owner.symbol.classId}", callables, owner)
    }

    private fun finish(declaration: String, callables: List<JsonElement>, owner: FirRegularClass?): JsonObject {
        val types = JsonArray(hierarchy(owner).map { type -> buildJsonObject {
            put("identity", "class:${type.symbol.classId}")
            put("annotations", annotations(type.annotations))
            put("directSupertypes", JsonArray(type.superTypeRefs.map { JsonPrimitive(it.coneTypeOrNull?.toString() ?: it.toString()) }))
        } })
        return buildJsonObject {
        put("schema", "jvm-annotation-facts/1.0")
        put("authority", "K2_RESOLVED_ANNOTATIONS")
        put("declaration", declaration)
        put("definitions", JsonObject(definitions.toMap()))
        put("types", types)
        put("callables", JsonArray(callables))
        put("boundaries", JsonArray(boundaries.map(::JsonPrimitive)))
        putJsonObject("coverage") {
            put("status", if (boundaries.isEmpty()) "COMPLETE" else "PARTIAL")
            put("scope", "REACHABLE_ANNOTATIONS_AND_HIERARCHY")
        }
    }

    }

    private fun callable(function: FirSimpleFunction, owner: FirRegularClass?, inherited: Boolean): JsonObject = buildJsonObject {
        put("method", method(function, owner, mutableSetOf()))
        put("classes", JsonArray(hierarchy(owner).map { type -> buildJsonObject {
            put("identity", "class:${type.symbol.classId}")
            put("annotations", annotations(type.annotations))
            put("directSupertypes", JsonArray(type.superTypeRefs.map { JsonPrimitive(it.coneTypeOrNull?.toString() ?: it.toString()) }))
        } }))
        owner?.let { put("beanClass", "class:${it.symbol.classId}") }
        put("abstractMethod", function.status.modality == org.jetbrains.kotlin.descriptors.Modality.ABSTRACT)
        put("inherited", inherited)
        put("implementationSource", function.origin.fromSource)
    }

    private fun hierarchy(owner: FirRegularClass?): List<FirRegularClass> {
        val result = mutableListOf<FirRegularClass>()
        val queue = ArrayDeque<FirRegularClass>()
        val seen = mutableSetOf<String>()
        owner?.let(queue::add)
        while (queue.isNotEmpty()) {
            val next = queue.removeFirst()
            if (!seen.add(next.symbol.classId.toString())) continue
            if (seen.size > 128) { boundaries += "TYPE_HIERARCHY_LIMIT"; break }
            result += next
            next.superTypeRefs.forEach { ref -> ref.coneTypeOrNull?.classId?.let(session::getRegularClassSymbolByClassId)?.fir?.let(queue::add) }
        }
        return result
    }

    private fun method(function: FirSimpleFunction, owner: FirRegularClass?, seen: MutableSet<String>): JsonObject {
        val id = identity(function) ?: "callable:${function.symbol.callableId}"
        val bases = mutableListOf<JsonElement>()
        if (seen.size >= 128) boundaries += "METHOD_HIERARCHY_LIMIT"
        else if (function.status.isOverride && owner != null && seen.add(id)) {
            val scope = owner.symbol.unsubstitutedScope(session, context.scopeSession, false, FirResolvePhase.BODY_RESOLVE)
            scope.processFunctionsByName(function.name) { }
            scope.getDirectOverriddenSafe(function.symbol).forEach { symbol ->
                val base = symbol.fir as? FirSimpleFunction ?: return@forEach
                val baseOwner = symbol.dispatchReceiverType?.classId?.let(session::getRegularClassSymbolByClassId)?.fir
                bases += method(base, baseOwner, seen)
            }
        }
        return buildJsonObject {
            put("identity", id)
            put("annotations", annotations(function.annotations))
            put("overrides", JsonArray(bases))
        }
    }

    private fun origin(identity: String, start: Int? = null, end: Int? = null): JsonObject = buildJsonObject {
        val source = start != null && end != null && start >= 0 && end >= start
        put("kind", if (source) "SOURCE" else "BINARY")
        put("identity", identity)
        if (source) { put("start", start); put("end", end) }
    }

    private fun annotations(values: List<FirAnnotation>): JsonArray = JsonArray(values.mapNotNull(::annotation))

    private fun annotation(annotation: FirAnnotation): JsonObject? {
        val id = annotation.toAnnotationClassIdSafe(session)?.asSingleFqName()?.asString()
        if (id == null) { boundaries += "UNRESOLVED_ANNOTATION_CLASS"; return null }
        if (++visits > 32768 || annotationDepth >= 32) { boundaries += "ANNOTATION_GRAPH_LIMIT"; return null }
        annotationDepth++
        try {
            annotation.toAnnotationClass(session)?.let(::definition) ?: run { boundaries += "ANNOTATION_DECLARATION_UNAVAILABLE" }
            return buildJsonObject {
                put("typeName", id)
                put("arguments", arguments(annotation))
                put("origin", origin(id, annotation.source?.startOffset, annotation.source?.endOffset))
                annotation.useSiteTarget?.let { put("useSiteTarget", it.name) }
            }
        } finally { annotationDepth-- }
    }

    private fun definition(declaration: FirRegularClass) {
        val id = declaration.symbol.classId.asSingleFqName().asString()
        if (id in definitions || !collecting.add(id)) return
        if (definitions.size + collecting.size > 2048) { boundaries += "ANNOTATION_DEFINITION_LIMIT"; collecting.remove(id); return }
        try {
            val defaults = declaration.declarations.filterIsInstance<FirConstructor>().flatMap { it.valueParameters }
                .mapNotNull { parameter -> parameter.defaultValue?.let { parameter.name.asString() to it } }.toMap()
            val members = sortedMapOf<String, JsonElement>()
            declaration.declarations.filterIsInstance<FirCallableDeclaration>().forEach { member ->
                if (member is FirConstructor) return@forEach
                val name = member.symbol.callableId?.callableName?.asString() ?: return@forEach
                val memberAnnotations = member.annotations + if (member is FirProperty) member.getter?.annotations.orEmpty() else emptyList()
                members[name] = buildJsonObject {
                    put("annotations", annotations(memberAnnotations))
                    defaults[name]?.let { put("defaultValue", value(it, 0)) }
                    put("returnType", member.returnTypeRef.coneTypeOrNull?.toString() ?: member.returnTypeRef.toString())
                }
            }
            defaults.forEach { (name, expression) -> if (name !in members) members[name] = buildJsonObject {
                put("annotations", JsonArray(emptyList()))
                put("defaultValue", value(expression, 0))
                put("returnType", expression.resolvedType.toString())
            } }
            definitions[id] = buildJsonObject {
                put("origin", origin(id, declaration.source?.startOffset, declaration.source?.endOffset))
                put("annotations", annotations(declaration.annotations))
                put("members", JsonObject(members))
            }
        } finally { collecting.remove(id) }
    }

    private fun arguments(annotation: FirAnnotation): JsonObject = buildJsonObject {
        annotation.argumentMapping.mapping.forEach { (name, expression) ->
            val classId = annotation.getKClassArgument(name, session)?.classId
            if (classId != null) {
                session.getRegularClassSymbolByClassId(classId)?.fir?.takeIf { it.classKind == org.jetbrains.kotlin.descriptors.ClassKind.ANNOTATION_CLASS }?.let(::definition)
                put(name.asString(), buildJsonObject { put("kind", "CLASS"); put("value", classId.asSingleFqName().asString()) })
            } else put(name.asString(), value(expression, 0))
        }
    }

    private fun asAnnotation(expression: FirExpression): FirAnnotation? {
        if (expression is FirAnnotation) return expression
        val call = expression as? FirFunctionCall ?: return null
        val constructor = (call.calleeReference as? FirResolvedNamedReference)?.resolvedSymbol as? org.jetbrains.kotlin.fir.symbols.impl.FirConstructorSymbol ?: return null
        val owner = constructor.callableId.classId?.let(session::getRegularClassSymbolByClassId) ?: return null
        if (owner.fir.classKind != org.jetbrains.kotlin.descriptors.ClassKind.ANNOTATION_CLASS) return null
        val arguments = call.argumentList as? org.jetbrains.kotlin.fir.expressions.impl.FirResolvedArgumentList ?: return null
        return org.jetbrains.kotlin.fir.expressions.builder.buildAnnotation {
            annotationTypeRef = org.jetbrains.kotlin.fir.types.builder.buildResolvedTypeRef { coneType = call.resolvedType }
            argumentMapping = org.jetbrains.kotlin.fir.expressions.builder.buildAnnotationArgumentMapping {
                arguments.mapping.forEach { (expression, parameter) -> mapping[parameter.name] = expression }
            }
        }
    }

    private fun unresolved(reason: String): JsonObject {
        boundaries += reason
        return buildJsonObject { put("kind", "UNRESOLVED"); put("reason", reason) }
    }
    private fun array(values: List<FirExpression>, depth: Int): JsonObject = buildJsonObject {
        put("kind", "ARRAY"); put("values", JsonArray(values.map { value(it, depth + 1) }))
    }
    private fun value(expression: FirExpression, depth: Int): JsonObject {
        if (depth > 32) return unresolved("ANNOTATION_VALUE_LIMIT")
        if (expression !is FirAnnotation) asAnnotation(expression)?.let { return value(it, depth + 1) }
        when (expression) {
            is FirGetClassCall -> {
                val id = (expression.argument as? FirClassReferenceExpression)?.classTypeRef?.coneTypeOrNull?.classId
                    ?: expression.argument.resolvedType.classId
                if (id != null) {
                    session.getRegularClassSymbolByClassId(id)?.fir?.takeIf { it.classKind == org.jetbrains.kotlin.descriptors.ClassKind.ANNOTATION_CLASS }?.let(::definition)
                    return buildJsonObject { put("kind", "CLASS"); put("value", id.asSingleFqName().asString()) }
                }
            }
            is FirWrappedArgumentExpression -> return value(expression.expression, depth + 1)
            is FirVarargArgumentsExpression -> return array(expression.arguments, depth)
            is FirArrayLiteral -> return array(expression.argumentList.arguments, depth)
            is FirAnnotation -> return annotation(expression)?.let { buildJsonObject { put("kind", "ANNOTATION"); put("value", it) } }
                ?: unresolved("UNRESOLVED_ANNOTATION_CLASS")
            is FirLiteralExpression -> return buildJsonObject {
                put("kind", "CONSTANT")
                put("value", when (val literal = expression.value) {
                    is String -> JsonPrimitive(literal)
                    is Number -> JsonPrimitive(literal)
                    is Boolean -> JsonPrimitive(literal)
                    null -> JsonNull
                    else -> JsonPrimitive(literal.toString())
                })
            }
        }
        expression.extractEnumValueArgumentInfo()?.let { info -> return buildJsonObject {
            put("kind", "ENUM"); put("type", expression.resolvedType.classId?.asSingleFqName()?.asString() ?: expression.resolvedType.toString())
            put("value", info.enumEntryName.asString())
        } }
        if (expression is FirQualifiedAccessExpression) {
            val property = (expression.calleeReference as? FirResolvedNamedReference)?.resolvedSymbol as? FirPropertySymbol
            if (property?.fir?.status?.isConst == true) property.fir.initializer?.let { return value(it, depth + 1) }
        }
        val evaluated = runCatching { expression.evaluateAs<FirLiteralExpression>(session) }.getOrNull()
        if (evaluated != null && evaluated !== expression) return value(evaluated, depth + 1)
        return unresolved("UNRESOLVED_ANNOTATION_VALUE")
    }
}
