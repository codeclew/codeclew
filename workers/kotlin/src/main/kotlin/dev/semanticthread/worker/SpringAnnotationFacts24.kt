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

/** Framework interpretation is separate from descriptor resolution and runtime activation. */
internal fun springAnnotationFacts24(function: FirNamedFunction, context: CheckerContext): JsonObject =
    SpringAnnotationReader24(context).read(function)

internal fun springClassAnnotationFacts24(owner: FirRegularClass, context: CheckerContext): JsonObject =
    SpringAnnotationReader24(context).readInherited(owner)

private const val WEB = "org.springframework.web.bind.annotation."
private const val KAFKA = "org.springframework.kafka.annotation."
private const val SCHEDULING = "org.springframework.scheduling.annotation."
private const val ALIAS = "org.springframework.core.annotation.AliasFor"
private val HTTP_METHODS = mapOf(
    "GetMapping" to "GET", "PostMapping" to "POST", "PutMapping" to "PUT",
    "DeleteMapping" to "DELETE", "PatchMapping" to "PATCH",
)
private val ROOT_ANNOTATIONS = setOf(
    WEB + "RequestMapping", KAFKA + "KafkaListener", KAFKA + "KafkaHandler",
    SCHEDULING + "Scheduled", "org.springframework.stereotype.Controller",
)
private val CONTAINERS = mapOf(
    KAFKA + "KafkaListeners" to KAFKA + "KafkaListener",
    SCHEDULING + "Schedules" to SCHEDULING + "Scheduled",
)

private data class SpringBinding24(
    val annotation: String,
    val chain: List<String>,
    val attributes: Map<String, JsonElement>,
)

private class SpringAnnotationReader24(private val context: CheckerContext) {
    private val session: FirSession = context.session
    private val boundaries = sortedSetOf<String>()
    private var visits = 0

    fun read(function: FirNamedFunction, beanOwner: FirRegularClass? = null): JsonObject {
        val owner = beanOwner ?: function.symbol.dispatchReceiverType?.classId
            ?.let(session::getRegularClassSymbolByClassId)?.fir
        val classes = classHierarchy(owner)
        val classBindings = classes.map { expandAll(it.annotations) }
        val direct = expandAll(function.annotations)
        val inherited = inheritedAnnotations(function, owner, mutableSetOf())
        // Spring uses the nearest method declaration for each annotation family.
        val bindings = direct + inherited.filter { base -> direct.none { it.annotation == base.annotation } }
        val typeMappings = classBindings.firstOrNull { list -> list.any { it.annotation == WEB + "RequestMapping" } }
            .orEmpty().filter { it.annotation == WEB + "RequestMapping" }
        val controller = classBindings.flatten().any { it.annotation == "org.springframework.stereotype.Controller" }
        val entries = mutableListOf<JsonObject>()
        fun entry(binding: SpringBinding24, kind: String, extra: Map<String, JsonElement> = emptyMap()) {
            entries += buildJsonObject {
                put("kind", kind)
                put("annotation", binding.annotation)
                put("annotationChain", JsonArray(binding.chain.map(::JsonPrimitive)))
                put("attributes", JsonObject(binding.attributes.toSortedMap()))
                put("registration", "RUNTIME_CONDITIONAL")
                owner?.let { put("beanClass", "class:${it.symbol.classId}") }
                extra.forEach { (key, value) -> put(key, value) }
            }
        }
        val mappings = bindings.filter { it.annotation == WEB + "RequestMapping" }
        if (mappings.size > 1 || typeMappings.size > 1) boundaries += "MULTIPLE_REQUEST_MAPPINGS_ON_ELEMENT"
        mappings.forEach { binding ->
            if (!controller) boundaries += "CONTROLLER_REGISTRATION_UNPROVEN"
            entry(binding, "HTTP_ENDPOINT", mapOf(
                "controller" to JsonPrimitive(controller),
                "classAttributes" to JsonArray(typeMappings.map { JsonObject(it.attributes.toSortedMap()) }),
            ))
        }
        bindings.filter { it.annotation == KAFKA + "KafkaListener" }
            .forEach { entry(it, "KAFKA_LISTENER") }
        val handlers = bindings.filter { it.annotation == KAFKA + "KafkaHandler" }
        if (handlers.isNotEmpty()) {
            val listeners = classBindings.firstOrNull { list -> list.any { it.annotation == KAFKA + "KafkaListener" } }
                .orEmpty().filter { it.annotation == KAFKA + "KafkaListener" }
            if (listeners.isEmpty()) boundaries += "KAFKA_HANDLER_WITHOUT_CLASS_LISTENER"
            listeners.forEach { entry(it, "KAFKA_LISTENER", mapOf("handlerAttributes" to JsonObject(handlers.first().attributes))) }
        }
        bindings.filter { it.annotation == SCHEDULING + "Scheduled" }
            .forEach { entry(it, "SCHEDULED_JOB") }
        if (owner == null && entries.isNotEmpty()) boundaries += "NO_BEAN_OWNER"
        if (function.status.modality == org.jetbrains.kotlin.descriptors.Modality.ABSTRACT && entries.isNotEmpty()) {
            boundaries += "ABSTRACT_HANDLER_REQUIRES_IMPLEMENTATION"
        }
        return buildJsonObject {
            put("schema", "spring-entrypoints/0.1")
            put("authority", "K2_RESOLVED_ANNOTATIONS")
            put("entries", JsonArray(entries))
            put("boundaries", JsonArray(boundaries.map(::JsonPrimitive)))
        }
    }

