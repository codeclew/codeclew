@file:OptIn(
    org.jetbrains.kotlin.compiler.plugin.ExperimentalCompilerApi::class,
    org.jetbrains.kotlin.fir.symbols.SymbolInternals::class,
)

package dev.semanticthread.worker

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import org.jetbrains.kotlin.KtRealSourceElementKind
import org.jetbrains.kotlin.compiler.plugin.AbstractCliOption
import org.jetbrains.kotlin.compiler.plugin.CliOption
import org.jetbrains.kotlin.compiler.plugin.CliOptionProcessingException
import org.jetbrains.kotlin.compiler.plugin.CommandLineProcessor
import org.jetbrains.kotlin.compiler.plugin.CompilerPluginRegistrar
import org.jetbrains.kotlin.config.CompilerConfiguration
import org.jetbrains.kotlin.config.CompilerConfigurationKey
import org.jetbrains.kotlin.diagnostics.DiagnosticReporter
import org.jetbrains.kotlin.fir.FirSession
import org.jetbrains.kotlin.fir.analysis.checkers.MppCheckerKind
import org.jetbrains.kotlin.fir.analysis.checkers.context.CheckerContext
import org.jetbrains.kotlin.fir.analysis.checkers.declaration.DeclarationCheckers
import org.jetbrains.kotlin.fir.analysis.checkers.declaration.FirDeclarationChecker
import org.jetbrains.kotlin.fir.analysis.checkers.expression.ExpressionCheckers
import org.jetbrains.kotlin.fir.analysis.checkers.expression.FirExpressionChecker
import org.jetbrains.kotlin.fir.analysis.extensions.FirAdditionalCheckersExtension
import org.jetbrains.kotlin.fir.declarations.FirAnonymousFunction
import org.jetbrains.kotlin.fir.declarations.FirCallableDeclaration
import org.jetbrains.kotlin.fir.declarations.FirConstructor
import org.jetbrains.kotlin.fir.declarations.FirFunction
import org.jetbrains.kotlin.fir.declarations.FirNamedFunction
import org.jetbrains.kotlin.fir.declarations.FirProperty
import org.jetbrains.kotlin.fir.declarations.FirRegularClass
import org.jetbrains.kotlin.fir.declarations.FirResolvePhase
import org.jetbrains.kotlin.fir.declarations.FirTypeParameter
import org.jetbrains.kotlin.fir.declarations.hasAnnotationSafe
import org.jetbrains.kotlin.fir.expressions.FirExpression
import org.jetbrains.kotlin.fir.expressions.FirElvisExpression
import org.jetbrains.kotlin.fir.expressions.FirFunctionCall
import org.jetbrains.kotlin.fir.expressions.FirNamedArgumentExpression
import org.jetbrains.kotlin.fir.expressions.FirQualifiedAccessExpression
import org.jetbrains.kotlin.fir.expressions.FirReturnExpression
import org.jetbrains.kotlin.fir.expressions.FirSafeCallExpression
import org.jetbrains.kotlin.fir.expressions.FirStatement
import org.jetbrains.kotlin.fir.expressions.FirTryExpression
import org.jetbrains.kotlin.fir.expressions.FirVariableAssignment
import org.jetbrains.kotlin.fir.expressions.FirWhenExpression
import org.jetbrains.kotlin.fir.expressions.impl.FirResolvedArgumentList
import org.jetbrains.kotlin.fir.expressions.impl.FirSingleExpressionBlock
import org.jetbrains.kotlin.fir.extensions.FirExtensionRegistrar
import org.jetbrains.kotlin.fir.extensions.FirExtensionRegistrarAdapter
import org.jetbrains.kotlin.fir.references.FirResolvedNamedReference
import org.jetbrains.kotlin.fir.resolve.dfa.FirControlFlowGraphReferenceImpl
import org.jetbrains.kotlin.fir.resolve.providers.getRegularClassSymbolByClassId
import org.jetbrains.kotlin.fir.scopes.getDirectOverriddenSafe
import org.jetbrains.kotlin.fir.scopes.jvm.computeJvmDescriptor
import org.jetbrains.kotlin.fir.scopes.unsubstitutedScope
import org.jetbrains.kotlin.fir.symbols.impl.FirCallableSymbol
import org.jetbrains.kotlin.fir.symbols.impl.FirClassSymbol
import org.jetbrains.kotlin.fir.symbols.impl.FirConstructorSymbol
import org.jetbrains.kotlin.fir.symbols.impl.FirNamedFunctionSymbol
import org.jetbrains.kotlin.fir.symbols.impl.FirPropertySymbol
import org.jetbrains.kotlin.fir.types.FirResolvedTypeRef
import org.jetbrains.kotlin.fir.types.ConeKotlinType
import org.jetbrains.kotlin.fir.types.classId
import org.jetbrains.kotlin.fir.types.isMarkedNullable
import org.jetbrains.kotlin.fir.types.resolvedType
import org.jetbrains.kotlin.fir.visitors.FirVisitorVoid
import org.jetbrains.kotlin.name.JvmStandardClassIds
import java.security.MessageDigest
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardOpenOption

internal const val FACTS_PLUGIN_ID = "semantic-thread-facts"
private val OUTPUT_KEY = CompilerConfigurationKey<String>("semantic thread FIR facts output")

class FirFactsCommandLineProcessor : CommandLineProcessor {
    override val pluginId: String = FACTS_PLUGIN_ID
    override val pluginOptions = listOf(
        CliOption("output", "path", "FIR facts JSONL output", required = true, allowMultipleOccurrences = false),
    )

    override fun processOption(
        option: AbstractCliOption,
        value: String,
        configuration: CompilerConfiguration,
    ) {
        if (option.optionName == "output") {
            configuration.put(OUTPUT_KEY, value)
        } else {
            throw CliOptionProcessingException("unknown option ${option.optionName}")
        }
    }
}

class FirFactsCompilerPluginRegistrar : CompilerPluginRegistrar() {
    override val pluginId: String = FACTS_PLUGIN_ID
    override val supportsK2: Boolean = true

    override fun ExtensionStorage.registerExtensions(configuration: CompilerConfiguration) {
        val output = configuration.get(OUTPUT_KEY) ?: return
        FirExtensionRegistrarAdapter.registerExtension(FirFactsExtensionRegistrar(output))
    }
}

private class FirFactsExtensionRegistrar(private val output: String) : FirExtensionRegistrar() {
    override fun ExtensionRegistrarContext.configurePlugin() {
        +{ session: FirSession -> FirFactsCheckersExtension(session, output) }
    }
}

private class FirFactsCheckersExtension(
    session: FirSession,
    output: String,
) : FirAdditionalCheckersExtension(session) {
    override val expressionCheckers: ExpressionCheckers = object : ExpressionCheckers() {
        override val basicExpressionCheckers: Set<FirExpressionChecker<FirStatement>> =
            setOf(FirFactsExpressionChecker(output))
    }
    override val declarationCheckers: DeclarationCheckers = object : DeclarationCheckers() {
        override val fileCheckers: Set<FirDeclarationChecker<org.jetbrains.kotlin.fir.declarations.FirFile>> =
            setOf(object : FirDeclarationChecker<org.jetbrains.kotlin.fir.declarations.FirFile>(MppCheckerKind.Common) {
                context(context: CheckerContext, reporter: DiagnosticReporter)
                override fun check(declaration: org.jetbrains.kotlin.fir.declarations.FirFile) {
                    val file = context.containingFilePath ?: declaration.sourceFile?.path ?: return
                    appendFact(
                        output,
                        buildJsonObject {
                            put("recordType", "FIR_FILE_RECEIPT")
                            put("schema", "fir-file-receipt/0.1")
                            put("file", file)
                        },
                    )
                }
            })
        override val functionCheckers: Set<FirDeclarationChecker<FirFunction>> =
            setOf(FirFactsCfgChecker(output))
        override val simpleFunctionCheckers = setOf(
            FirFactsOverrideChecker(output),
            FirFactsFunctionDescriptorChecker(output),
        )
        override val propertyCheckers = setOf(
            FirFactsPropertyChecker(output),
            FirFactsPropertyDescriptorChecker(output),
        )
        override val regularClassCheckers = setOf(FirFactsClassDescriptorChecker(output))
        override val constructorCheckers = setOf(FirFactsConstructorDescriptorChecker(output))
    }
}

