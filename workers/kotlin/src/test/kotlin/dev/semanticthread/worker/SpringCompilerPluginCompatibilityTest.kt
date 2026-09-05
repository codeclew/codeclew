package dev.semanticthread.worker

import java.io.ByteArrayOutputStream
import java.io.File
import java.io.PrintStream
import java.lang.reflect.Modifier
import java.net.URLClassLoader
import java.nio.file.Files
import java.nio.file.Path
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream
import kotlinx.serialization.json.*
import org.jetbrains.kotlin.cli.jvm.K2JVMCompiler
import kotlin.test.*

class SpringCompilerPluginCompatibilityTest {
    @Test
    fun annotationDefaultTargetPreservesConstructorAndFieldAnnotationsInBaseline() {
        val option = "-Xannotation-default-target=param-property"
        val semantics = KotlinProjectSemantics("2.3.0", "TEST", "2.3", "2.3", "17", emptyList(), listOf(option))
        val decision = kotlinEngineCompatibilityDecision(semantics)
        assertEquals("QUALIFIED", decision.status)
        assertFalse(decision.btaEligible)
        assertEquals("REJECTED", kotlinEngineCompatibilityDecision(semantics.copy(unstableCompilerOptions = listOf("-Xunqualified-option"))).status)
        val root = Files.createTempDirectory("annotation-target-qualification").toRealPath()
        try {
            val source = Files.writeString(root.resolve("Fixture.kt"), """
                package annotationfixture
                @Target(AnnotationTarget.FIELD, AnnotationTarget.VALUE_PARAMETER)
                @Retention(AnnotationRetention.RUNTIME)
                annotation class Marker
                class Annotated(@Marker val name: String)
            """.trimIndent())
            val classes = root.resolve("classes")
            val arguments = arrayOf("-no-stdlib", "-no-reflect", "-language-version", "2.3", "-api-version", "2.3", "-jvm-target", "17", "-classpath", System.getProperty("java.class.path"), "-d", classes.toString(), option, source.toString())
            val output = ByteArrayOutputStream()
            val status = PrintStream(output).use { stream -> synchronized(K2JVMCompiler::class.java) { K2JVMCompiler().exec(stream, *arguments) } }
            assertEquals(0, status.code, output.toString())
            URLClassLoader(arrayOf(classes.toUri().toURL()), javaClass.classLoader).use { loader ->
                val type = loader.loadClass("annotationfixture.Annotated")
                assertEquals("annotationfixture.Marker", type.getDeclaredField("name").annotations.single().annotationClass.java.name)
                assertEquals("annotationfixture.Marker", type.getDeclaredConstructor(String::class.java).parameterAnnotations.single().single().annotationClass.java.name)
            }
        } finally { root.toFile().deleteRecursively() }
    }