    fun readInherited(owner: FirRegularClass): JsonObject {
        val entries = mutableListOf<JsonElement>()
        // Interfaces/abstract bases carry declaration evidence; only concrete
        // classes add distinct inherited bean registrations.
        if (owner.status.modality != org.jetbrains.kotlin.descriptors.Modality.ABSTRACT &&
            owner.classKind != org.jetbrains.kotlin.descriptors.ClassKind.ANNOTATION_CLASS) {
            val scope = owner.symbol.unsubstitutedScope(session, context.scopeSession, false, FirResolvePhase.BODY_RESOLVE)
            val seen = mutableSetOf<String>()
            scope.getCallableNames().sortedBy { it.asString() }.forEach { name ->
                scope.processFunctionsByName(name) { symbol ->
                    if (symbol.callableId.classId == owner.symbol.classId && symbol.fir.origin.fromSource) return@processFunctionsByName
                    var target = symbol
                    val visited = mutableSetOf<org.jetbrains.kotlin.fir.symbols.impl.FirNamedFunctionSymbol>()
                    while (!target.fir.origin.fromSource && target.callableId.classId == owner.symbol.classId && visited.add(target)) {
                        val bases = scope.getDirectOverriddenSafe(target)
                        if (bases.size != 1) break
                        target = bases.single() as? org.jetbrains.kotlin.fir.symbols.impl.FirNamedFunctionSymbol ?: break
                    }
                    val method = target.fir
                    val reader = SpringAnnotationReader24(context)
                    val metadata = reader.read(method, owner)
                    val inheritedEntries = metadata["entries"]!!.jsonArray
                    if (inheritedEntries.isEmpty()) return@processFunctionsByName
                    val descriptor = compilerJvmMethodDescriptor(method)
                    if (descriptor == null || target.callableId.classId == owner.symbol.classId) {
                        boundaries += "INHERITED_HANDLER_IDENTITY_UNRESOLVED"
                        return@processFunctionsByName
                    }
                    val identity = "callable:${target.callableId}#jvm:$descriptor"
                    if (!seen.add(identity)) return@processFunctionsByName
                    boundaries += metadata["boundaries"]!!.jsonArray.map { it.jsonPrimitive.content }
                    inheritedEntries.forEach { entry ->
                        entries += JsonObject(entry.jsonObject + ("targetSymbol" to JsonPrimitive(identity)))
                    }
                }
            }
        }
        return buildJsonObject {
            put("schema", "spring-entrypoints/0.1")
            put("authority", "K2_RESOLVED_ANNOTATIONS")
            put("entries", JsonArray(entries))
            put("boundaries", JsonArray(boundaries.map(::JsonPrimitive)))
        }
    }