private class FirFactsConstructorDescriptorChecker(
    private val output: String,
) : FirDeclarationChecker<FirConstructor>(MppCheckerKind.Common) {
    context(context: CheckerContext, reporter: DiagnosticReporter)
    override fun check(declaration: FirConstructor) {
        val source = declaration.source
        val callableId = declaration.symbol.callableId
        if (callableId == null) {
            appendFact(
                output,
                unknownDescriptor(
                    context,
                    source,
                    null,
                    "CONSTRUCTOR_DECLARATION",
                    "NO_COMPILER_CALLABLE_ID",
                ),
            )
            return
        }
        val provisionalIdentity = "constructor:$callableId"
        if (source == null || !declaration.origin.fromSource || declaration.origin.generated) {
            appendFact(
                output,
                unknownDescriptor(
                    context,
                    source,
                    provisionalIdentity,
                    "CONSTRUCTOR_DECLARATION",
                    "GENERATED_OR_NO_SOURCE",
                ),
            )
            return
        }
        if (callableId.isLocal) {
            appendFact(
                output,
                unknownDescriptor(
                    context,
                    source,
                    provisionalIdentity,
                    "CONSTRUCTOR_DECLARATION",
                    "LOCAL_CONSTRUCTOR_UNSUPPORTED",
                ),
            )
            return
        }
        // Enum-entry bodies have a compiler-owned anonymous subclass which is
        // absent from containingElements' FirRegularClass list. The constructor
        // callable identity, not the enclosing enum, binds its actual owner.
        val ownerClassId = callableId.classId
        val parameterTypes = declaration.valueParameters.map {
            resolvedDescriptorType(it.returnTypeRef)
        }
        val status = runCatching { declaration.symbol.resolvedStatus }.getOrNull()
        val jvmDescriptor = compilerJvmMethodDescriptor(declaration)
        if (ownerClassId == null || parameterTypes.any { it == null }
            || status == null || jvmDescriptor.isNullOrEmpty()
        ) {
            appendFact(
                output,
                unknownDescriptor(
                    context,
                    source,
                    provisionalIdentity,
                    "CONSTRUCTOR_DECLARATION",
                    "UNRESOLVED_CONSTRUCTOR_DESCRIPTOR",
                ),
            )
            return
        }
        val symbolIdentity = "$provisionalIdentity#jvm:$jvmDescriptor"
        val record = descriptorBase(
            context,
            source,
            declaration,
            symbolIdentity,
            "CONSTRUCTOR",
            callableId.packageName.asString(),
            status.visibility.name,
            status.effectiveVisibility.name,
            status.effectiveVisibility.publicApi,
            status.effectiveVisibility.privateApi,
            "FINAL",
        ).toMutableMap()
        record["compilerCallableId"] = JsonPrimitive(callableId.toString())
        val owner = constructorOwnerAuthority(ownerClassId.toString(), descriptorContainment(context, declaration))
        record["compilerClassId"] = JsonPrimitive(owner.compilerClassId)
        record["ownerIdentity"] = JsonPrimitive(owner.ownerIdentity)
        record["containment"] = JsonArray(owner.containment.map(::JsonPrimitive))
        record["isPrimary"] = JsonPrimitive(declaration.isPrimary)
        record["jvmDescriptor"] = JsonPrimitive(jvmDescriptor)
        record["parameterTypes"] = JsonArray(
            parameterTypes.filterNotNull().mapIndexed { index, type -> buildJsonObject {
                put("index", index)
                put("type", type.toString())
                put("nullable", type.isMarkedNullable)
                put("hasDefault", declaration.valueParameters[index].defaultValue != null)
            } },
        )
        record["typeParameters"] = JsonArray(emptyList())
        appendFact(output, JsonObject(record))
    }
}

private fun FirCallableSymbol<*>.relationSymbol(): String = callableId.toString()

internal fun compilerJvmMethodDescriptor(declaration: FirFunction): String? = runCatching {
    declaration.computeJvmDescriptor(null, true)
}.getOrNull()
    ?.substringAfter('(', "")
    ?.takeIf(String::isNotEmpty)
    ?.let { "($it" }
    ?.let(::canonicalJvmMethodDescriptor)

private data class ResolvedCallableTarget24(
    val compilerCallableId: String,
    val symbolIdentity: String,
    val jvmDescriptor: String,
)

private data class ResolvedCallableTargetResult24(
    val target: ResolvedCallableTarget24?,
    val unknownCode: String?,
)

/**
 * Bind a call relation to the exact FIR-selected JVM overload. This is not a
 * name/arity reconstruction: both the callable id and descriptor come from
 * the resolved FirCallableSymbol and its resolved FirFunction declaration.
 */
private fun resolvedCallableTarget24(
    callable: FirCallableSymbol<*>,
    session: FirSession,
): ResolvedCallableTargetResult24 {
    val callableId = callable.callableId
        ?: return ResolvedCallableTargetResult24(null, "NO_COMPILER_CALLABLE_ID")
    if (callableId.isLocal) {
        return ResolvedCallableTargetResult24(null, "LOCAL_CALL_TARGET_UNSUPPORTED")
    }
    val function = callable.fir as? FirFunction
        ?: return ResolvedCallableTargetResult24(null, "CALL_TARGET_NOT_FUNCTION")
    if (!function.origin.fromSource || function.origin.generated) {
        return ResolvedCallableTargetResult24(null, "EXTERNAL_OR_GENERATED_CALL_TARGET")
    }
    if (function.hasAnnotationSafe(JvmStandardClassIds.Annotations.JvmName, session)) {
        return ResolvedCallableTargetResult24(null, "JVM_NAME_OVERRIDE_UNSUPPORTED")
    }
    val prefix = when (callable) {
        is FirConstructorSymbol -> "constructor:"
        is FirNamedFunctionSymbol -> "callable:"
        else -> return ResolvedCallableTargetResult24(null, "UNSUPPORTED_CALLABLE_SYMBOL_KIND")
    }
    val descriptor = compilerJvmMethodDescriptor(function)
        ?: return ResolvedCallableTargetResult24(null, "UNRESOLVED_TARGET_JVM_DESCRIPTOR")
    val compilerId = callableId.toString()
    return ResolvedCallableTargetResult24(
        ResolvedCallableTarget24(
            compilerCallableId = compilerId,
            symbolIdentity = "$prefix$compilerId#jvm:$descriptor",
            jvmDescriptor = descriptor,
        ),
        null,
    )
}
private fun descriptorContainment(
    context: CheckerContext,
    declaration: Any,
): List<String> = context.containingElements
    .filterNot { it === declaration }
    .mapNotNull { containing ->
        when (containing) {
            is FirCallableDeclaration ->
                (containing.symbol as? FirCallableSymbol<*>)?.let { "callable:${it.callableId}" }
            is FirRegularClass -> "class:${containing.symbol.classId}"
            else -> null
        }
    }

