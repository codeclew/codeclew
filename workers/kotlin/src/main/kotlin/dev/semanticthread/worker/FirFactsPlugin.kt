@file:OptIn(org.jetbrains.kotlin.compiler.plugin.ExperimentalCompilerApi::class, org.jetbrains.kotlin.fir.symbols.SymbolInternals::class)

package dev.semanticthread.worker

import kotlinx.serialization.json.*
import org.jetbrains.kotlin.compiler.plugin.*
import org.jetbrains.kotlin.config.CompilerConfiguration
import org.jetbrains.kotlin.config.CompilerConfigurationKey
import org.jetbrains.kotlin.diagnostics.DiagnosticReporter
import org.jetbrains.kotlin.fir.FirSession
import org.jetbrains.kotlin.fir.analysis.checkers.MppCheckerKind
import org.jetbrains.kotlin.fir.analysis.checkers.context.CheckerContext
import org.jetbrains.kotlin.fir.analysis.checkers.expression.ExpressionCheckers
import org.jetbrains.kotlin.fir.analysis.checkers.expression.FirExpressionChecker
import org.jetbrains.kotlin.fir.analysis.checkers.declaration.DeclarationCheckers
import org.jetbrains.kotlin.fir.analysis.checkers.declaration.FirDeclarationChecker
import org.jetbrains.kotlin.fir.analysis.extensions.FirAdditionalCheckersExtension
import org.jetbrains.kotlin.fir.declarations.FirFunction
import org.jetbrains.kotlin.fir.expressions.*
import org.jetbrains.kotlin.fir.expressions.impl.FirResolvedArgumentList
import org.jetbrains.kotlin.fir.extensions.FirExtensionRegistrar
import org.jetbrains.kotlin.fir.extensions.FirExtensionRegistrarAdapter
import org.jetbrains.kotlin.fir.references.FirResolvedNamedReference
import org.jetbrains.kotlin.fir.resolve.dfa.FirControlFlowGraphReferenceImpl
import org.jetbrains.kotlin.fir.symbols.impl.FirCallableSymbol
import org.jetbrains.kotlin.fir.types.resolvedType
import org.jetbrains.kotlin.fir.types.FirResolvedTypeRef
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardOpenOption

internal const val FACTS_PLUGIN_ID = "semantic-thread-facts"
private val OUTPUT_KEY = CompilerConfigurationKey<String>("semantic thread FIR facts output")

class FirFactsCommandLineProcessor : CommandLineProcessor {
    override val pluginId: String = FACTS_PLUGIN_ID
    override val pluginOptions = listOf(CliOption("output", "path", "FIR facts JSONL output", required = true, allowMultipleOccurrences = false))
    override fun processOption(option: AbstractCliOption, value: String, configuration: CompilerConfiguration) {
        if (option.optionName == "output") configuration.put(OUTPUT_KEY, value) else throw CliOptionProcessingException("unknown option ${option.optionName}")
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

private class FirFactsCheckersExtension(session: FirSession, output: String) : FirAdditionalCheckersExtension(session) {
    override val expressionCheckers: ExpressionCheckers = object : ExpressionCheckers() {
        override val basicExpressionCheckers: Set<FirExpressionChecker<FirStatement>> = setOf(FirFactsExpressionChecker(output))
    }
    override val declarationCheckers: DeclarationCheckers = object : DeclarationCheckers() {
        override val functionCheckers: Set<FirDeclarationChecker<FirFunction>> = setOf(FirFactsCfgChecker(output))
    }
}

private class FirFactsCfgChecker(private val output: String) : FirDeclarationChecker<FirFunction>(MppCheckerKind.Common) {
    context(context: CheckerContext, reporter: DiagnosticReporter)
    override fun check(declaration: FirFunction) {
        val source = declaration.source ?: return
        val graph = (declaration.controlFlowGraphReference as? FirControlFlowGraphReferenceImpl)?.controlFlowGraph ?: return
        val record = buildJsonObject {
            put("recordType", "FIR_CFG"); put("file", context.containingFile?.path ?: return)
            put("start", source.startOffset); put("end", source.endOffset); put("name", graph.name)
            (declaration.symbol as? FirCallableSymbol<*>)?.let { put("symbol", it.callableIdAsString()) }
            put("returnType", (declaration.returnTypeRef as? FirResolvedTypeRef)?.coneType?.toString() ?: "<unresolved>")
            putJsonArray("parameterTypes") {
                declaration.valueParameters.forEach { parameter ->
                    add((parameter.returnTypeRef as? FirResolvedTypeRef)?.coneType?.toString() ?: "<unresolved>")
                }
            }
            declaration.receiverParameter?.let { receiver ->
                put("receiverType", (receiver.typeRef as? FirResolvedTypeRef)?.coneType?.toString() ?: "<unresolved>")
            }
            putJsonArray("nodes") {
                graph.nodes.sortedBy { it.id }.forEach { node ->
                    add(buildJsonObject {
                        put("id", node.id); put("kind", node::class.simpleName ?: "CFGNode"); put("dead", node.isDead)
                        node.fir.source?.let { put("start", it.startOffset); put("end", it.endOffset) }
                    })
                }
            }
            putJsonArray("edges") {
                graph.nodes.sortedBy { it.id }.forEach { from -> from.followingNodes.sortedBy { it.id }.forEach { to ->
                    val edge = from.edgeTo(to)
                    add(buildJsonObject { put("from", from.id); put("to", to.id); put("label", edge.label.toString()); put("edgeKind", edge.kind.toString()) })
                } }
            }
        }
        appendFact(output, record)
    }
}

private class FirFactsExpressionChecker(private val output: String) : FirExpressionChecker<FirStatement>(MppCheckerKind.Common) {
    context(context: CheckerContext, reporter: DiagnosticReporter)
    override fun check(expression: FirStatement) {
        val value = expression as? FirExpression ?: return
        val source = value.source ?: return
        val access = value as? FirQualifiedAccessExpression
        val resolved = access?.calleeReference as? FirResolvedNamedReference
        val callable = resolved?.resolvedSymbol as? FirCallableSymbol<*>
        val receiver = access?.explicitReceiver ?: access?.extensionReceiver ?: access?.dispatchReceiver
        val arguments = (value as? FirFunctionCall)?.argumentList as? FirResolvedArgumentList
        val fact = buildJsonObject {
            put("recordType", "SEMANTIC_FACT"); put("file", context.containingFile?.path ?: return); put("start", source.startOffset); put("end", source.endOffset)
            put("kind", value::class.simpleName ?: "FirExpression"); put("type", value.resolvedType.toString())
            callable?.let { put("symbol", it.callableIdAsString()); put("returnType", it.resolvedReturnType.toString()) }
            putJsonArray("effects") { if ((callable?.fir as? FirFunction)?.status?.isSuspend == true) add("SUSPEND") }
            receiver?.let { put("receiverType", it.resolvedType.toString()) }
            putJsonArray("argumentToParameter") {
                arguments?.mapping?.entries?.forEach { (argument, parameter) ->
                    add(buildJsonObject { put("argumentStart", argument.source?.startOffset ?: -1); put("parameter", parameter.name.asString()); put("parameterType", (parameter.returnTypeRef as? FirResolvedTypeRef)?.coneType?.toString() ?: "<unresolved>") })
                }
            }
        }
        appendFact(output, fact)
    }
}

private fun appendFact(output: String, fact: JsonObject) = synchronized(FirFactsExpressionChecker::class.java) {
    Files.writeString(Path.of(output), fact.toString() + "\n", StandardOpenOption.CREATE, StandardOpenOption.APPEND)
}
