package dev.semanticthread.worker

import java.io.ByteArrayOutputStream
import java.io.PrintStream
import java.nio.file.Files
import java.nio.file.Path
import kotlinx.serialization.json.*
import org.jetbrains.kotlin.cli.jvm.K2JVMCompiler
import kotlin.test.*

class DescriptorNullabilityFactsTest {
    @Test
    fun resolvedAliasAndNestedTypesKeepRootNullabilityConsistent() {
        val root = Files.createTempDirectory("descriptor-nullability").toRealPath()
        try {
            val source = Files.writeString(root.resolve("Types.kt"), """
                package nullability
                typealias OptionalText = String?
                typealias OptionalAction = ((String) -> Int)?
                typealias TextList = List<String?>
                typealias Action = (String) -> Int?
                fun aliases(a: OptionalText, b: OptionalAction, c: TextList) {}
                fun nested(a: List<OptionalText?>, b: Array<out String?>?, c: Action?) {}
                fun <T> generic(a: T?, b: T & Any, c: (T?) -> T?): T? = a
                class Box(val value: OptionalText, val action: OptionalAction)
                val property: OptionalText = null
            """.trimIndent())
            val facts = root.resolve("facts.jsonl")
            val plugin = Path.of(FirFactsCompilerPluginRegistrar::class.java.protectionDomain.codeSource.location.toURI())
            val output = ByteArrayOutputStream()
            val status = PrintStream(output).use { stream -> synchronized(K2JVMCompiler::class.java) {
                K2JVMCompiler().exec(stream, "-no-stdlib", "-no-reflect", "-jvm-target", "17", "-classpath", System.getProperty("java.class.path"), "-d", root.resolve("classes").toString(), "-Xplugin=$plugin", "-P", "plugin:semantic-thread-facts:output=$facts", source.toString())
            } }
            assertEquals(0, status.code, output.toString())
            val rows = Files.readAllLines(facts).map { Json.parseToJsonElement(it).jsonObject }.filter { it["recordType"]?.jsonPrimitive?.content == "DECLARATION_DESCRIPTOR" }
            assertTrue(rows.isNotEmpty())
            fun parameters(name: String) = rows.single { it["compilerCallableId"]?.jsonPrimitive?.content == "nullability/$name" }["parameterTypes"]!!.jsonArray.map { it.jsonObject }
            assertEquals(listOf(true, true, false), parameters("aliases").map { it["nullable"]!!.jsonPrimitive.boolean })
            assertEquals(listOf(false, true, true), parameters("nested").map { it["nullable"]!!.jsonPrimitive.boolean })
            assertEquals(listOf(true, false, false), parameters("generic").map { it["nullable"]!!.jsonPrimitive.boolean })
            val functionParameter = parameters("generic")[2]
            assertEquals("(T?) -> T?", functionParameter["type"]!!.jsonPrimitive.content)
            assertFalse(functionParameter["nullable"]!!.jsonPrimitive.boolean)
        } finally { root.toFile().deleteRecursively() }
    }
}