private fun descriptorOwner(
    context: CheckerContext,
    declaration: Any,
    packageName: String,
): String = descriptorContainment(context, declaration).lastOrNull() ?: "package:$packageName"

private fun resolvedDescriptorType(typeRef: org.jetbrains.kotlin.fir.types.FirTypeRef): ConeKotlinType? =
    (typeRef as? FirResolvedTypeRef)?.coneType

private fun typeParameterDescriptors(parameters: List<FirTypeParameter>) =
    parameters.mapIndexed { index, parameter ->
        val bounds = parameter.bounds.mapNotNull(::resolvedDescriptorType)
        if (bounds.size != parameter.bounds.size) return@mapIndexed null
        buildJsonObject {
            put("index", index)
            put("compilerName", parameter.symbol.name.asString())
            putJsonArray("bounds") {
                bounds.map(ConeKotlinType::toString).sorted().forEach { add(JsonPrimitive(it)) }
            }
        }
    }.takeIf { descriptors -> descriptors.none { it == null } }?.filterNotNull()

private fun descriptorBase(
    context: CheckerContext,
    source: org.jetbrains.kotlin.KtSourceElement,
    declaration: Any,
    symbolIdentity: String,
    declarationKind: String,
    packageName: String,
    visibility: String,
    effectiveVisibility: String,
    publicApi: Boolean,
    privateApi: Boolean,
    modality: String,
) = buildJsonObject {
    put("recordType", "DECLARATION_DESCRIPTOR")
    put("schema", "declaration-descriptor/0.1")
    put("file", context.containingFilePath.orEmpty())
    put("start", source.startOffset)
    put("end", source.endOffset)
    put("symbolIdentity", symbolIdentity)
    put("declarationKind", declarationKind)
    put("ownerIdentity", descriptorOwner(context, declaration, packageName))
    putJsonArray("containment") {
        descriptorContainment(context, declaration).forEach { add(JsonPrimitive(it)) }
    }
    put("visibility", visibility)
    put("effectiveVisibility", effectiveVisibility)
    put("exportBoundary", when {
        publicApi -> "PUBLIC_API"
        privateApi -> "PRIVATE_API"
        else -> "MODULE_API"
    })
    put("modality", modality)
    put("resolution", "PROVEN")
    put("provider", "K2_FIR")
}

private fun unknownDescriptor(
    context: CheckerContext,
    source: org.jetbrains.kotlin.KtSourceElement?,
    symbolIdentity: String?,
    stage: String,
    code: String,
) = buildJsonObject {
    put("recordType", "DECLARATION_DESCRIPTOR_BOUNDARY")
    put("schema", "declaration-descriptor-boundary/0.1")
    put("file", context.containingFilePath.orEmpty())
    source?.let {
        put("start", it.startOffset)
        put("end", it.endOffset)
    }
    symbolIdentity?.let { put("symbolIdentity", it) }
    put("stage", stage)
    put("code", code)
    put("resolution", "UNKNOWN")
    put("provider", "K2_FIR")
}

private class FirFactsFunctionDescriptorChecker(
    private val output: String,
) : FirDeclarationChecker<FirNamedFunction>(MppCheckerKind.Common) {
    context(context: CheckerContext, reporter: DiagnosticReporter)
    override fun check(declaration: FirNamedFunction) {
        val source = declaration.source
        val symbol = declaration.symbol
        val callableId = symbol.callableId
        val provisionalIdentity = "callable:$callableId"
        if (source == null || !declaration.origin.fromSource || declaration.origin.generated) {
            appendFact(output, unknownDescriptor(context, source, provisionalIdentity, "DECLARATION", "GENERATED_OR_NO_SOURCE"))
            return
        }
        if (callableId.isLocal) {
            appendFact(output, unknownDescriptor(context, source, provisionalIdentity, "DECLARATION", "LOCAL_DECLARATION_UNSUPPORTED"))
            return
        }
        val returnType = resolvedDescriptorType(declaration.returnTypeRef)
        val parameterTypes = declaration.valueParameters.map { resolvedDescriptorType(it.returnTypeRef) }
        val receiverType = declaration.receiverParameter?.typeRef?.let(::resolvedDescriptorType)
        val typeParameters = typeParameterDescriptors(declaration.typeParameters)
        val jvmDescriptor = compilerJvmMethodDescriptor(declaration)
        val status = runCatching { symbol.resolvedStatus }.getOrNull()
        if (declaration.hasAnnotationSafe(JvmStandardClassIds.Annotations.JvmName, context.session)) {
            appendFact(
                output,
                unknownDescriptor(
                    context,
                    source,
                    provisionalIdentity,
                    "DECLARATION",
                    "JVM_NAME_OVERRIDE_UNSUPPORTED",
                ),
            )
            return
        }
        if (returnType == null || parameterTypes.any { it == null }
            || declaration.receiverParameter != null && receiverType == null
            || typeParameters == null || jvmDescriptor.isNullOrEmpty() || status == null
        ) {
            appendFact(output, unknownDescriptor(context, source, provisionalIdentity, "DECLARATION", "UNRESOLVED_DESCRIPTOR_BOUNDARY"))
            return
        }
        val symbolIdentity = "$provisionalIdentity#jvm:$jvmDescriptor"
        val record = descriptorBase(
            context,
            source,
            declaration,
            symbolIdentity,
            "FUNCTION",
            callableId.packageName.asString(),
            status.visibility.name,
            status.effectiveVisibility.name,
            status.effectiveVisibility.publicApi,
            status.effectiveVisibility.privateApi,
            status.modality.name,
        ).toMutableMap()
        record["compilerCallableId"] = JsonPrimitive(callableId.toString())
        record["jvmDescriptor"] = JsonPrimitive(jvmDescriptor)
        record["isOverride"] = JsonPrimitive(status.isOverride)
        record["spring"] = springAnnotationFacts24(declaration, context)
        record["returnType"] = JsonPrimitive(returnType.toString())
        record["returnNullable"] = JsonPrimitive(returnType.isMarkedNullable)
        record["parameterTypes"] = kotlinx.serialization.json.JsonArray(
            parameterTypes.filterNotNull().mapIndexed { index, type -> buildJsonObject {
                put("index", index)
                put("type", type.toString())
                put("nullable", type.isMarkedNullable)
                put("hasDefault", declaration.valueParameters[index].defaultValue != null)
            } },
        )
        receiverType?.let {
            record["receiverType"] = buildJsonObject {
                put("type", it.toString())
                put("nullable", it.isMarkedNullable)
            }
        }
        record["typeParameters"] = kotlinx.serialization.json.JsonArray(typeParameters)
        appendFact(output, kotlinx.serialization.json.JsonObject(record))
    }
}

