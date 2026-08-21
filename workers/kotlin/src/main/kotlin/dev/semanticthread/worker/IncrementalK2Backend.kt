package dev.semanticthread.worker

import dev.semanticthread.worker.IncrementalK2Result
import dev.semanticthread.worker.IncrementalK2Runtime
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
    private fun recordFields(values: JsonObject) {
        val previous = profiling.get()
        profiling.set(buildJsonObject {
            previous?.forEach { (key, value) -> put(key, value) }
            values.forEach { (key, value) -> put(key, value) }
        })
    }

    fun record(value: IncrementalK2Result, fallbackUsed: Boolean) {
        recordFields(buildJsonObject {
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
            value.diagnostics.asSequence()
                .mapNotNull { row ->
                    (row["code"] as? kotlinx.serialization.json.JsonPrimitive)
                        ?.takeIf { it.isString }
                        ?.content
                }
                .firstOrNull { Regex("[A-Z][A-Z0-9_]{0,95}").matches(it) }
                ?.let { put("failureCode", it) }
            value.graphDigest?.let { put("graphDigest", it) }
        })
    }

    fun recordConfigurationEvidence(
        semanticInputManifestDigest: String,
        factsPluginDigest: String,
        extractorAuthorityDigest: String,
        semanticConfigurationDigest: String,
    ) {
        val values = listOf(
            semanticInputManifestDigest,
            factsPluginDigest,
            extractorAuthorityDigest,
            semanticConfigurationDigest,
        )
        require(values.all { Regex("^sha256:[0-9a-f]{64}$").matches(it) })
        recordFields(buildJsonObject {
            put("semanticInputManifestDigest", semanticInputManifestDigest)
            put("factsPluginDigest", factsPluginDigest)
            put("extractorAuthorityDigest", extractorAuthorityDigest)
            put("semanticConfigurationDigest", semanticConfigurationDigest)
        })
    }

    fun recordProjectModel(
        status: String,
        totalMicros: Long,
        keyMicros: Long,
        loadMicros: Long,
        extractionMicros: Long,
        publishMicros: Long,
        persistentConfigured: Boolean,
        published: Boolean,
        publishOutcome: String = "NOT_ATTEMPTED",
        publishInvalidReason: String = "NOT_APPLICABLE",
    ) {
        if (profiling.get()?.containsKey("projectModelCacheStatus") == true) return
        recordFields(buildJsonObject {
            put("projectModelCacheStatus", status)
            put("projectModelTotalMicros", totalMicros)
            put("projectModelKeyMicros", keyMicros)
            put("projectModelLoadMicros", loadMicros)
            put("projectModelExtractionMicros", extractionMicros)
            put("projectModelPublishMicros", publishMicros)
            put("projectModelPersistentConfigured", persistentConfigured)
put("projectModelPublished", published)
put("projectModelPublishOutcome", publishOutcome)
put("projectModelPublishInvalidReason", publishInvalidReason)
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
