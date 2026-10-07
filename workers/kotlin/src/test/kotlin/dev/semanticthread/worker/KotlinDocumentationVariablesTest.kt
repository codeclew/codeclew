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

/** Real K2 2.4 compiler occurrences; no manufactured name-to-symbol bindings. */
class KotlinDocumentationVariablesTest {
    private fun compile(source: String): List<JsonObject> = compileAll(source).filter {
        it["recordType"]?.jsonPrimitive?.content in setOf("DOCUMENTATION_VARIABLE", "DOCUMENTATION_DATA_BOUNDARY")
    }
    private fun compileAll(source: String): List<JsonObject> {
        val root = Files.createTempDirectory("kotlin-documentation-variables").toRealPath()
        try {
            val input = Files.writeString(root.resolve("Variables.kt"), source)
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
            return Files.readAllLines(facts).map { Json.parseToJsonElement(it).jsonObject }
        } finally {
            root.toFile().deleteRecursively()
        }
    }

    private fun extractData(source: String): Map<String, JsonObject> {
        val rows = compileAll(source)
        val coordinates = assertNotNull(CompilerUtf16ToUtf8ByteMap.fromCompilerInput(source))
        val descriptors = rows.filter { it["recordType"]?.jsonPrimitive?.content == "DECLARATION_DESCRIPTOR" }.map { row ->
            val range = assertNotNull(coordinates.range(row["start"]!!.jsonPrimitive.int, row["end"]!!.jsonPrimitive.int))
            JsonObject(row + mapOf("start" to JsonPrimitive(range.first), "end" to JsonPrimitive(range.last + 1)))
        }
        val disposable = Disposer.newDisposable("kotlin-documentation-data-test")
        try {
            val environment = KotlinCoreEnvironment.createForProduction(disposable, CompilerConfiguration(), EnvironmentConfigFiles.JVM_CONFIG_FILES)
            val file = KtPsiFactory(environment.project, markGenerated = false).createFile("Variables.kt", source)
            val documentation = KotlinDocumentationSource("src/Variables.kt", file, source, ":/main", descriptors,
                rows.filter { it["recordType"]?.jsonPrimitive?.content == "DOCUMENTATION_CALL" },
                rows.filter { it["recordType"]?.jsonPrimitive?.content in setOf("DOCUMENTATION_VARIABLE", "DOCUMENTATION_VARIABLE_RECEIPT", "DOCUMENTATION_DATA_BOUNDARY") })
            return PsiTreeUtil.collectElementsOfType(file, KtNamedFunction::class.java).associate { function ->
                val enriched = documentation.enrich(function, buildJsonObject { put("name", function.name) })
                function.name!! to assertNotNull(enriched["documentation"]?.jsonObject?.get("dataInput"), enriched.toString()).jsonObject
            }
        } finally { Disposer.dispose(disposable) }
    }

    @Test
    fun normalizedInputPreservesExactByteSyntaxAndCompilerNamedDefaultArgumentSlots() {
        val source = "\uFEFF" + """
            package docs
            // π🙂 before exact spans
            fun helper(input: String): String { val chosen = input; return chosen }
            fun format(input: String, flag: Boolean): String {
                var chosen = input
                if (flag) { chosen = "π🙂" } else { chosen = input }
                val transformed = helper(chosen)
                return transformed
            }
            fun pair(first: String, second: String = "default"): String { val copy = first; return copy + second }
            fun named(input: String): String = pair(second = input, first = "prefix")
            fun omitted(input: String): String = pair(first = input)
            fun constant(): String = "literal"
        """.trimIndent().replace("\n", "\r\n") + "\r\n"
        val data = extractData(source)
        assertEquals(setOf("helper", "format", "pair", "named", "omitted", "constant"), data.keys)
        val bytes = source.toByteArray(Charsets.UTF_8)
        for ((_, input) in data) {
            assertEquals("codeclew-kotlin-documentation-data/1.0", input["schema"]!!.jsonPrimitive.content)
            assertEquals("ORIGINAL_UTF8_BYTES", input["coordinateDomain"]!!.jsonPrimitive.content)
            val nodes = input["nodes"]!!.jsonArray
            assertTrue(nodes.isNotEmpty())
            assertTrue(input["body"]!!.jsonPrimitive.int in nodes.indices)
            for (node in nodes.map { it.jsonObject }) {
                val start = node["byteStart"]!!.jsonPrimitive.int; val end = node["byteEnd"]!!.jsonPrimitive.int
                assertTrue(start >= input["ownerByteStart"]!!.jsonPrimitive.int && end <= input["ownerByteEnd"]!!.jsonPrimitive.int && start < end)
                assertTrue(bytes.copyOfRange(start, end).decodeToString().isNotEmpty())
                assertTrue(node["children"]!!.jsonArray.all { it.jsonPrimitive.int in nodes.indices })
                if (node["kind"]!!.jsonPrimitive.content == "LOCAL") {
                    assertTrue(input["variables"]!!.jsonArray.map { it.jsonObject }.any {
                        it["kind"]!!.jsonPrimitive.content == "VARIABLE_DECLARATION" && it["byteStart"] == node["byteStart"] && it["byteEnd"] == node["byteEnd"]
                    }, "local syntax requires the exact FIR declaration span")
                }
            }
        }
        val formatKinds = data["format"]!!["nodes"]!!.jsonArray.map { it.jsonObject["kind"]!!.jsonPrimitive.content }
        assertTrue(setOf("LOCAL", "IF", "ASSIGNMENT", "CALL", "RETURN").all(formatKinds::contains), formatKinds.toString())
        val named = data["named"]!!["nodes"]!!.jsonArray.map { it.jsonObject }.single { it["kind"]!!.jsonPrimitive.content == "CALL" }
        assertEquals(listOf(1, 0), named["actuals"]!!.jsonArray.map { it.jsonObject["formalSlot"]!!.jsonPrimitive.int })
        val namedArguments = named["actuals"]!!.jsonArray.map { it.jsonObject["expression"]!!.jsonPrimitive.int }
            .map { data["named"]!!["nodes"]!!.jsonArray[it].jsonObject }
            .map { bytes.copyOfRange(it["byteStart"]!!.jsonPrimitive.int, it["byteEnd"]!!.jsonPrimitive.int).decodeToString() }
        assertEquals(listOf("input", "\"prefix\""), namedArguments)
        val omitted = data["omitted"]!!["nodes"]!!.jsonArray.map { it.jsonObject }.single { it["kind"]!!.jsonPrimitive.content == "CALL" }
        assertEquals(listOf(1), omitted["defaultArguments"]!!.jsonArray.map { it.jsonPrimitive.int })
        assertTrue(data["constant"]!!["variables"]!!.jsonArray.isEmpty(), "a compiler receipt qualifies even an empty variable set")
    }