private class FirFactsPropertyDescriptorChecker(
    private val output: String,
) : FirDeclarationChecker<FirProperty>(MppCheckerKind.Common) {
    context(context: CheckerContext, reporter: DiagnosticReporter)
    override fun check(declaration: FirProperty) {
        val source = declaration.source
        val symbol = declaration.symbol
        val callableId = symbol.callableId
        if (callableId == null) {
            appendFact(
                output,
                unknownDescriptor(
                    context,
                    source,
                    null,
                    "DECLARATION",
                    "NO_COMPILER_CALLABLE_ID",
                ),
            )
            return
        }
        val symbolIdentity = "property:$callableId"
        if (source == null || !declaration.origin.fromSource || declaration.origin.generated) {
            appendFact(output, unknownDescriptor(context, source, symbolIdentity, "DECLARATION", "GENERATED_OR_NO_SOURCE"))
            return
        }
        val hasCompilerContainingCallable = context.containingElements.any { containing ->
            containing !== declaration && containing is FirCallableDeclaration
        }
        if (hasCompilerContainingCallable) {
            appendFact(output, unknownDescriptor(context, source, symbolIdentity, "DECLARATION", "LOCAL_DECLARATION_UNSUPPORTED"))
            return
        }
        val declaredType = resolvedDescriptorType(declaration.returnTypeRef)
        val typeParameters = typeParameterDescriptors(declaration.typeParameters)
        val status = runCatching { symbol.resolvedStatus }.getOrNull()
        if (declaredType == null || typeParameters == null || status == null) {
            appendFact(output, unknownDescriptor(context, source, symbolIdentity, "DECLARATION", "UNRESOLVED_DESCRIPTOR_BOUNDARY"))
            return
        }
        val record = descriptorBase(
            context,
            source,
            declaration,
            symbolIdentity,
            if (declaration.isVar) "MUTABLE_PROPERTY" else "PROPERTY",
            callableId.packageName.asString(),
            status.visibility.name,
            status.effectiveVisibility.name,
            status.effectiveVisibility.publicApi,
            status.effectiveVisibility.privateApi,
            status.modality.name,
        ).toMutableMap()
        record["compilerCallableId"] = JsonPrimitive(callableId.toString())
        record["isOverride"] = JsonPrimitive(status.isOverride)
        record["declaredType"] = JsonPrimitive(declaredType.toString())
        record["declaredNullable"] = JsonPrimitive(declaredType.isMarkedNullable)
        record["typeParameters"] = kotlinx.serialization.json.JsonArray(typeParameters)
        appendFact(output, kotlinx.serialization.json.JsonObject(record))
    }
}

private class FirFactsClassDescriptorChecker(
    private val output: String,
) : FirDeclarationChecker<FirRegularClass>(MppCheckerKind.Common) {
    context(context: CheckerContext, reporter: DiagnosticReporter)
    override fun check(declaration: FirRegularClass) {
        val source = declaration.source
        val symbol = declaration.symbol
        val symbolIdentity = "class:${symbol.classId}"
        if (source == null || !declaration.origin.fromSource || declaration.origin.generated || symbol.classId.isLocal) {
            appendFact(output, unknownDescriptor(context, source, symbolIdentity, "DECLARATION", "LOCAL_GENERATED_OR_NO_SOURCE"))
            return
        }
        val typeParameters = typeParameterDescriptors(declaration.typeParameters.filterIsInstance<FirTypeParameter>())
        val status = runCatching { symbol.resolvedStatus }.getOrNull()
        if (typeParameters == null || status == null) {
            appendFact(output, unknownDescriptor(context, source, symbolIdentity, "DECLARATION", "UNRESOLVED_DESCRIPTOR_BOUNDARY"))
            return
        }
        val record = descriptorBase(
            context,
            source,
            declaration,
            symbolIdentity,
            "CLASS",
            symbol.classId.packageFqName.asString(),
            status.visibility.name,
            status.effectiveVisibility.name,
            status.effectiveVisibility.publicApi,
            status.effectiveVisibility.privateApi,
            status.modality.name,
        ).toMutableMap()
        record["compilerClassId"] = JsonPrimitive(symbol.classId.toString())
        record["spring"] = springClassAnnotationFacts24(declaration, context)
        record["typeParameters"] = kotlinx.serialization.json.JsonArray(typeParameters)
        appendFact(output, kotlinx.serialization.json.JsonObject(record))
    }
}



private fun relationOwner(context: CheckerContext): String? {
    for (declaration in context.containingElements.asReversed()) {
        when (declaration) {
            is FirAnonymousFunction -> continue
            is FirCallableDeclaration -> {
                val symbol = declaration.symbol as? FirCallableSymbol<*> ?: return null
                val callableId = symbol.callableId
                // A local `val` is a lexical carrier for its initializer, not
                // the stable callable that owns relations inside that value.
                // Skip it, but keep local functions fail-closed below so their
                // calls cannot be attributed to an outer callable.
                if (declaration is FirProperty && (callableId == null || callableId.isLocal)) {
                    continue
                }
                callableId ?: return null
                if (
                    callableId.isLocal ||
                    !declaration.origin.fromSource ||
                    declaration.origin.generated ||
                    declaration.source == null
                ) {
                    return null
                }
                return callableId.toString()
            }
            is FirRegularClass -> {
                val symbol = declaration.symbol as? FirClassSymbol<*> ?: return null
                val classId = symbol.classId
                if (
                    classId.isLocal ||
                    !declaration.origin.fromSource ||
                    declaration.origin.generated ||
                    declaration.source == null
                ) {
                    return null
                }
                return classId.toString()
            }
            else -> continue
        }
    }
    return null
}

private fun relationBase(
    context: CheckerContext,
    source: org.jetbrains.kotlin.KtSourceElement,
    kind: String,
    owner: String,
    target: String,
) = buildJsonObject {
    put("recordType", "DECLARATION_RELATION")
    put("schema", "declaration-relation/0.1")
    put("file", context.containingFilePath.orEmpty())
    put("start", source.startOffset)
    put("end", source.endOffset)
    put("kind", kind)
    put("owner", owner)
    put("target", target)
    put("resolution", "PROVEN")
    put("provider", "K2_FIR")
}

private fun unknownRelation(
    context: CheckerContext,
    source: org.jetbrains.kotlin.KtSourceElement,
    owner: String?,
    stage: String,
    code: String,
) = buildJsonObject {
    put("recordType", "DECLARATION_RELATION_BOUNDARY")
    put("schema", "declaration-relation-boundary/0.1")
    put("file", context.containingFilePath.orEmpty())
    put("start", source.startOffset)
    put("end", source.endOffset)
    owner?.let { put("owner", it) }
    put("stage", stage)
    put("code", code)
    put("resolution", "UNKNOWN")
    put("provider", "K2_FIR")
}

private data class ResolvedArgumentMapping24(
    val rows: List<JsonObject>?,
    val unknownCode: String?,
    val omittedDefaultParameterIndices: List<Int>? = null,
)

