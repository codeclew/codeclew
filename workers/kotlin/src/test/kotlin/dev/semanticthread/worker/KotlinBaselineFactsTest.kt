package dev.semanticthread.worker

import java.io.ByteArrayOutputStream
import java.io.PrintStream
import java.nio.file.Files
import java.nio.file.Path
import kotlinx.serialization.json.*
import org.jetbrains.kotlin.cli.jvm.K2JVMCompiler
import kotlin.test.*

class KotlinBaselineFactsTest {
    @Test
    fun supportedLanguageLevelsProduceResolvedDeclarationAndCallFacts() {
        for (level in listOf("1.9", "2.0", "2.1", "2.2", "2.3", "2.4")) {
            val root = Files.createTempDirectory("kotlin-baseline-facts").toRealPath()
            try {
                val source = Files.writeString(root.resolve("Baseline.kt"), "package baseline\nfun twice(value: Int): Int = value * 2\nfun run(): Int = twice(21)\n")
                val facts = root.resolve("facts.jsonl")
                val plugin = Path.of(FirFactsCompilerPluginRegistrar::class.java.protectionDomain.codeSource.location.toURI())
                val output = ByteArrayOutputStream()
                val status = PrintStream(output).use { stream ->
                    synchronized(K2JVMCompiler::class.java) {
                        K2JVMCompiler().exec(stream, "-no-stdlib", "-no-reflect", "-jvm-target", "17",
                            "-language-version", kotlinAnalysisLanguageVersion(level)!!, "-api-version", kotlinAnalysisLanguageVersion(level)!!,
                            "-classpath", System.getProperty("java.class.path"), "-d", root.resolve("classes").toString(),
                            "-Xplugin=$plugin", "-P", "plugin:semantic-thread-facts:output=$facts", source.toString())
                    }
                }
                assertEquals(0, status.code, "language $level: $output")
                assertTrue(Files.isRegularFile(facts), "language $level did not run the FIR extractor: $output")
                val rows = Files.readAllLines(facts).map { Json.parseToJsonElement(it).jsonObject }
                assertTrue(rows.any { it["compilerCallableId"]?.jsonPrimitive?.content == "baseline/twice" }, "language $level: $rows")
                assertTrue(rows.any { it["kind"]?.jsonPrimitive?.content == "CALLS" }, "language $level: no compiler call edge")
            } finally { root.toFile().deleteRecursively() }
        }
    }
}
