package dev.semanticthread.worker

import java.io.ByteArrayOutputStream
import java.io.PrintStream
import java.nio.file.Files
import java.nio.file.Path
import kotlinx.serialization.json.*
import org.jetbrains.kotlin.cli.jvm.K2JVMCompiler
import kotlin.test.*

/** Real K2 2.4 compiler occurrences; no manufactured name-to-symbol bindings. */
class KotlinDocumentationVariablesTest {
    private fun compile(source: String): List<JsonObject> {
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
                .filter { it["recordType"]?.jsonPrimitive?.content in setOf("DOCUMENTATION_VARIABLE", "DOCUMENTATION_DATA_BOUNDARY") }
        } finally {
            root.toFile().deleteRecursively()
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