private fun resolvedArgumentMapping24(
    callable: FirCallableSymbol<*>,
    arguments: FirResolvedArgumentList?,
): ResolvedArgumentMapping24 {
    val function = callable.fir as? FirFunction
        ?: return ResolvedArgumentMapping24(null, "ARGUMENT_OWNER_NOT_FUNCTION")
    val callableId = callable.callableId
        ?: return ResolvedArgumentMapping24(null, "NO_COMPILER_CALLABLE_ID")
    if (!function.origin.fromSource || function.origin.generated || callableId.isLocal) {
        return ResolvedArgumentMapping24(null, "EXTERNAL_OR_LOCAL_ARGUMENT_TARGET")
    }
    if (function.receiverParameter != null) {
        return ResolvedArgumentMapping24(null, "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED")
    }
    if (function.contextParameters.isNotEmpty()) {
        return ResolvedArgumentMapping24(null, "CONTEXT_ARGUMENT_MAPPING_UNSUPPORTED")
    }
    if (function.status.isSuspend) {
        return ResolvedArgumentMapping24(null, "SUSPEND_ARGUMENT_MAPPING_UNSUPPORTED")
    }
    if (function.valueParameters.any { it.isVararg }) {
        return ResolvedArgumentMapping24(null, "VARARG_ARGUMENT_MAPPING_UNSUPPORTED")
    }
    val resolved = arguments
        ?: return ResolvedArgumentMapping24(null, "MISSING_RESOLVED_ARGUMENT_MAPPING")
    val rows = ArrayList<JsonObject>()
    for ((argument, parameter) in resolved.mapping.entries) {
        val matches = function.valueParameters.withIndex().filter { (_, candidate) ->
            candidate === parameter || candidate.symbol === parameter.symbol
        }
        if (matches.size != 1) {
            return ResolvedArgumentMapping24(null, "UNRESOLVED_PARAMETER_IDENTITY")
        }
        val argumentSource = argument.source
            ?: return ResolvedArgumentMapping24(null, "MISSING_ARGUMENT_SOURCE")
        rows += buildJsonObject {
            put("argumentStart", argumentSource.startOffset)
            put("argumentEnd", argumentSource.endOffset)
            (argument as? FirNamedArgumentExpression)?.name?.asString()?.let {
                put("argumentName", it)
            }
            put("argumentType", argument.resolvedType.toString())
            put("parameter", parameter.name.asString())
            put("parameterIndex", matches.single().index)
            put(
                "parameterType",
                (parameter.returnTypeRef as? FirResolvedTypeRef)?.coneType?.toString()
                    ?: "<unresolved>",
            )
        }
    }
    if (rows.size != resolved.mapping.size) {
        return ResolvedArgumentMapping24(null, "INCOMPLETE_ARGUMENT_MAPPING")
    }
    val mappedIndices = rows.mapNotNull {
        (it["parameterIndex"] as? JsonPrimitive)?.content?.toIntOrNull()
    }.toSet()
    val omitted = function.valueParameters.indices.filterNot(mappedIndices::contains)
    if (omitted.any { function.valueParameters[it].defaultValue == null }) {
        return ResolvedArgumentMapping24(null, "UNMAPPED_NON_DEFAULT_PARAMETER")
    }
    return ResolvedArgumentMapping24(
        rows.sortedWith(compareBy<JsonObject> {
            it["argumentStart"]?.let { start -> (start as JsonPrimitive).content.toInt() }
        }.thenBy {
            it["parameterIndex"]?.let { index -> (index as JsonPrimitive).content.toInt() }
        }),
        null,
        omitted,
    )
}

private class FirFactsOverrideChecker(
    private val output: String,
) : FirDeclarationChecker<FirNamedFunction>(MppCheckerKind.Common) {
    context(context: CheckerContext, reporter: DiagnosticReporter)
    override fun check(declaration: FirNamedFunction) {
        if (!declaration.status.isOverride) return
        val source = declaration.source ?: return
        val containingClass = declaration.symbol.dispatchReceiverType
            ?.classId
            ?.takeUnless { it.isLocal }
            ?.let(context.session::getRegularClassSymbolByClassId)
        val scope = containingClass?.unsubstitutedScope(
            context.session,
            context.scopeSession,
            false,
            FirResolvePhase.BODY_RESOLVE,
        )
        val symbol = declaration.symbol as? FirNamedFunctionSymbol
        if (scope == null || symbol == null) {
            appendFact(output, unknownRelation(context, source, symbol?.relationSymbol(), "OVERRIDE", "NO_RESOLVED_CLASS_SCOPE"))
            return
        }
        scope.processFunctionsByName(symbol.name) { }
        val bases = scope.getDirectOverriddenSafe(symbol)
        if (bases.isEmpty()) {
            appendFact(output, unknownRelation(context, source, symbol.relationSymbol(), "OVERRIDE", "NO_RESOLVED_BASE"))
            return
        }
        bases.sortedBy { it.callableId.toString() }.forEach { base ->
            val baseFunction = base.fir as? FirFunction
            if (baseFunction == null) {
                appendFact(
                    output,
                    unknownRelation(
                        context,
                        source,
                        symbol.relationSymbol(),
                        "OVERRIDE",
                        "NON_FUNCTION_RESOLVED_BASE",
                    ),
                )
                return@forEach
            }
            val record = relationBase(context, source, "OVERRIDES", symbol.relationSymbol(), base.relationSymbol()).toMutableMap()
            record["sourceReturnType"] = JsonPrimitive(symbol.resolvedReturnType.toString())
            record["baseReturnType"] = JsonPrimitive(base.resolvedReturnType.toString())
            record["sourceParameterTypes"] = JsonArray(
                declaration.valueParameters.map {
                    JsonPrimitive((it.returnTypeRef as? FirResolvedTypeRef)?.coneType?.toString() ?: "<unresolved>")
                },
            )
            record["baseParameterTypes"] = JsonArray(
                baseFunction.valueParameters.map {
                    JsonPrimitive((it.returnTypeRef as? FirResolvedTypeRef)?.coneType?.toString() ?: "<unresolved>")
                },
            )
            appendFact(output, JsonObject(record))
        }
    }
}

private class FirFactsPropertyChecker(
    private val output: String,
) : FirDeclarationChecker<FirProperty>(MppCheckerKind.Common) {
    context(context: CheckerContext, reporter: DiagnosticReporter)
    override fun check(declaration: FirProperty) {
        if (declaration.status.isOverride) {
            val boundarySource = declaration.source ?: return
            appendFact(
                output,
                unknownRelation(
                    context,
                    boundarySource,
                    declaration.symbol.relationSymbol(),
                    "OVERRIDE",
                    "NON_FUNCTION_OVERRIDE_UNSUPPORTED",
                ),
            )
        }
        val source = declaration.initializer?.source ?: return
        val target = declaration.symbol.relationSymbol()
        val owner = context.containingElements.asReversed()
            .filterNot { it === declaration }
            .mapNotNull { candidate ->
                when (candidate) {
                    is FirCallableDeclaration -> (candidate.symbol as? FirCallableSymbol<*>)?.relationSymbol()
                    is FirRegularClass -> candidate.symbol.classId.toString()
                    else -> null
                }
            }
            .firstOrNull()
        if (owner == null) {
            appendFact(output, unknownRelation(context, source, null, "INITIALIZER", "NO_RESOLVED_OWNER"))
            return
        }
        val record = relationBase(context, source, "INITIALIZES", owner, target).toMutableMap()
        record["valueType"] = JsonPrimitive(declaration.initializer?.resolvedType?.toString() ?: "<unresolved>")
        record["targetType"] = JsonPrimitive((declaration.returnTypeRef as? FirResolvedTypeRef)?.coneType?.toString() ?: "<unresolved>")
        record["orderKey"] = JsonPrimitive(source.startOffset)
        record["orderProvenance"] = JsonPrimitive("FIR_SOURCE_RANGE")
        appendFact(output, JsonObject(record))
    }
}

