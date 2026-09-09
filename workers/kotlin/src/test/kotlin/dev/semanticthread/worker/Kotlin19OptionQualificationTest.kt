package dev.semanticthread.worker

import java.io.File
import java.net.URLClassLoader
import java.nio.file.Files
import java.nio.file.Path
import java.util.concurrent.TimeUnit
import kotlin.test.*

/** Sampled compiler oracles; admission does not whitelist these project versions. */
class Kotlin19OptionQualificationTest {
    private data class CompilerCase(val version: String, val target: String = "17") {
        val outputName get() = "classes-$version-$target"
    }
    private val cases = listOf("1.9.0", "1.9.24", "1.9.25", "2.0.21", "2.4.10").map { CompilerCase(it) } +
        listOf(CompilerCase("2.4.10", "1.8"), CompilerCase("2.4.10", "21"))
    private val javaHome = Path.of(System.getProperty("java.home"))
    private val testClasspath = System.getProperty("java.class.path")

    private fun run(root: Path, command: List<String>): Pair<Int, String> {
        val log = root.resolve("command.log").toFile()
        val process = ProcessBuilder(command).redirectErrorStream(true).redirectOutput(log).start()
        if (!process.waitFor(90, TimeUnit.SECONDS)) {
            process.destroyForcibly().waitFor()
            fail("Compiler qualification timed out")
        }
        return process.exitValue() to log.readText()
    }

    private fun compile(root: Path, case: CompilerCase, source: Path, options: List<String>, dependencies: Path? = null): Pair<Int, String> {
        val version = case.version
        val compilerClasspath = if (version == "2.4.10") testClasspath else
            checkNotNull(System.getProperty("codeclew.test.optionOracle.$version"))
        return run(root, listOf(javaHome.resolve("bin/java").toString(), "-cp", compilerClasspath,
            "org.jetbrains.kotlin.cli.jvm.K2JVMCompiler", "-no-stdlib", "-no-reflect",
            "-language-version", if (version.startsWith("1.9.")) "1.9" else "2.0",
            "-api-version", if (version.startsWith("1.9.")) "1.9" else "2.0", "-jvm-target", case.target,
            "-classpath", listOfNotNull(compilerClasspath, dependencies?.toString(), Path.of(javax.annotation.Nonnull::class.java.protectionDomain.codeSource.location.toURI()).toString()).joinToString(File.pathSeparator),
            "-d", root.resolve(case.outputName).toString()) + options + source.toString())
    }

    @Test
    fun jsr305ModesPreserveDefaultQualifierDiagnosticsAcrossCompilers() {
        val root = Files.createTempDirectory("jsr305-qualification")
        try {
            val annotations = Path.of(javax.annotation.Nonnull::class.java.protectionDomain.codeSource.location.toURI())
            val javaSource = Files.writeString(root.resolve("Api.java"), """
                @javax.annotation.ParametersAreNonnullByDefault
                public class Api { public static String accept(String value) { return value; } }
            """.trimIndent())
            val deps = root.resolve("java")
            val javac = run(root, listOf(javaHome.resolve("bin/javac").toString(), "--release", "17", "-cp", annotations.toString(), "-d", deps.toString(), javaSource.toString()))
            assertEquals(0, javac.first, javac.second)
            val source = Files.writeString(root.resolve("Use.kt"), "fun use(): String = Api.accept(null)")
            for (case in cases) for (mode in listOf("strict", "warn", "ignore")) {
                val result = compile(root, case, source, listOf("-Xjsr305=$mode", "-Xjvm-default=all-compatibility"), deps)
                assertEquals(if (mode == "strict") 1 else 0, result.first, "$case/$mode: ${result.second}")
                if (mode == "strict") assertTrue(result.second.contains("Use.kt:") && result.second.contains("error:"), result.second)
                assertEquals(mode == "warn", result.second.lines().any { it.contains("Use.kt:") && it.contains("warning:") }, "$case/$mode: ${result.second}")
            }
        } finally { root.toFile().deleteRecursively() }
    }

    @Test
    fun jvmDefaultModesPreserveDispatchAndCompatibilityMethodsAcrossCompilers() {
        val root = Files.createTempDirectory("jvm-default-qualification")
        try {
            val source = Files.writeString(root.resolve("Defaults.kt"), """
                interface Parent { fun answer(): String = "parent" }
                interface Child : Parent { override fun answer(): String = super.answer() + "-child" }
                class Implementation : Child
            """.trimIndent())
            for (case in cases) for (mode in listOf("disable", "all", "all-compatibility")) {
                val classes = root.resolve(case.outputName)
                classes.toFile().deleteRecursively()
                val result = compile(root, case, source, listOf("-Xjvm-default=$mode", "-Xjsr305=strict"))
                assertEquals(0, result.first, "$case/$mode: ${result.second}")
                assertFalse(result.second.contains("not supported"), result.second)
                URLClassLoader(arrayOf(classes.toUri().toURL()), javaClass.classLoader).use { loader ->
                    val child = loader.loadClass("Child")
                    assertEquals(mode != "disable", child.getDeclaredMethod("answer").isDefault, "$case/$mode")
                    val implementation = loader.loadClass("Implementation")
                    assertEquals("parent-child", implementation.getMethod("answer").invoke(implementation.getConstructor().newInstance()))
                    assertEquals(mode != "all", Files.exists(classes.resolve("Child\$DefaultImpls.class")), "$case/$mode")
                    if (mode != "all") {
                        assertEquals("parent-child", loader.loadClass("Child\$DefaultImpls").getMethod("answer", child).invoke(null, implementation.getConstructor().newInstance()))
                    }
                }
            }
        } finally { root.toFile().deleteRecursively() }
    }
}
