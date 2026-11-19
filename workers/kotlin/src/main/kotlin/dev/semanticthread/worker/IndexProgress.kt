package dev.semanticthread.worker

import java.io.IOException
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

internal enum class IndexPhase {
    RECEIVE_REQUEST, RESOLVE_INPUTS, COLLECT_COMPILER_FACTS, BUILD_INDEX,
    ATTACH_LOCAL_CFG, SERIALIZE_RESPONSE, ENCODE_RESPONSE, WRITE_RESPONSE, RESPONSE_WRITTEN,
}

/** One bounded, non-authoritative checkpoint in the controller-owned transport directory. */
internal class IndexProgress private constructor(
    private val root: Path,
    private val requestId: Long,
    private val requestBytes: Int,
) {
    var requestedFiles: Int? = null
    var compilationFiles: Int? = null
    var selectedFiles: Int? = null
    var selectedSourceBytes: Long? = null
    var compilerFacts: Int? = null
    var responseFrameBytes: Int? = null

    fun mark(phase: IndexPhase) {
        try {
            val runtime = Runtime.getRuntime()
            val progress = buildJsonObject {
                put("phase", phase.name)
                put("requestBytes", requestBytes)
                put("heapUsedBytes", (runtime.totalMemory() - runtime.freeMemory()).coerceAtLeast(0))
                put("heapLimitBytes", runtime.maxMemory())
                requestedFiles?.let { put("requestedFiles", it) }
                compilationFiles?.let { put("compilationFiles", it) }
                selectedFiles?.let { put("selectedFiles", it) }
                selectedSourceBytes?.let { put("selectedSourceBytes", it) }
                compilerFacts?.let { put("compilerFacts", it) }
                responseFrameBytes?.let { put("responseFrameBytes", it) }
            }
            val checkpoint = buildJsonObject {
                put("schema", "codeclew-worker-index-progress/1")
                put("requestId", requestId)
                put("progress", progress)
            }
            val temporary = Files.createTempFile(root, ".index-progress-", ".tmp")
            try {
                Files.writeString(temporary, checkpoint.toString())
                Files.move(temporary, root.resolve("index-progress.json"),
                    StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
            } finally {
                Files.deleteIfExists(temporary)
            }
        } catch (_: IOException) {
            // Keep the last complete checkpoint if storage is unavailable.
        } catch (_: SecurityException) {
            // Diagnostics do not change compiler admission.
        }
    }

    companion object {
        fun start(requestId: Long, requestBytes: Int): IndexProgress? {
            val path = System.getenv("CODECLEW_WORKER_TRANSPORT_ROOT")?.takeIf(String::isNotBlank) ?: return null
            val root = Path.of(path)
            if (!root.isAbsolute || Files.isSymbolicLink(root) || !Files.isDirectory(root)) return null
            return IndexProgress(root, requestId, requestBytes).also { it.mark(IndexPhase.RECEIVE_REQUEST) }
        }
    }
}