private class FirFactsCfgChecker(
    private val output: String,
) : FirDeclarationChecker<FirFunction>(MppCheckerKind.Common) {
    context(context: CheckerContext, reporter: DiagnosticReporter)
    override fun check(declaration: FirFunction) {
        val extractionStarted = System.nanoTime()
        val source = declaration.source ?: return
        if (declaration.body is FirSingleExpressionBlock) {
            appendFact(
                output,
                unknownRelation(
                    context,
                    source,
                    (declaration.symbol as? FirCallableSymbol<*>)?.callableId?.toString(),
                    "RETURN_VALUE",
                    "IMPLICIT_RETURN_UNSUPPORTED",
                ),
            )
        }
        val graph = (declaration.controlFlowGraphReference as? FirControlFlowGraphReferenceImpl)
            ?.controlFlowGraph ?: return
        val record = buildJsonObject {
            put("recordType", "FIR_CFG")
            put("file", context.containingFilePath ?: return)
            put("start", source.startOffset)
            put("end", source.endOffset)
            put("name", graph.name)
            (declaration.symbol as? FirCallableSymbol<*>)?.let { put("symbol", it.callableId.toString()) }
            compilerJvmMethodDescriptor(declaration)?.let { put("jvmDescriptor", it) }
            put(
                "returnType",
                (declaration.returnTypeRef as? FirResolvedTypeRef)?.coneType?.toString() ?: "<unresolved>",
            )
            putJsonArray("parameterTypes") {
                declaration.valueParameters.forEach { parameter ->
                    add(JsonPrimitive(
                        (parameter.returnTypeRef as? FirResolvedTypeRef)?.coneType?.toString()
                            ?: "<unresolved>",
                    ))
                }
            }
            declaration.receiverParameter?.let { receiver ->
                put(
                    "receiverType",
                    (receiver.typeRef as? FirResolvedTypeRef)?.coneType?.toString() ?: "<unresolved>",
                )
            }
            putJsonArray("nodes") {
                graph.nodes.sortedBy { it.id }.forEach { node ->
                    add(buildJsonObject {
                        put("id", node.id)
                        put("kind", node::class.simpleName ?: "CFGNode")
                        put("dead", node.isDead)
                        node.fir.source?.let {
                            put("start", it.startOffset)
                            put("end", it.endOffset)
                        }
                    })
                }
            }
            putJsonArray("edges") {
                graph.nodes.sortedBy { it.id }.forEach { from ->
                    from.followingNodes.sortedBy { it.id }.forEach { to ->
                        val edge = from.edgeTo(to)
                        add(buildJsonObject {
                            put("from", from.id)
                            put("to", to.id)
                            put("label", edge.label.toString())
                            put("edgeKind", edge.kind.toString())
                        })
                    }
                }
            }
            put("firExtractionMicros", (System.nanoTime() - extractionStarted) / 1_000)
        }
        appendFact(output, record)
    }
}

private fun resolvedOccurrenceSymbol(expression: FirExpression): FirCallableSymbol<*>? =
    ((expression as? FirQualifiedAccessExpression)?.calleeReference as? FirResolvedNamedReference)
        ?.resolvedSymbol as? FirCallableSymbol<*>

private data class ReturnBodyShape24(
    var returnCount: Int = 0,
    var branchCount: Int = 0,
    var tryCount: Int = 0,
    var safeCallCount: Int = 0,
    var elvisCount: Int = 0,
)

private fun returnBodyShape24(function: FirFunction): ReturnBodyShape24 {
    val shape = ReturnBodyShape24()
    function.body?.accept(object : FirVisitorVoid() {
        override fun visitElement(element: org.jetbrains.kotlin.fir.FirElement) {
            when (element) {
                is FirReturnExpression -> shape.returnCount += 1
                is FirWhenExpression -> shape.branchCount += 1
                is FirTryExpression -> shape.tryCount += 1
                is FirSafeCallExpression -> shape.safeCallCount += 1
                is FirElvisExpression -> shape.elvisCount += 1
            }
            element.acceptChildren(this)
        }
    })
    return shape
}

private fun resolvedReturnValueOccurrences24(
    expression: FirExpression,
): List<FirCallableSymbol<*>> {
    val occurrences = mutableListOf<FirCallableSymbol<*>>()
    expression.accept(object : FirVisitorVoid() {
        override fun visitElement(element: org.jetbrains.kotlin.fir.FirElement) {
            val access = element as? FirQualifiedAccessExpression
            val symbol = (access?.calleeReference as? FirResolvedNamedReference)
                ?.resolvedSymbol as? FirCallableSymbol<*>
            if (symbol is FirPropertySymbol || symbol is FirNamedFunctionSymbol) {
                occurrences += symbol
            }
            element.acceptChildren(this)
        }
    })
    return occurrences
}

private fun returnDiagnosticHash24(value: String): String = "sha256:" +
    MessageDigest.getInstance("SHA-256")
        .digest(value.toByteArray())
        .joinToString("") { byte -> "%02x".format(byte) }

private fun cfgReaches24(
    from: org.jetbrains.kotlin.fir.resolve.dfa.cfg.CFGNode<*>,
    to: org.jetbrains.kotlin.fir.resolve.dfa.cfg.CFGNode<*>,
    blocked: org.jetbrains.kotlin.fir.resolve.dfa.cfg.CFGNode<*>? = null,
): Boolean {
    if (from === blocked) return false
    val pending = ArrayDeque<org.jetbrains.kotlin.fir.resolve.dfa.cfg.CFGNode<*>>()
    val visited = mutableSetOf<Int>()
    pending.add(from)
    while (pending.isNotEmpty()) {
        val node = pending.removeFirst()
        if (node === blocked || !visited.add(node.id)) continue
        if (node === to) return true
        node.followingNodes.filterNot { it.isDead }.forEach(pending::addLast)
    }
    return false
}