    private fun classHierarchy(owner: FirRegularClass?): List<FirRegularClass> {
        val result = mutableListOf<FirRegularClass>()
        val queue = ArrayDeque<FirRegularClass>()
        val seen = mutableSetOf<String>()
        owner?.let(queue::add)
        while (queue.isNotEmpty()) {
            val next = queue.removeFirst()
            if (!seen.add(next.symbol.classId.toString())) continue
            if (seen.size > 128) { boundaries += "TYPE_HIERARCHY_LIMIT"; break }
            result += next
            next.superTypeRefs.forEach { ref ->
                (ref as? FirResolvedTypeRef)?.coneType?.classId
                    ?.let(session::getRegularClassSymbolByClassId)?.fir?.let(queue::add)
            }
        }
        return result
    }

    private fun inheritedAnnotations(
        function: FirNamedFunction, owner: FirRegularClass?, seen: MutableSet<String>,
    ): List<SpringBinding24> {
        if (!function.status.isOverride || owner == null) return emptyList()
        val key = function.symbol.callableId.toString() + function.valueParameters.map { it.returnTypeRef.toString() }
        if (!seen.add(key)) return emptyList()
        if (seen.size > 128) { boundaries += "METHOD_HIERARCHY_LIMIT"; return emptyList() }
        val scope = owner.symbol.unsubstitutedScope(session, context.scopeSession, false, FirResolvePhase.BODY_RESOLVE)
        scope.processFunctionsByName(function.name) { }
        val bases = scope.getDirectOverriddenSafe(function.symbol)
        return bases.flatMap { symbol ->
            val base = symbol.fir as? FirNamedFunction ?: return@flatMap emptyList()
            val baseOwner = symbol.dispatchReceiverType?.classId?.let(session::getRegularClassSymbolByClassId)?.fir
            val direct = expandAll(base.annotations)
            direct + inheritedAnnotations(base, baseOwner, seen).filter { inherited ->
                direct.none { it.annotation == inherited.annotation }
            }
        }
    }

    private fun expandAll(annotations: List<FirAnnotation>): List<SpringBinding24> =
        annotations.flatMap { expand(it, emptyList()) }