    @Test
    fun normalizedInputDoesNotTreatCustomOperatorsPropertiesOrExternalCallsAsPureTransfers() {
        val source = """
            package docs
            class Box(val value: String) { operator fun plus(other: Box): Box = this }
            fun operator(value: Box): String { val result = value + value; return "later" }
            fun property(value: Box): String { val result = value.value; return "later" }
            fun external(value: String): String { val result = value.trim(); return "later" }
            fun trigger(): Boolean = true
            val enabled: Boolean get() = trigger()
            operator fun Boolean?.not(): Boolean = trigger()
            fun getter(): String { if (enabled) {}; return "later" }
            fun nullable(flag: Boolean?): String { if (!flag) {}; return "later" }
            fun variable(vararg value: String): String = "result"
            fun zero(): String = variable()
        """.trimIndent()
        // Class methods using `this` are outside this bounded pure-expression slice.
        val data = extractData(source)
        for (name in listOf("operator", "property", "external", "getter", "nullable", "zero")) {
            assertTrue(data[name]!!["nodes"]!!.jsonArray.map { it.jsonObject }.any { it["kind"]!!.jsonPrimitive.content == "UNSUPPORTED" }, name)
        }
        val bytes = source.toByteArray(Charsets.UTF_8)
        for ((name, text) in listOf("getter" to "enabled", "nullable" to "!flag", "zero" to "variable()")) {
            val matched = data[name]!!["nodes"]!!.jsonArray.map { it.jsonObject }.filter {
                it["kind"]!!.jsonPrimitive.content != "RETURN" && bytes.copyOfRange(it["byteStart"]!!.jsonPrimitive.int, it["byteEnd"]!!.jsonPrimitive.int).decodeToString() == text
            }
            assertEquals(1, matched.size, "$name must retain its exact unsupported expression")
            assertEquals("UNSUPPORTED", matched.single()["kind"]!!.jsonPrimitive.content)
        }
    }