private fun emitDirectReturnValueFact24(
    output: String,
    expression: FirReturnExpression,
    context: CheckerContext,
) {
    val source = expression.source
    val result = expression.result
    val resultSource = result.source
    val owner = context.containingElements.asReversed()
        .filterIsInstance<FirFunction>()
        .firstOrNull()
    val ownerSymbol = owner?.symbol as? FirCallableSymbol<*>
    val ownerId = ownerSymbol?.callableId
    val diagnosticOccurrences = resolvedReturnValueOccurrences24(result)
    fun refuse(code: String) {
        val boundary = unknownRelation(
            context,
            source ?: resultSource ?: return,
            ownerId?.toString(),
            "RETURN_VALUE",
            code,
        ).toMutableMap()
        boundary["ownerIdentityHash"] = JsonPrimitive(
            returnDiagnosticHash24(ownerId?.toString() ?: "<unknown>"),
        )
        boundary["rootFirKindHash"] = JsonPrimitive(
            returnDiagnosticHash24(result::class.qualifiedName ?: "<unknown>"),
        )
        boundary["nestedResolvedOccurrenceCount"] = JsonPrimitive(diagnosticOccurrences.size)
        boundary["nestedResolvedOccurrenceKindHashes"] = JsonArray(
            diagnosticOccurrences
                .map { occurrence ->
                    returnDiagnosticHash24(occurrence::class.qualifiedName ?: "<unknown>")
                }
                .sorted()
                .map(::JsonPrimitive),
        )
        appendFact(output, JsonObject(boundary))
    }

    if (source == null || resultSource == null || source.kind !== KtRealSourceElementKind) {
        refuse("IMPLICIT_OR_MISSING_RETURN_SOURCE")
        return
    }
    if (owner == null || ownerSymbol == null || ownerId == null) {
        refuse("UNRESOLVED_RETURN_OWNER")
        return
    }
    if (!owner.origin.fromSource || owner.origin.generated || ownerId.isLocal) {
        refuse("LOCAL_OR_GENERATED_RETURN_OWNER")
        return
    }
    if (expression.target.labeledElement !== owner) {
        refuse("RETURN_TARGET_IDENTITY_MISMATCH")
        return
    }
    val bodyShape = returnBodyShape24(owner)
    if (bodyShape.returnCount != 1 || bodyShape.branchCount != 0
        || bodyShape.tryCount != 0 || bodyShape.safeCallCount != 0
        || bodyShape.elvisCount != 0
    ) {
        refuse("NON_LINEAR_OR_MULTIPLE_RETURN_FLOW")
        return
    }
    val target = resolvedOccurrenceSymbol(result)
    val kind = when {
        result is FirFunctionCall && target is FirNamedFunctionSymbol -> "FUNCTION_CALL_RESULT"
        result is FirQualifiedAccessExpression && result !is FirFunctionCall
            && target is FirPropertySymbol -> "PROPERTY_READ"
        else -> null
    }
    if (target == null || kind == null) {
        refuse("RETURN_VALUE_NOT_DIRECT_RESOLVED_READ_OR_CALL")
        return
    }
    val occurrences = diagnosticOccurrences
    if (occurrences.size != 1 || occurrences.single() !== target) {
        refuse("MULTIPLE_OR_AMBIGUOUS_RETURN_VALUE_OCCURRENCES")
        return
    }
    val targetId = target.callableId
    if (targetId == null || targetId.isLocal
        || !target.fir.origin.fromSource || target.fir.origin.generated
    ) {
        refuse("LOCAL_GENERATED_OR_UNRESOLVED_RETURN_VALUE")
        return
    }
    val graph = (owner.controlFlowGraphReference as? FirControlFlowGraphReferenceImpl)
        ?.controlFlowGraph
    if (graph == null) {
        refuse("MISSING_RETURN_CFG")
        return
    }
    val sourceNodes = graph.nodes.filter { node ->
        node.fir === result && when (kind) {
            "FUNCTION_CALL_RESULT" -> node::class.simpleName == "FunctionCallExitNode"
            "PROPERTY_READ" -> node::class.simpleName == "QualifiedAccessNode"
            else -> false
        }
    }
    val returnNodes = graph.nodes.filter { node ->
        node.fir === expression && node::class.simpleName == "JumpNode"
    }
    if (sourceNodes.size != 1 || returnNodes.size != 1) {
        refuse("AMBIGUOUS_RETURN_CFG_NODE")
        return
    }
    val sourceNode = sourceNodes.single()
    val returnNode = returnNodes.single()
    val sourceReachesReturn = cfgReaches24(sourceNode, returnNode)
    val sourceDominatesReturn = sourceReachesReturn
        && !cfgReaches24(graph.enterNode, returnNode, sourceNode)
    if (sourceNode.isDead || returnNode.isDead || !sourceReachesReturn || !sourceDominatesReturn) {
        refuse("RETURN_VALUE_CFG_PROOF_UNAVAILABLE")
        return
    }
    val resultType = result.resolvedType
    val record = relationBase(
        context,
        source,
        "RETURNS_VALUE_FROM",
        ownerId.toString(),
        targetId.toString(),
    ).toMutableMap()
    record["sourceKind"] = JsonPrimitive(kind)
    record["sourceOccurrence"] = buildJsonObject {
        put("start", resultSource.startOffset)
        put("end", resultSource.endOffset)
        put("cfgNodeId", sourceNode.id)
    }
    record["returnOccurrence"] = buildJsonObject {
        put("start", source.startOffset)
        put("end", source.endOffset)
        put("cfgNodeId", returnNode.id)
    }
    record["resultOccurrence"] = buildJsonObject {
        put("start", resultSource.startOffset)
        put("end", resultSource.endOffset)
        put("cfgNodeId", sourceNode.id)
    }
    record["resultType"] = JsonPrimitive(resultType.toString())
    record["resultNullable"] = JsonPrimitive(resultType.isMarkedNullable)
    record["valueProvenance"] = JsonPrimitive("FIR_RETURN_RESULT_IDENTITY")
    record["cfgProvenance"] = buildJsonObject {
        put("graphName", graph.name)
        put("sourceReachesReturn", true)
        put("sourceDominatesReturn", true)
        put("sourceNodeKind", sourceNode::class.simpleName ?: "CFGNode")
        put("returnNodeKind", returnNode::class.simpleName ?: "CFGNode")
    }
    record["evaluationCount"] = JsonPrimitive(1)
    record["orderKey"] = JsonPrimitive(resultSource.startOffset)
    appendFact(output, JsonObject(record))
}

private fun emitNullCoalescingFact(
    output: String,
    expression: FirElvisExpression,
    context: CheckerContext,
) {
    val source = expression.source ?: return
    val owner = relationOwner(context)
    val lhsSource = expression.lhs.source
    val rhsSource = expression.rhs.source
    val lhsSymbol = resolvedOccurrenceSymbol(expression.lhs)
    val rhsSymbol = resolvedOccurrenceSymbol(expression.rhs)
    val unknown = when {
        owner == null -> "UNRESOLVED_NULL_POLICY_OWNER"
        lhsSource == null || rhsSource == null -> "MISSING_NULL_POLICY_OCCURRENCE"
        lhsSymbol == null -> "UNRESOLVED_NULLABLE_SOURCE_OCCURRENCE"
        rhsSymbol == null -> "UNRESOLVED_FALLBACK_OCCURRENCE"
        !expression.lhs.resolvedType.isMarkedNullable -> "SOURCE_OCCURRENCE_NOT_NULLABLE"
        expression.rhs.resolvedType.isMarkedNullable -> "FALLBACK_OCCURRENCE_NULLABLE"
        expression.resolvedType.isMarkedNullable -> "MERGED_RESULT_NULLABLE"
        else -> null
    }
    if (unknown != null) {
        appendFact(output, unknownRelation(context, source, owner, "NULL_POLICY", unknown))
        return
    }
    val exactOwner = owner ?: return
    val exactLhsSource = lhsSource ?: return
    val exactRhsSource = rhsSource ?: return
    val exactLhsSymbol = lhsSymbol ?: return
    val exactRhsSymbol = rhsSymbol ?: return
    val relation = relationBase(
        context,
        source,
        "NULL_COALESCES",
        exactOwner,
        exactRhsSymbol.relationSymbol(),
    ).toMutableMap()
    relation["sourceTarget"] = JsonPrimitive(exactLhsSymbol.relationSymbol())
    relation["fallbackTarget"] = JsonPrimitive(exactRhsSymbol.relationSymbol())
    relation["sourceOccurrence"] = buildJsonObject {
        put("start", exactLhsSource.startOffset)
        put("end", exactLhsSource.endOffset)
        put("type", expression.lhs.resolvedType.toString())
        put("nullable", true)
    }
    relation["fallbackOccurrence"] = buildJsonObject {
        put("start", exactRhsSource.startOffset)
        put("end", exactRhsSource.endOffset)
        put("type", expression.rhs.resolvedType.toString())
        put("nullable", false)
    }
    relation["mergedOccurrence"] = buildJsonObject {
        put("start", source.startOffset)
        put("end", source.endOffset)
        put("type", expression.resolvedType.toString())
        put("nullable", false)
    }
    relation["branchProvenance"] = buildJsonObject {
        put("kind", "FIR_ELVIS_EXPRESSION")
        put("nullableBranchStart", exactLhsSource.startOffset)
        put("fallbackBranchStart", exactRhsSource.startOffset)
        put("mergeStart", source.startOffset)
        put("mergeEnd", source.endOffset)
    }
    relation["orderKey"] = JsonPrimitive(source.startOffset)
    appendFact(output, JsonObject(relation))
}

