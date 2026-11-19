package dev.semanticthread.worker

import java.nio.file.Files
import java.nio.file.Path
import kotlinx.serialization.SerialFormat
import kotlinx.serialization.json.*
import kotlin.io.path.*

private val observerSources = listOf(
    "CompilerCfgLabels.kt", "ConstructorDescriptorIdentity.kt", "FirFactsPlugin.kt",
    "JvmAnnotationFacts24.kt", "JvmMethodDescriptor.kt", "KotlinValueFacts24.kt",
)

internal fun prepareNativeFirObserver(directory: Path) {
    val loader = Worker::class.java.classLoader
    fun copy(resource: String, output: Path) {
        output.parent.createDirectories()
        checkNotNull(loader.getResourceAsStream(resource)).use { Files.copy(it, output) }
    }
    observerSources.forEach {
        copy("native-fir-observer/$it", directory.resolve("sources").resolve(it))
    }
    listOf("CommandLineProcessor", "CompilerPluginRegistrar").forEach {
        val resource = "META-INF/services/org.jetbrains.kotlin.compiler.plugin.$it"
        copy(resource, directory.resolve("services").resolve(resource))
    }
    val libraries = listOf(Json::class.java, SerialFormat::class.java).map {
        Path.of(it.protectionDomain.codeSource.location.toURI()).toRealPath().toString()
    }
    Files.write(directory.resolve("libraries"), libraries)
}

internal data class NativeFirObservation(val receipt: JsonObject, val facts: List<JsonObject>)

internal fun nativeFirFailure(category: String) = WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "$category: Selected native Kotlin capture failed.")

internal fun readNativeFirObservation(
    repo: Path,
    model: JsonObject,
    captureRevision: String,
    factsFile: Path,
): NativeFirObservation {
    val receipt = model.getValue("nativeFirCompilation").jsonObject
    require(receipt["revision"] == JsonPrimitive(captureRevision))
    require(receipt["compileTask"] == model["compileTask"])
    require(receipt["outcome"] == JsonPrimitive("EXECUTED"))
    require(receipt["compilerVersion"] == model["compilerVersion"] && receipt["compilerVersion"] is JsonPrimitive)
    require(receipt["gradleVersion"] == model["gradleVersion"])
    require(receipt["jvmTarget"] == model["jvmTarget"])
    verifyNativeCompilationInputs(repo, model, receipt)
    val bytes = factsFile.readBytes()
    require(receipt["factsDigest"] == JsonPrimitive(nativeHash(bytes)))
    val selected = model.getValue("sourceFiles").jsonArray.map { Path.of(it.jsonPrimitive.content) }
    val rows = parseCompilerFactLines(bytes.decodeToString().lineSequence().toList())
    val (compiler, facts) = rows.partition { it["recordType"] == JsonPrimitive("FIR_NATIVE_COMPILER") }
    require(compiler.isNotEmpty() && compiler.all { it["version"] == receipt["compilerVersion"] })
    val sealedFacts = validateDirectK2ReceiptClosure(repo, selected, selected, facts)
    val summary = buildJsonObject {
        put("schema", "codeclew-kotlin-native-fir/1")
        listOf("revision", "compileTask", "outcome", "compilerInvocationDigest", "observerArtifactDigest").forEach {
            put(it, receipt.getValue(it))
        }
        put("requestedCompiler", receipt.getValue("compilerVersion"))
        put("nativeGradleVersion", receipt.getValue("gradleVersion"))
        put("jvmTarget", receipt.getValue("jvmTarget").jsonPrimitive.content.removePrefix("JVM_"))
        put("receiptDigest", nativeHash(receipt))
        put("nativeFactsDigest", receipt.getValue("factsDigest"))
        put("selectedSourcesDigest", nativeHash(receipt.getValue("sourceDigests")))
        put("generatedJavaSelectionDigest", nativeHash(receipt.getValue("generatedJavaDigests")))
        put("orderedClasspathDigest", nativeHash(receipt.getValue("classpathDigests")))
        put("nativeClassfilesDigest", nativeHash(receipt.getValue("classDigests")))
        put("requestedCompilerArtifactsDigest", nativeHash(receipt.getValue("compilerArtifacts")))
        put("requestedPluginArtifactsDigest", nativeHash(receipt.getValue("requestedPluginArtifacts")))
        put("javaSourceProducersDigest", nativeHash(receipt.getValue("javaSourceProducers")))
    }
    return NativeFirObservation(summary, sealedFacts.sortedBy(JsonObject::toString))
}
