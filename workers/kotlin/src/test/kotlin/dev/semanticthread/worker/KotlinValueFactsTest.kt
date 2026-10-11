package dev.semanticthread.worker

import kotlinx.serialization.json.*
import org.jetbrains.kotlin.cli.jvm.K2JVMCompiler
import org.jetbrains.kotlin.cli.jvm.compiler.EnvironmentConfigFiles
import org.jetbrains.kotlin.cli.jvm.compiler.KotlinCoreEnvironment
import org.jetbrains.kotlin.com.intellij.openapi.util.Disposer
import org.jetbrains.kotlin.com.intellij.psi.util.PsiTreeUtil
import org.jetbrains.kotlin.config.CompilerConfiguration
import org.jetbrains.kotlin.psi.KtNamedFunction
import org.jetbrains.kotlin.psi.KtPsiFactory
import java.io.ByteArrayOutputStream
import java.io.PrintStream
import java.nio.file.Files
import java.nio.file.Path
import kotlin.test.*

class KotlinValueFactsTest {
    private val fixtures = Path.of("../../fixtures/kotlin-values").toAbsolutePath().normalize()

    @Test
    fun nativeImmutableLocalsMatchPortableFixtureAndOriginalCoordinates() {
        val source = Files.readString(fixtures.resolve("Values.kt"))
        val first = extract(source)
        assertEquals(first, extract(source), "Compiler projection must be deterministic")
        val golden = fixtures.resolve("projections.json")
        assertTrue(Files.isRegularFile(golden), "Missing golden projection: $first")
        assertEquals(Json.parseToJsonElement(Files.readString(golden)), first)
        val bytes = source.toByteArray(Charsets.UTF_8)
        first.forEach { value ->
            value.jsonObject["nodes"]!!.jsonArray.forEach { node ->
                val row = node.jsonObject
                val slice = bytes.copyOfRange(row["byteStart"]!!.jsonPrimitive.int, row["byteEnd"]!!.jsonPrimitive.int).toString(Charsets.UTF_8)
                assertFalse(slice.contains('\uFFFD'), slice)
                assertTrue(slice.isNotEmpty())
            }
        }
        val choose = first.map { it.jsonObject }.single { it["owner"]!!.jsonPrimitive.content.endsWith("choose#jvm:(Ljava/lang/String;)Ljava/lang/String;") }
        val nodes = choose["nodes"]!!.jsonArray.map { it.jsonObject }
        assertEquals(1, nodes.count { it["operation"]!!.jsonObject["kind"]!!.jsonPrimitive.content == "INPUT" })
        assertEquals(2, nodes.count { it["operation"]!!.jsonObject["kind"]!!.jsonPrimitive.content == "COALESCE" })
        val parameter = nodes.single { it["operation"]!!.jsonObject["kind"]!!.jsonPrimitive.content == "INPUT" }["id"]!!
        val copiesToParameter = nodes.filter { it["operation"]!!.jsonObject["value"] == parameter }
        assertEquals(1, copiesToParameter.size, "Shadowed local must not resolve to the incoming parameter")
        val crlfSource = source.replace("\n", "\r\n")
        val crlf = extract(crlfSource)
        assertEquals(first.map { equations(it.jsonObject) }, crlf.map { equations(it.jsonObject) })
        val crlfBytes = crlfSource.toByteArray(Charsets.UTF_8)
        first.zip(crlf).forEach { (lf, windows) ->
            lf.jsonObject["nodes"]!!.jsonArray.zip(windows.jsonObject["nodes"]!!.jsonArray).forEach { (a, b) ->
                fun slice(bytes: ByteArray, row: JsonObject) = bytes.copyOfRange(row["byteStart"]!!.jsonPrimitive.int,
                    row["byteEnd"]!!.jsonPrimitive.int).toString(Charsets.UTF_8).replace("\r\n", "\n")
                assertEquals(slice(bytes, a.jsonObject), slice(crlfBytes, b.jsonObject))
            }
        }
        assertNotEquals(first.first().jsonObject["fullCompilationSourceDigest"], crlf.first().jsonObject["fullCompilationSourceDigest"])
    }