private class FirFactsExpressionChecker(
    private val output: String,
) : FirExpressionChecker<FirStatement>(MppCheckerKind.Common) {
    context(context: CheckerContext, reporter: DiagnosticReporter)
    override fun check(expression: FirStatement) {
        if (expression is FirReturnExpression) {
            emitDirectReturnValueFact24(output, expression, context)
        }
        val source = expression.source ?: return
        val owner = relationOwner(context)
        val assignment = expression as? FirVariableAssignment
        if (assignment != null) {
            val lvalue = assignment.lValue as? FirQualifiedAccessExpression
            val target = (lvalue?.calleeReference as? FirResolvedNamedReference)
                ?.resolvedSymbol as? FirPropertySymbol
            if (owner == null || target == null) {
                appendFact(output, unknownRelation(context, source, owner, "WRITE", "UNRESOLVED_PROPERTY_TARGET"))
            } else {
                val record = relationBase(context, source, "WRITES", owner, target.relationSymbol()).toMutableMap()
                record["valueType"] = JsonPrimitive(assignment.rValue.resolvedType.toString())
                record["targetType"] = JsonPrimitive(target.resolvedReturnType.toString())
                record["orderKey"] = JsonPrimitive(source.startOffset)
                appendFact(output, JsonObject(record))
            }
            return
        }
        val value = expression as? FirExpression ?: return
        if (value is FirSafeCallExpression) {
            appendFact(
                output,
                unknownRelation(
                    context,
                    source,
                    owner,
                    "NULL_POLICY",
                    "SAFE_CALL_POLICY_UNSUPPORTED",
                ),
            )
        }
        if (value is FirElvisExpression) {
            emitNullCoalescingFact(output, value, context)
        }
        val access = value as? FirQualifiedAccessExpression
        val resolved = access?.calleeReference as? FirResolvedNamedReference
        val callable = resolved?.resolvedSymbol as? FirCallableSymbol<*>
        val explicitReceiver = access?.explicitReceiver
        val extensionReceiver = access?.extensionReceiver
        val dispatchReceiver = access?.dispatchReceiver
        val receiver = explicitReceiver ?: extensionReceiver ?: dispatchReceiver
        val receiverSelection = when {
            explicitReceiver != null -> "EXPLICIT"
            extensionReceiver != null && dispatchReceiver != null -> "EXTENSION_AND_DISPATCH"
            extensionReceiver != null -> "EXTENSION"
            dispatchReceiver != null -> "DISPATCH"
            else -> "NONE"
        }
        val arguments = (value as? FirFunctionCall)?.argumentList as? FirResolvedArgumentList
        if (access != null) {
            if (owner == null || callable == null) {
                appendFact(output, unknownRelation(context, source, owner, "REFERENCE", "UNRESOLVED_CALLABLE_TARGET"))
            } else {
                val callableFamily = callable.relationSymbol()
                val reference = relationBase(context, source, "REFERENCES", owner, callableFamily).toMutableMap()
                reference["resultType"] = JsonPrimitive(value.resolvedType.toString())
                receiver?.let { reference["receiverType"] = JsonPrimitive(it.resolvedType.toString()) }
                appendFact(output, JsonObject(reference))
                if (callableFamily.startsWith("kotlin/reflect/") || callableFamily.startsWith("java/lang/reflect/")) {
                    appendFact(output, unknownRelation(context, source, owner, "REFERENCE", "DYNAMIC_REFLECTION_BOUNDARY"))
                }
                val relationKind = when {
                    callable is FirConstructorSymbol -> "CONSTRUCTS"
                    value is FirFunctionCall -> "CALLS"
                    callable is FirPropertySymbol -> "READS"
                    else -> null
                }
                if (relationKind != null) {
                    val argumentMapping = if (relationKind == "CALLS" || relationKind == "CONSTRUCTS") {
                        resolvedArgumentMapping24(callable, arguments)
                    } else {
                        ResolvedArgumentMapping24(emptyList(), null)
                    }
                    if (argumentMapping.unknownCode != null) {
                        appendFact(
                            output,
                            unknownRelation(
                                context,
                                source,
                                owner,
                                "ARGUMENT_MAPPING",
                                argumentMapping.unknownCode,
                            ),
                        )
                        return
                    }
                    val exactReceiverEligible = relationKind == "CONSTRUCTS" || receiverSelection == "EXPLICIT"
                    if (relationKind == "CALLS" && !exactReceiverEligible) {
                        appendFact(
                            output,
                            unknownRelation(
                                context,
                                source,
                                owner,
                                "RECEIVER_IDENTITY",
                                if (receiverSelection == "EXTENSION_AND_DISPATCH") {
                                    "MULTIPLE_IMPLICIT_RECEIVERS_UNSUPPORTED"
                                } else {
                                    "NON_EXPLICIT_CALL_RECEIVER_UNSUPPORTED"
                                },
                            ),
                        )
                    }
                    val exactTarget = if (
                        (relationKind == "CALLS" || relationKind == "CONSTRUCTS") &&
                        exactReceiverEligible
                    ) {
                        resolvedCallableTarget24(callable, context.session)
                    } else {
                        ResolvedCallableTargetResult24(null, null)
                    }
                    if ((relationKind == "CALLS" || relationKind == "CONSTRUCTS") && exactReceiverEligible) {
                        if (exactTarget.unknownCode != null || exactTarget.target == null) {
                            appendFact(
                                output,
                                unknownRelation(
                                    context,
                                    source,
                                    owner,
                                    "TARGET_IDENTITY",
                                    exactTarget.unknownCode ?: "UNRESOLVED_TARGET_IDENTITY",
                                ),
                            )
                            return
                        }
                    }
                    val target = exactTarget.target
                    val relation = relationBase(
                        context,
                        source,
                        relationKind,
                        owner,
                        target?.symbolIdentity ?: callableFamily,
                    ).toMutableMap()
                    target?.let {
                        relation["targetCompilerCallableId"] = JsonPrimitive(it.compilerCallableId)
                        relation["targetJvmDescriptor"] = JsonPrimitive(it.jvmDescriptor)
                    }
                    if (relationKind == "CALLS" || relationKind == "CONSTRUCTS") {
                        if (relationKind == "CONSTRUCTS") {
                            relation["receiverSelection"] = JsonPrimitive("NONE")
                        } else if (receiverSelection != "EXTENSION_AND_DISPATCH") {
                            relation["receiverSelection"] = JsonPrimitive(receiverSelection)
                        }
                        relation["omittedDefaultParameterIndices"] = JsonArray(
                            argumentMapping.omittedDefaultParameterIndices.orEmpty()
                                .map(::JsonPrimitive),
                        )
                    }
                    relation["resultType"] = JsonPrimitive(value.resolvedType.toString())
                    if (relationKind != "CONSTRUCTS") {
                        receiver?.let { relation["receiverType"] = JsonPrimitive(it.resolvedType.toString()) }
                    }
                    relation["argumentToParameter"] = JsonArray(
                        argumentMapping.rows.orEmpty(),
                    )
                    relation["orderKey"] = JsonPrimitive(source.startOffset)
                    appendFact(output, JsonObject(relation))
                }
            }
        }
        val fact = buildJsonObject {
            put("recordType", "SEMANTIC_FACT")
            put("file", context.containingFilePath ?: return)
            put("start", source.startOffset)
            put("end", source.endOffset)
            put("kind", value::class.simpleName ?: "FirExpression")
            put("type", value.resolvedType.toString())
            callable?.let {
                put("symbol", it.callableId.toString())
                put("returnType", it.resolvedReturnType.toString())
            }
            putJsonArray("effects") {
                if ((callable?.fir as? FirFunction)?.status?.isSuspend == true) {
                    add(JsonPrimitive("SUSPEND"))
                }
            }
            receiver?.let { put("receiverType", it.resolvedType.toString()) }
            putJsonArray("argumentToParameter") {
                if (value is FirFunctionCall && callable != null) {
                    resolvedArgumentMapping24(callable, arguments).rows.orEmpty().forEach(::add)
                }
            }
        }
        appendFact(output, fact)
    }
}

private fun appendFact(output: String, fact: JsonObject) =
    synchronized(FirFactsExpressionChecker::class.java) {
        Files.writeString(
            Path.of(output),
            fact.toString() + "\n",
            StandardOpenOption.CREATE,
            StandardOpenOption.APPEND,
        )
    }
