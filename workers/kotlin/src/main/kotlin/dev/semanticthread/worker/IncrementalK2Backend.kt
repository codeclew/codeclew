package dev.semanticthread.worker

import dev.semanticthread.worker.IncrementalK2Result
import java.nio.file.Path
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

internal const val K2_INDEX_ROOT_ENV = "CODECLEW_K2_INDEX_ROOT"

enum class IncrementalK2Status {
    UNCHANGED_HIT, COLD_FULL, INCREMENTAL, RECOVERED_FULL, BUSY, FAILED_RECOVERABLE,
}

data class IncrementalK2Request(
    val indexRoot: Path,
    val repo: Path,
    val compilation: String,
    val semanticConfigurationDigest: String,
    val expectedCompilerVersion: String,
    val moduleName: String,
    val sources: List<Path>,
    val classpath: List<Path>,
    val friendPaths: List<Path>,
    val compilerPlugins: List<Path>,
    val compilerPluginOptions: List<String>,
    val freeCompilerArguments: List<String>,
    val optIns: List<String>,
    val jdkHome: Path,
    val jvmTarget: String,
    val languageVersion: String?,
    val apiVersion: String?,
    val factsPlugin: Path,
)

data class IncrementalK2Result(
    val valid: Boolean,
    val facts: List<JsonObject>,
    val diagnostics: List<JsonObject>,
    val status: IncrementalK2Status,
    val totalMicros: Long,
    val compilerMicros: Long,
    val firExtractionMicros: Long,
    val totalFiles: Int,
    val compiledFiles: Int,
    val reusedFiles: Int,
    val recovered: Boolean,
    val graphDigest: String?,
)

interface IncrementalK2Backend {
    fun analyze(request: IncrementalK2Request): IncrementalK2Result
}

internal object IncrementalK2Runtime {
    private val profiling = ThreadLocal<JsonObject?>()
    private val backend: IncrementalK2Backend? by lazy {
        try {
            Class.forName("dev.semanticthread.worker.BtaIncrementalBackend21", true, javaClass.classLoader)
                .getDeclaredConstructor().newInstance() as IncrementalK2Backend
        } catch (_: ReflectiveOperationException) {
            null
        } catch (_: LinkageError) {
            null
        } catch (_: ClassCastException) {
            null
        } catch (_: SecurityException) {
            null
        }
    }

    fun backendOrNull(): IncrementalK2Backend? = backend
    fun reset() = profiling.remove()
    fun record(value: IncrementalK2Result, fallbackUsed: Boolean) {
            profiling.set(buildJsonObject {
                put("backend", "BTA_PERSISTENT")
                put("status", value.status.name)
                put("valid", value.valid)
                put("totalMicros", value.totalMicros)
                put("compilerMicros", value.compilerMicros)
                put("firExtractionMicros", value.firExtractionMicros)
                put("totalFiles", value.totalFiles)
                put("compiledFiles", value.compiledFiles)
                put("reusedFiles", value.reusedFiles)
                put("recovered", value.recovered)
                put("fallbackUsed", fallbackUsed)
                value.graphDigest?.let { put("graphDigest", it) }
            })
        }
    fun mergeProfiling(response: JsonObject, incremental: JsonObject?): JsonObject {
        if (incremental == null) return response
        return buildJsonObject {
            response.forEach { (key, value) -> put(key, value) }
            put("profiling", buildJsonObject {
                (response["profiling"] as? JsonObject)?.forEach { (key, value) -> put(key, value) }
                incremental.forEach { (key, value) -> put(key, value) }
            })
        }
    }

    fun takeProfiling(): JsonObject? = profiling.get().also { profiling.remove() }
}