    @Test
    fun extractionBudgetKeepsAnExplicitUnknownResult() {
        val source = "package values\nfun budget(input: String?): String? {\n" +
            (0..130).joinToString("\n") { "val local$it = input" } + "\nreturn input\n}"
        val projection = extract(source).single().jsonObject
        val root = projection["nodes"]!!.jsonArray.single().jsonObject
        assertEquals("result", root["id"]!!.jsonPrimitive.content)
        assertEquals("KOTLIN_VALUE_BODY_OR_BUDGET_UNSUPPORTED", root["operation"]!!.jsonObject["boundary"]!!.jsonPrimitive.content)
    }

    @Test
    fun originalByteEvidenceBudgetCollapsesTheWholeModel() {
        val parameters = (0..80).joinToString(", ") { "input$it: String?" }
        val projection = extract("package values\nfun wide($parameters): String? = input0",
            "src/" + "longpath".repeat(100) + "/Values.kt").single().jsonObject
        val root = projection["nodes"]!!.jsonArray.single().jsonObject
        assertEquals("result", root["id"]!!.jsonPrimitive.content)
        assertEquals("KOTLIN_VALUE_BODY_OR_BUDGET_UNSUPPORTED", root["operation"]!!.jsonObject["boundary"]!!.jsonPrimitive.content)
    }

    private fun equations(p: JsonObject): List<JsonObject> = p["nodes"]!!.jsonArray.map {
        JsonObject(it.jsonObject.filterKeys { key -> key !in setOf("byteStart", "byteEnd") })
    }

    private fun extract(source: String, relative: String = "fixtures/kotlin-values/Values.kt"): JsonArray {
        val root = Files.createTempDirectory("kotlin-values").toRealPath()
        val disposable = Disposer.newDisposable("kotlin-values")
        try {
            val input = Files.writeString(root.resolve("Values.kt"), source)
            val facts = root.resolve("facts.jsonl")
            val plugin = Path.of(FirFactsCompilerPluginRegistrar::class.java.protectionDomain.codeSource.location.toURI())
            val output = ByteArrayOutputStream()
            val status = PrintStream(output).use { stream ->
                synchronized(K2JVMCompiler::class.java) {
                    K2JVMCompiler().exec(stream, "-no-stdlib", "-no-reflect", "-jvm-target", "21",
                        "-classpath", System.getProperty("java.class.path"), "-d", root.resolve("classes").toString(),
                        "-Xplugin=$plugin", "-P", "plugin:semantic-thread-facts:output=$facts", input.toString())
                }
            }
            assertEquals(0, status.code, output.toString())
            val coordinates = assertNotNull(CompilerUtf16ToUtf8ByteMap.fromCompilerInput(source))
            val raw = Files.readAllLines(facts).map { Json.parseToJsonElement(it).jsonObject }.distinct()
            val descriptors = raw.filter { it["recordType"]?.jsonPrimitive?.content == "DECLARATION_DESCRIPTOR" }.map { row ->
                val range = assertNotNull(coordinates.range(row["start"]!!.jsonPrimitive.int, row["end"]!!.jsonPrimitive.int))
                JsonObject(row + mapOf("start" to JsonPrimitive(range.first), "end" to JsonPrimitive(range.last + 1)))
            }
            val environment = KotlinCoreEnvironment.createForProduction(disposable, CompilerConfiguration(), EnvironmentConfigFiles.JVM_CONFIG_FILES)
            val file = KtPsiFactory(environment.project, markGenerated = false).createFile("Values.kt", source)
            val documentation = KotlinDocumentationSource(relative, file, source, ":/main", descriptors,
                emptyList(), raw.filter { it["recordType"]?.jsonPrimitive?.content == "PORTABLE_VALUE_PROJECTION" })
            return JsonArray(PsiTreeUtil.collectElementsOfType(file, KtNamedFunction::class.java).map { declaration ->
                assertNotNull(documentation.enrich(declaration, buildJsonObject { put("name", declaration.name) })["valueProjection"], declaration.text)
            }.sortedBy { it.jsonObject["owner"]!!.jsonPrimitive.content })
        } finally {
            Disposer.dispose(disposable)
            root.toFile().deleteRecursively()
        }
    }
}
