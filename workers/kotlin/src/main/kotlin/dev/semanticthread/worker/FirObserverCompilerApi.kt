@file:OptIn(org.jetbrains.kotlin.compiler.plugin.ExperimentalCompilerApi::class)

package dev.semanticthread.worker

import org.jetbrains.kotlin.compiler.plugin.CompilerPluginRegistrar
import org.jetbrains.kotlin.fir.FirSession
import org.jetbrains.kotlin.fir.declarations.FirNamedFunction
import org.jetbrains.kotlin.fir.declarations.FirProperty
import org.jetbrains.kotlin.fir.declarations.getKClassArgument
import org.jetbrains.kotlin.fir.expressions.FirAnnotation
import org.jetbrains.kotlin.fir.expressions.FirCollectionLiteral
import org.jetbrains.kotlin.name.Name

internal typealias ObservedFunction = FirNamedFunction
internal typealias ObservedCollectionLiteral = FirCollectionLiteral

abstract class ObservedCompilerPluginRegistrar : CompilerPluginRegistrar() {
    override val pluginId: String = FACTS_PLUGIN_ID
}

internal val FirProperty.isObservedLocal: Boolean get() = isLocal

@Suppress("UNUSED_PARAMETER")
internal fun FirAnnotation.observedKClassArgument(name: Name, session: FirSession) = getKClassArgument(name)