    private fun expand(annotation: FirAnnotation, path: List<String>): List<SpringBinding24> {
        val id = annotation.toAnnotationClassIdSafe(session)?.asSingleFqName()?.asString()
        if (id == null) { boundaries += "UNRESOLVED_ANNOTATION_CLASS"; return emptyList() }
        if (id in path) return emptyList()
        if (++visits > 2048 || path.size >= 32) { boundaries += "ANNOTATION_GRAPH_LIMIT"; return emptyList() }
        if (id.startsWith("kotlin.") || id.startsWith("java.lang.annotation.")) return emptyList()
        val chain = path + id
        val attributes = arguments(annotation)
        CONTAINERS[id]?.let { expected ->
            val nested = annotation.argumentMapping.mapping.values.flatMap(::elements)
                .mapNotNull(::asAnnotation)
            if (nested.isEmpty()) boundaries += "UNRESOLVED_REPEATABLE_CONTAINER"
            return nested.flatMap { child ->
                if (child.toAnnotationClassIdSafe(session)?.asSingleFqName()?.asString() != expected) {
                    boundaries += "INVALID_REPEATABLE_CONTAINER"; emptyList()
                } else expand(child, chain)
            }
        }
        HTTP_METHODS[id.removePrefix(WEB)]?.takeIf { id.startsWith(WEB) }?.let { method ->
            return listOf(SpringBinding24(WEB + "RequestMapping", chain, aliases(attributes) + ("method" to JsonArray(listOf(JsonPrimitive(method))))))
        }
        if (id in ROOT_ANNOTATIONS) return listOf(SpringBinding24(id, chain, if (id == WEB + "RequestMapping") aliases(attributes) else attributes))
        val declaration = annotation.toAnnotationClass(session) ?: run {
            boundaries += "ANNOTATION_DECLARATION_UNAVAILABLE"; return emptyList()
        }
        val metas = declaration.annotations.flatMap { expand(it, chain) }
        if (metas.isEmpty()) return emptyList()
        val defaults = declaration.declarations.filterIsInstance<FirConstructor>()
            .flatMap { it.valueParameters }.mapNotNull { parameter ->
                parameter.defaultValue?.let { parameter.name.asString() to value(it, 0) }
            }.toMap()
        val effective = defaults + attributes
        // AliasFor can be declared on Kotlin annotation constructor properties or Java methods.
        val members = declaration.declarations.filterIsInstance<FirCallableDeclaration>().mapNotNull { member ->
            val name = member.symbol.callableId?.callableName?.asString() ?: return@mapNotNull null
            val annotations = member.annotations + if (member is FirProperty) member.getter?.annotations.orEmpty() else emptyList()
            name to annotations.filter { it.toAnnotationClassIdSafe(session)?.asSingleFqName()?.asString() == ALIAS }
        }
        return metas.map { meta ->
            val merged = meta.attributes.toMutableMap()
            members.forEach { (name, aliases) -> aliases.forEach { alias ->
                val target = alias.getKClassArgument(org.jetbrains.kotlin.name.Name.identifier("annotation"))
                    ?.classId?.asSingleFqName()?.asString()
                val targetName = alias.getStringArgument(org.jetbrains.kotlin.name.Name.identifier("attribute"))
                    ?.takeIf(String::isNotEmpty)
                    ?: alias.getStringArgument(org.jetbrains.kotlin.name.Name.identifier("value"))?.takeIf(String::isNotEmpty)
                    ?: name
                if (target == null || target == "kotlin.Annotation" || target == "java.lang.annotation.Annotation" || target == id) {
                    val peer = effective[targetName]
                    val selected = attributes[name] ?: attributes[targetName] ?: effective[name] ?: peer
                    if (selected != null && targetName in merged) merged[targetName] = selected
                    if (attributes[name] != null && attributes[targetName] != null && attributes[name] != attributes[targetName]) boundaries += "CONFLICTING_ANNOTATION_ALIASES"
                } else if (target == meta.annotation || target in meta.chain) {
                    val finalName = aliasDestination(target, targetName, meta.annotation, mutableSetOf())
                    if (finalName == null) boundaries += "UNRESOLVED_COMPOSED_ALIAS"
                    else effective[name]?.let { merged[if (meta.annotation == WEB + "RequestMapping" && finalName == "value") "path" else finalName] = it }
                }
            } }
            // Legacy Spring convention: same-name attributes override meta-attributes.
            effective.forEach { (name, value) -> if (name != "value" && name in merged) merged[name] = value }
            meta.copy(attributes = if (meta.annotation == WEB + "RequestMapping") aliases(merged) else merged)
        }
    }

