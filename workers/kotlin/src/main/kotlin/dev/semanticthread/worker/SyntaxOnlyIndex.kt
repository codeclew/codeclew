package dev.semanticthread.worker

import dev.semanticthread.worker.syntaxOnlyIndexSourceFiles
import java.nio.file.Files
import java.nio.file.Path
import kotlin.io.path.extension
import kotlin.io.path.invariantSeparatorsPathString

internal fun syntaxOnlyIndexSourceFiles(repo: Path, requestedFiles: List<String> = emptyList()): List<Path> {
    val canonicalRepo = repo.toRealPath()
    val seen = mutableSetOf<String>()
    return requestedFiles.map { requested ->
        fun reject(): Nothing = throw IllegalArgumentException("invalid syntax-only index file: $requested")
        if (requested.isBlank() || requested.contains('\\')) reject()
        val relative = Path.of(requested)
        val normalized = relative.normalize()
        val canonicalRelative = normalized.invariantSeparatorsPathString
        if (
            relative.isAbsolute || normalized != relative || canonicalRelative != requested ||
            normalized.any { it.toString() == "." || it.toString() == ".." } ||
            relative.extension != "kt" || !seen.add(canonicalRelative)
        ) reject()
        val candidate = canonicalRepo.resolve(normalized).normalize()
        if (!candidate.startsWith(canonicalRepo)) reject()
        var current = canonicalRepo
        for (segment in normalized) {
            current = current.resolve(segment)
            if (Files.isSymbolicLink(current)) reject()
        }
        if (!Files.isRegularFile(candidate, java.nio.file.LinkOption.NOFOLLOW_LINKS)) reject()
        candidate.toRealPath(java.nio.file.LinkOption.NOFOLLOW_LINKS).also { resolved ->
            if (!resolved.startsWith(canonicalRepo)) reject()
        }
    }.sorted()
}
