package dev.semanticthread.worker

import java.io.ByteArrayOutputStream
import java.io.PrintStream
import java.nio.file.Files
import java.nio.file.Path
import kotlinx.serialization.json.*
import org.jetbrains.kotlin.cli.jvm.K2JVMCompiler
import org.jetbrains.kotlin.cli.jvm.compiler.EnvironmentConfigFiles
import org.jetbrains.kotlin.cli.jvm.compiler.KotlinCoreEnvironment
import org.jetbrains.kotlin.com.intellij.openapi.util.Disposer
import org.jetbrains.kotlin.com.intellij.psi.util.PsiTreeUtil
import org.jetbrains.kotlin.config.CompilerConfiguration
import org.jetbrains.kotlin.psi.KtNamedFunction
import org.jetbrains.kotlin.psi.KtPsiFactory
import kotlin.test.*

class KotlinDocumentationFlowTest {
    @Test
    fun compilerTargetsPreserveOverloadsBranchesAndUnicodeSources() {
        val flows = extract("""
            package docs
            // Warehouses: Zürich, 🏭
            class Importer {
                fun importStock(quantity: Int): String {
                    if (quantity < 0) return reject(quantity)
                    return save(quantity)
                }
                private fun reject(quantity: Int): String = "invalid"
                private fun save(quantity: Int): String = "saved"
                private fun save(quantity: String): String = quantity
            }
        """.trimIndent())
        val row = flows.single { it["name"]?.jsonPrimitive?.content == "importStock" }
        assertEquals("callable:docs/Importer.importStock#jvm:(I)Ljava/lang/String;", row["documentationSymbol"]!!.jsonPrimitive.content)
        val flow = row["documentation"]!!.jsonObject
        assertEquals(listOf("kotlin.Int"), flow["parameterTypes"]!!.jsonArray.map { it.jsonPrimitive.content })
        val events = flow["events"]!!.jsonArray.map { it.jsonObject }
        assertEquals(listOf("IF", "CALL", "RETURN", "END", "CALL", "RETURN"), events.map { it["kind"]!!.jsonPrimitive.content })
        assertEquals(listOf("callable:docs/Importer.reject#jvm:(I)Ljava/lang/String;", "callable:docs/Importer.save#jvm:(I)Ljava/lang/String;"), events.filter { it["kind"]!!.jsonPrimitive.content == "CALL" }.map { it["target"]?.jsonPrimitive?.content ?: "MISSING:$it" })
        assertEquals(5, events.first()["startLine"]!!.jsonPrimitive.int)
        assertTrue(flow["boundaries"]!!.jsonArray.isEmpty(), flow.toString())
        assertTrue(flows.filter { it["name"]?.jsonPrimitive?.content == "save" }.all { it["documentation"]!!.jsonObject["events"]!!.jsonArray.last().jsonObject["kind"]!!.jsonPrimitive.content == "RETURN" })
    }

    @Test
    fun nullableLoopsAndDeferredLambdasKeepTheirExecutionConditions() {
        val flows = extract("""
            package docs
            class Importer {
                fun importStock(values: List<String?>): Int {
                    var count = 0
                    for (value in values) {
                        val valid = value ?: return count
                        if (valid.isNotEmpty() && valid.length > 1) count++
                        valid?.trim()
                    }
                    val deferred = { count++ }
                    return count
                }
            }
        """.trimIndent())
        val flow = flows.single()["documentation"]!!.jsonObject
        val events = flow["events"]!!.jsonArray.map { it.jsonObject }
        assertEquals(1, events.count { it["kind"]!!.jsonPrimitive.content == "LOOP" })
        assertEquals(4, events.count { it["kind"]!!.jsonPrimitive.content == "IF" })
        assertEquals(6, events.count { it["kind"]!!.jsonPrimitive.content == "END" })
        assertTrue(flow["boundaries"]!!.jsonArray.map { it.jsonPrimitive.content }.contains("CALLBACK_INVOCATION_ORDER_AND_COUNT_NOT_ESTABLISHED"))
        assertTrue(events.any { it["kind"]!!.jsonPrimitive.content == "DEFERRED" && it["invocation"]!!.jsonPrimitive.content == "NOT_ESTABLISHED" })
    }

