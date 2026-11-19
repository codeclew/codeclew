@file:OptIn(org.jetbrains.kotlin.compiler.plugin.ExperimentalCompilerApi::class)

package dev.semanticthread.worker

import org.jetbrains.kotlin.compiler.plugin.CompilerPluginRegistrar
import org.jetbrains.kotlin.fir.FirSession
import org.jetbrains.kotlin.fir.declarations.FirProperty
import org.jetbrains.kotlin.fir.declarations.FirSimpleFunction
import org.jetbrains.kotlin.fir.declarations.getKClassArgument
import org.jetbrains.kotlin.fir.expressions.FirAnnotation
import org.jetbrains.kotlin.fir.expressions.FirArrayLiteral
import org.jetbrains.kotlin.name.Name

internal typealias ObservedFunction = FirSimpleFunction
internal typealias ObservedCollectionLiteral = FirArrayLiteral

abstract class ObservedCompilerPluginRegistrar : CompilerPluginRegistrar()

internal val FirProperty.isObservedLocal: Boolean get() = symbol.isLocal

internal fun FirAnnotation.observedKClassArgument(name: Name, session: FirSession) = getKClassArgument(name, session)
