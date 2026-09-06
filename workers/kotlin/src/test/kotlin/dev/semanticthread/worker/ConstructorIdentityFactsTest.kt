package dev.semanticthread.worker

import java.io.ByteArrayOutputStream
import java.io.PrintStream
import java.nio.file.Files
import java.nio.file.Path
import kotlinx.serialization.json.*
import org.jetbrains.kotlin.cli.jvm.K2JVMCompiler
import kotlin.test.*

class ConstructorIdentityFactsTest {
    @Test
    fun constructorsBindCompilerClassAsOwnerIncludingNestedAndCompanionContexts() {
        val root = Files.createTempDirectory("constructor-facts").toRealPath()
        try {
            val source = Files.writeString(root.resolve("Constructors.kt"), """
                package constructors
                @jakarta.persistence.Entity class Entity(val id: String)
                @org.springframework.stereotype.Component class Bean(val value: String)
                class Plain
                class WithSecondary(val value: String) { constructor(): this("default") }
                class Outer { class Nested; inner class Inner; companion object { fun create() = Outer() } }
                object Singleton
                enum class Choice { FIRST, SECOND }
                enum class CustomChoice { FIRST { override fun value() = 1 }, SECOND { override fun value() = 2 }; abstract fun value(): Int }
                class InlineDefault(val factory: () -> Any = { object : Runnable { override fun run() {} } })
                sealed class Base { class Child : Base() }
                data class Data(val value: String)
                annotation class Marker
                class Anonymous {
                    val member = object : Runnable { override fun run() {} }
                    fun factory() = object : Runnable { override fun run() {} }
                    fun local() { class Local; Local() }
                }
            """.trimIndent())
            val facts = root.resolve("facts.jsonl")
            val plugin = Path.of(FirFactsCompilerPluginRegistrar::class.java.protectionDomain.codeSource.location.toURI())
            val plan = effectiveCompilerPluginPlan(System.getProperty("codeclew.test.projectCompilerPlugins").split(java.io.File.pathSeparator).map(Path::of), "2.3.0")
            val pluginArgs = plan.plugins.map { "-Xplugin=$it" } + listOf("-P", "plugin:org.jetbrains.kotlin.allopen:preset=spring", "-P", "plugin:org.jetbrains.kotlin.noarg:preset=jpa")
            val output = ByteArrayOutputStream()
            val status = PrintStream(output).use { stream ->
                synchronized(K2JVMCompiler::class.java) {
                    K2JVMCompiler().exec(stream, "-no-stdlib", "-no-reflect", "-jvm-target", "17",
                        "-classpath", System.getProperty("java.class.path"), "-d", root.resolve("classes").toString(),
                        "-Xplugin=$plugin", "-P", "plugin:semantic-thread-facts:output=$facts", *pluginArgs.toTypedArray(), source.toString())
                }
            }
            assertEquals(0, status.code, output.toString())
            val rows = Files.readAllLines(facts).map { Json.parseToJsonElement(it).jsonObject }
                .filter { it["declarationKind"]?.jsonPrimitive?.content == "CONSTRUCTOR" }
            assertTrue(rows.size >= 10, "too few constructor facts: ${rows.size}")
            val enumEntry = rows.single { it["compilerCallableId"]?.jsonPrimitive?.content == "constructors/CustomChoice.FIRST.FIRST" }
            assertEquals("constructors/CustomChoice.FIRST", enumEntry["compilerClassId"]!!.jsonPrimitive.content)
            rows.forEach { row ->
                val expectedOwner = "class:${row["compilerClassId"]!!.jsonPrimitive.content}"
                assertEquals(expectedOwner, row["ownerIdentity"]!!.jsonPrimitive.content, row.toString())
                assertEquals(expectedOwner, row["containment"]!!.jsonArray.last().jsonPrimitive.content, row.toString())
                assertEquals("constructor:${row["compilerCallableId"]!!.jsonPrimitive.content}#jvm:${row["jvmDescriptor"]!!.jsonPrimitive.content}", row["symbolIdentity"]!!.jsonPrimitive.content)
                val partial = descriptorCorePayload(row, "sha256:" + "a".repeat(64))
                assertEquals(row["ownerIdentity"], partial["ownerIdentity"])
                assertEquals(row["containment"], partial["containment"])
            }
        } finally { root.toFile().deleteRecursively() }
    }
}