    @Test
    fun realMavenPluginsRebindAndPreserveSpringJpaAndCustomOptions() {
        val requested = System.getProperty("codeclew.test.projectCompilerPlugins").split(File.pathSeparator).map(Path::of)
        assertEquals(2, requested.size)
        val plan = effectiveCompilerPluginPlan(requested, "2.3.0")
        assertEquals(setOf("kotlin-allopen-compiler-plugin-embeddable-$WORKER_COMPILER_VERSION.jar", "kotlin-noarg-compiler-plugin-embeddable-$WORKER_COMPILER_VERSION.jar"), plan.plugins.map { it.fileName.toString() }.toSet())
        assertEquals(listOf("KOTLIN_ANALYSIS_ALLOPEN_PLUGIN_REBOUND_TO_ANALYZER_PATCH", "KOTLIN_ANALYSIS_NOARG_PLUGIN_REBOUND_TO_ANALYZER_PATCH"), plan.boundaries)
        val semantics = KotlinProjectSemantics("2.3.0", "TEST", "2.3", "2.3", "17", plan.plugins.map { it.fileName.toString() }, emptyList())
        assertEquals("QUALIFIED", kotlinEngineCompatibilityDecision(semantics).status)
        val root = Files.createTempDirectory("spring-plugin-qualification").toRealPath()
        try {
            val source = Files.writeString(root.resolve("Fixture.kt"), """
                package pluginfixture
                import org.springframework.web.bind.annotation.RestController
                import org.springframework.web.bind.annotation.GetMapping
                import jakarta.persistence.Entity
                @RestController class Api {
                    @GetMapping("/orders") fun orders(): String = Order("42").id
                }
                @Entity class Order(val id: String)
                annotation class NoArgMarker
                annotation class OpenMarker
                @NoArgMarker class Custom(val id: String) { val initialized = 7 }
                @OpenMarker class CustomOpen
                class Derived : CustomOpen()
            """.trimIndent())
            val facts = root.resolve("facts.jsonl")
            val outputDirectory = root.resolve("classes")
            val factsPlugin = Path.of(FirFactsCompilerPluginRegistrar::class.java.protectionDomain.codeSource.location.toURI())
            val options = listOf("plugin:org.jetbrains.kotlin.allopen:preset=spring", "plugin:org.jetbrains.kotlin.noarg:preset=jpa", "plugin:org.jetbrains.kotlin.allopen:annotation=pluginfixture.OpenMarker", "plugin:org.jetbrains.kotlin.noarg:annotation=pluginfixture.NoArgMarker", "plugin:org.jetbrains.kotlin.noarg:invokeInitializers=true")
            val arguments = mutableListOf("-no-stdlib", "-no-reflect", "-language-version", "2.3", "-api-version", "2.3", "-jvm-target", "17", "-classpath", System.getProperty("java.class.path"), "-d", outputDirectory.toString(), "-Xplugin=$factsPlugin", "-P", "plugin:semantic-thread-facts:output=$facts")
            plan.plugins.forEach { arguments += "-Xplugin=$it" }
            options.forEach { arguments += listOf("-P", it) }
            arguments += source.toString()
            val output = ByteArrayOutputStream()
            val status = PrintStream(output).use { stream -> synchronized(K2JVMCompiler::class.java) { K2JVMCompiler().exec(stream, *arguments.toTypedArray()) } }
            assertEquals(0, status.code, output.toString())
            val rows = Files.readAllLines(facts).map { Json.parseToJsonElement(it).jsonObject }
            val endpoint = rows.single { it["compilerCallableId"]?.jsonPrimitive?.content == "pluginfixture/Api.orders" }
            assertEquals("HTTP_ENDPOINT", endpoint["spring"]!!.jsonObject["entries"]!!.jsonArray.single().jsonObject["kind"]!!.jsonPrimitive.content)
            assertTrue(rows.any { it["compilerClassId"]?.jsonPrimitive?.content == "pluginfixture/Order" })
            URLClassLoader(arrayOf(outputDirectory.toUri().toURL()), javaClass.classLoader).use { loader ->
                val api = loader.loadClass("pluginfixture.Api")
                assertFalse(Modifier.isFinal(api.modifiers))
                assertFalse(Modifier.isFinal(api.getDeclaredMethod("orders").modifiers))
                assertNotNull(loader.loadClass("pluginfixture.Order").getDeclaredConstructor())
                val custom = loader.loadClass("pluginfixture.Custom")
                val value = custom.getDeclaredConstructor().newInstance()
                assertEquals(7, custom.getDeclaredMethod("getInitialized").invoke(value))
                assertFalse(Modifier.isFinal(loader.loadClass("pluginfixture.CustomOpen").modifiers))
            }
        } finally { root.toFile().deleteRecursively() }
    }

    @Test
    fun unknownPluginsAndMissingAnalyzerArtifactRemainErrors() {
        val root = Files.createTempDirectory("spring-plugin-negative")
        try {
            fun registrarJar(name: String): Path = root.resolve(name).also { path ->
                ZipOutputStream(Files.newOutputStream(path)).use { archive ->
                    archive.putNextEntry(ZipEntry("META-INF/services/org.jetbrains.kotlin.compiler.plugin.CompilerPluginRegistrar"))
                    archive.write("vendor.UnknownRegistrar".toByteArray())
                    archive.closeEntry()
                }
            }
            val unknown = registrarJar("unknown-plugin-2.3.0.jar")
            assertEquals("UNSUPPORTED_COMPILER_PLUGIN_ABI", assertFailsWith<WorkerFailure> { effectiveCompilerPluginPlan(listOf(unknown), "2.3.0") }.code)
            val known = System.getProperty("codeclew.test.projectCompilerPlugins").split(File.pathSeparator).map(Path::of)
            assertEquals("UNSUPPORTED_COMPILER_PLUGIN_ABI", assertFailsWith<WorkerFailure> { effectiveCompilerPluginPlan(known, "2.3.0", emptyList()) }.code)
        } finally { root.toFile().deleteRecursively() }
    }
}