    @Test
    fun callbackCallsRemainConditionalAndFinallySurvivesCaughtFailures() {
        val flows = extract("""
            package docs
            class Importer {
                fun importStock() {
                    try {
                        run { store() }
                    } catch (failure: IllegalArgumentException) {
                        reject()
                    } finally {
                        finish()
                    }
                }
                private fun store() {}
                private fun reject() {}
                private fun finish() {}
            }
        """.trimIndent())
        val flow = flows.single { it["name"]!!.jsonPrimitive.content == "importStock" }["documentation"]!!.jsonObject
        val events = flow["events"]!!.jsonArray.map { it.jsonObject }
        val deferred = events.indexOfFirst { it["kind"]!!.jsonPrimitive.content == "DEFERRED" }
        val store = events.indexOfFirst { it["target"]?.jsonPrimitive?.content == "callable:docs/Importer.store#jvm:()V" }
        assertTrue(deferred > 0 && store > deferred, events.toString())
        assertEquals("TRY", events.first()["kind"]!!.jsonPrimitive.content)
        assertTrue(events.any { it["kind"]!!.jsonPrimitive.content == "ELSE" && it["condition"]!!.jsonPrimitive.content.contains("IllegalArgumentException") })
        val finally = events.indexOfFirst { it["kind"]!!.jsonPrimitive.content == "FINALLY" }
        assertTrue(finally > store)
        assertEquals("callable:docs/Importer.finish#jvm:()V", events.last()["target"]!!.jsonPrimitive.content)
        assertTrue(flow["boundaries"]!!.jsonArray.map { it.jsonPrimitive.content }.contains("EXCEPTION_TYPES_AND_DISPATCH_NOT_ESTABLISHED"))
    }

    private fun extract(source: String): List<JsonObject> {
        val root = Files.createTempDirectory("kotlin-docs-facts").toRealPath()
        val disposable = Disposer.newDisposable("kotlin-docs-test")
        try {
            val input = Files.writeString(root.resolve("Importer.kt"), source)
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
            val coordinates = assertNotNull(CompilerUtf16ToUtf8ByteMap.from(source))
            val rows = Files.readAllLines(facts).map { Json.parseToJsonElement(it).jsonObject }.mapNotNull { row ->
                val start = row["start"]?.jsonPrimitive?.intOrNull ?: return@mapNotNull null
                val end = row["end"]?.jsonPrimitive?.intOrNull ?: return@mapNotNull null
                if (row["recordType"]?.jsonPrimitive?.content == "DOCUMENTATION_CALL") return@mapNotNull row
                val range = coordinates.range(start, end) ?: return@mapNotNull null
                JsonObject(row + mapOf("start" to JsonPrimitive(range.first), "end" to JsonPrimitive(range.last + 1)))
            }.distinct()
            val environment = KotlinCoreEnvironment.createForProduction(disposable, CompilerConfiguration(), EnvironmentConfigFiles.JVM_CONFIG_FILES)
            val file = KtPsiFactory(environment.project, markGenerated = false).createFile("Importer.kt", source)
            val documentation = KotlinDocumentationSource("src/Importer.kt", file,
                rows.filter { it["recordType"]?.jsonPrimitive?.content == "DECLARATION_DESCRIPTOR" },
                rows.filter { it["recordType"]?.jsonPrimitive?.content == "DOCUMENTATION_CALL" })
            return PsiTreeUtil.collectElementsOfType(file, KtNamedFunction::class.java).map { declaration ->
                documentation.enrich(declaration, buildJsonObject { put("name", declaration.name) })
            }
        } finally {
            Disposer.dispose(disposable)
            root.toFile().deleteRecursively()
        }
    }
}
