package dev.semanticthread.worker

import org.jetbrains.kotlin.fir.resolve.dfa.cfg.EdgeLabel
import org.jetbrains.kotlin.fir.resolve.dfa.cfg.NormalPath

/** Preserve compiler jump targets without persisting JVM object identities. */
internal fun compilerCfgLabel(label: EdgeLabel): String =
    if (label === NormalPath) "NormalPath" else requireNotNull(label.label) {
        "Unsupported compiler CFG label without a semantic payload"
    }