    @Test
    fun parametersAndShadowedLocalsBindExactCompilerSymbolsAcrossOriginalByteCoordinates() {
        val source = "\uFEFF" + """
            package docs
            // π🙂 before source occurrences
            fun select(input: String, flag: Boolean = false): String {
                var chosen = input
                if (flag) { val input = "inner π🙂"; chosen = input }
                else { val input = "other"; chosen = input }
                return chosen
            }
        """.trimIndent().replace("\n", "\r\n") + "\r\n"
        val rows = compile(source)
        val variables = rows.filter { it["recordType"]!!.jsonPrimitive.content == "DOCUMENTATION_VARIABLE" }
        assertTrue(variables.isNotEmpty())
        val declarations = variables.filter { it["kind"]!!.jsonPrimitive.content == "VARIABLE_DECLARATION" }
        val sameNames = declarations.filter { it["name"]!!.jsonPrimitive.content == "input" }
        assertEquals(3, sameNames.size, declarations.toString())
        assertEquals(3, sameNames.map { it["variableIdentity"]!!.jsonPrimitive.content }.toSet().size)
        val owner = "callable:docs/select#jvm:(Ljava/lang/String;Z)Ljava/lang/String;"
        val coordinates = assertNotNull(CompilerUtf16ToUtf8ByteMap.fromCompilerInput(source))
        val bytes = source.toByteArray(Charsets.UTF_8)
        fun text(row: JsonObject, start: String = "start", end: String = "end"): String {
            val range = assertNotNull(coordinates.range(row[start]!!.jsonPrimitive.int, row[end]!!.jsonPrimitive.int))
            return bytes.copyOfRange(range.first, range.last + 1).decodeToString()
        }
        val byIdentity = declarations.associateBy { it["variableIdentity"]!!.jsonPrimitive.content }
        for (row in variables) {
            assertEquals(owner, row["ownerSymbolIdentity"]!!.jsonPrimitive.content)
            assertEquals("COMPILER_UTF16_OFFSETS", row["coordinateDomain"]!!.jsonPrimitive.content)
            assertEquals("COMPILER_EXACT", row["resolution"]!!.jsonPrimitive.content)
            assertEquals("K2_RESOLVED_VARIABLE_SYMBOL", row["authority"]!!.jsonPrimitive.content)
            val declaration = assertNotNull(byIdentity[row["variableIdentity"]!!.jsonPrimitive.content])
            assertEquals(declaration["start"], row["declarationStart"])
            assertEquals(declaration["end"], row["declarationEnd"])
            assertEquals(text(declaration), text(row, "declarationStart", "declarationEnd"))
            if (row["kind"]!!.jsonPrimitive.content == "VARIABLE_ACCESS") {
                assertEquals(row["name"]!!.jsonPrimitive.content, text(row))
            }
        }
        val parameter = sameNames.single { it["variableKind"]!!.jsonPrimitive.content == "PARAMETER" }
        assertEquals(0, parameter["parameterIndex"]!!.jsonPrimitive.int)
        assertFalse(parameter["hasDefault"]!!.jsonPrimitive.boolean)
        val flag = declarations.single { it["name"]!!.jsonPrimitive.content == "flag" }
        assertEquals(1, flag["parameterIndex"]!!.jsonPrimitive.int)
        assertTrue(flag["hasDefault"]!!.jsonPrimitive.boolean)
        val accesses = variables.filter { it["kind"]!!.jsonPrimitive.content == "VARIABLE_ACCESS" }
        val inputReads = accesses.filter { it["name"]!!.jsonPrimitive.content == "input" }
        assertEquals(sameNames.map { it["variableIdentity"] }.toSet(), inputReads.map { it["variableIdentity"] }.toSet())
        val chosen = declarations.single { it["name"]!!.jsonPrimitive.content == "chosen" }
        val chosenAccesses = accesses.filter { it["variableIdentity"] == chosen["variableIdentity"] }
        assertEquals(2, chosenAccesses.count { it["accessMode"]!!.jsonPrimitive.content == "WRITE" })
        assertEquals(1, chosenAccesses.count { it["accessMode"]!!.jsonPrimitive.content == "READ" })
        assertTrue(rows.none { it["recordType"]!!.jsonPrimitive.content == "DOCUMENTATION_DATA_BOUNDARY" }, rows.toString())
        // Compare complete retained compiler records after removing only the temp path.
        assertEquals(rows.map { JsonObject(it - "file") }, compile(source).map { JsonObject(it - "file") })
    }

    @Test
    fun fieldsDelegatedPropertiesAndDeferredBodiesKeepExplicitBoundaries() {
        val source = """
            package docs
            class Box(val value: String)
            fun opaque(box: Box, input: String): String {
                val property = box.value
                val delegated by lazy { input }
                val deferred = { input }
                fun nested(): String = input
                return input
            }
        """.trimIndent()
        val rows = compile(source)
        val owner = "callable:docs/opaque#jvm:(Ldocs/Box;Ljava/lang/String;)Ljava/lang/String;"
        val ownerRows = rows.filter { it["ownerSymbolIdentity"]?.jsonPrimitive?.content == owner }
        val variables = ownerRows.filter { it["recordType"]!!.jsonPrimitive.content == "DOCUMENTATION_VARIABLE" }
        assertTrue(variables.isNotEmpty())
        assertTrue(variables.none { it["variableKind"]!!.jsonPrimitive.content == "FIELD" })
        assertTrue(variables.none { it["name"]!!.jsonPrimitive.content in setOf("value", "delegated") })
        val inputReads = variables.filter { it["name"]!!.jsonPrimitive.content == "input" && it["kind"]!!.jsonPrimitive.content == "VARIABLE_ACCESS" }
        assertEquals(1, inputReads.size, "deferred/local-function captures do not become immediate reads: $variables")
        val codes = ownerRows.filter { it["recordType"]!!.jsonPrimitive.content == "DOCUMENTATION_DATA_BOUNDARY" }
            .map { it["code"]!!.jsonPrimitive.content }.toSet()
        assertTrue("DOCUMENTATION_PROPERTY_STORAGE_OPAQUE" in codes, codes.toString())
        assertTrue("DOCUMENTATION_NESTED_BODY_DEFERRED" in codes, codes.toString())
    }
}