    private fun aliasDestination(annotation: String, attribute: String, root: String, seen: MutableSet<String>): String? {
        if (annotation == root) return attribute
        if (!seen.add("$annotation#$attribute") || seen.size > 32) return null
        val declaration = session.getRegularClassSymbolByClassId(org.jetbrains.kotlin.name.ClassId.topLevel(org.jetbrains.kotlin.name.FqName(annotation)))?.fir ?: return null
        val member = declaration.declarations.filterIsInstance<FirCallableDeclaration>()
            .firstOrNull { it.symbol.callableId?.callableName?.asString() == attribute } ?: return null
        val annotations = member.annotations + if (member is FirProperty) member.getter?.annotations.orEmpty() else emptyList()
        val alias = annotations.firstOrNull { it.toAnnotationClassIdSafe(session)?.asSingleFqName()?.asString() == ALIAS } ?: return null
        val target = alias.getKClassArgument(org.jetbrains.kotlin.name.Name.identifier("annotation"))?.classId?.asSingleFqName()?.asString()
            ?.takeUnless { it == "kotlin.Annotation" || it == "java.lang.annotation.Annotation" } ?: annotation
        val name = alias.getStringArgument(org.jetbrains.kotlin.name.Name.identifier("attribute"))?.takeIf(String::isNotEmpty)
            ?: alias.getStringArgument(org.jetbrains.kotlin.name.Name.identifier("value"))?.takeIf(String::isNotEmpty) ?: attribute
        return aliasDestination(target, name, root, seen)
    }

    private fun aliases(input: Map<String, JsonElement>): Map<String, JsonElement> {
        val result = input.toMutableMap()
        val value = input["value"]?.takeUnless { it is JsonArray && it.isEmpty() }
        val path = input["path"]?.takeUnless { it is JsonArray && it.isEmpty() }
        if (value != null && path != null && value != path) boundaries += "CONFLICTING_PATH_ALIASES"
        (path ?: value)?.let { result["path"] = it }
        result.remove("value")
        return result
    }

    private fun arguments(annotation: FirAnnotation): Map<String, JsonElement> =
        annotation.argumentMapping.mapping.entries.associate { (name, expression) -> name.asString() to value(expression, 0) }

    private fun elements(expression: FirExpression): List<FirExpression> = when (expression) {
        is FirVarargArgumentsExpression -> expression.arguments.flatMap(::elements)
        is FirCollectionLiteral -> expression.argumentList.arguments.flatMap(::elements)
        is FirWrappedArgumentExpression -> elements(expression.expression)
        else -> listOf(expression)
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

    private fun value(expression: FirExpression, depth: Int): JsonElement {
        if (depth > 32) { boundaries += "ANNOTATION_VALUE_LIMIT"; return JsonNull }
        if (expression !is FirAnnotation) asAnnotation(expression)?.let { return value(it, depth + 1) }
        when (expression) {
            is FirWrappedArgumentExpression -> return value(expression.expression, depth + 1)
            is FirVarargArgumentsExpression -> return JsonArray(expression.arguments.map { value(it, depth + 1) })
            is FirCollectionLiteral -> return JsonArray(expression.argumentList.arguments.map { value(it, depth + 1) })
            is FirAnnotation -> return buildJsonObject {
                put("annotation", expression.toAnnotationClassIdSafe(session)?.asSingleFqName()?.asString().orEmpty())
                put("attributes", JsonObject(arguments(expression)))
            }
            is FirLiteralExpression -> return when (val literal = expression.value) {
                is String -> {
                    if (literal.contains("\${") || literal.contains("#{")) boundaries += "RUNTIME_EXPRESSION"
                    JsonPrimitive(literal)
                }
                is Number -> JsonPrimitive(literal)
                is Boolean -> JsonPrimitive(literal)
                null -> JsonNull
                else -> JsonPrimitive(literal.toString())
            }
        }
        expression.extractEnumValueArgumentInfo()?.let { return JsonPrimitive(it.enumEntryName.asString()) }
        if (expression is FirQualifiedAccessExpression) {
            val property = (expression.calleeReference as? FirResolvedNamedReference)?.resolvedSymbol as? FirPropertySymbol
            if (property?.fir?.status?.isConst == true) property.fir.initializer?.let { return value(it, depth + 1) }
        }
        val evaluated = runCatching { expression.evaluateAs<FirLiteralExpression>(session) }.getOrNull()
        if (evaluated != null && evaluated !== expression) return value(evaluated, depth + 1)
        boundaries += "UNRESOLVED_ANNOTATION_VALUE"
        return JsonNull
    }
}
