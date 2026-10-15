package dev.semanticthread.worker

import java.io.ByteArrayOutputStream
import java.io.File
import java.io.PrintStream
import java.nio.file.Files
import java.nio.file.Path
import java.util.concurrent.TimeUnit
import kotlinx.serialization.json.*
import org.jetbrains.kotlin.cli.jvm.K2JVMCompiler
import org.jetbrains.org.objectweb.asm.ClassReader
import org.jetbrains.org.objectweb.asm.ClassVisitor
import org.jetbrains.org.objectweb.asm.MethodVisitor
import org.jetbrains.org.objectweb.asm.Opcodes
import kotlin.test.*

/** Native compiler and packaged-analyzer oracles; project flags and target are never rewritten. */
class KotlinJvm25QualificationTest {
    private val copyFlag = "-Xconsistent-data-class-copy-visibility"
    private val previewFlag = "-Xjvm-enable-preview"
    private val cases = listOf(listOf(copyFlag), listOf(previewFlag), listOf(copyFlag, previewFlag))
    private val classpath = System.getProperty("java.class.path")
    private val jdk21 = Path.of(System.getProperty("java.home"))
    private fun jdk25(): Path = Path.of(checkNotNull(System.getenv("CODECLEW_TEST_JDK25_HOME")) {
        "JVM 25 qualification requires CODECLEW_TEST_JDK25_HOME pointing to JDK 25"
    }).toRealPath()
    private fun run(root: Path, command: List<String>): Pair<Int, String> {
        val log = root.resolve("command.log")
        val builder = ProcessBuilder(command).redirectErrorStream(true).redirectOutput(log.toFile())
        builder.environment()["JAVA_HOME"] = jdk21.toString()
        val process = builder.start()
        if (!process.waitFor(120, TimeUnit.SECONDS)) {
            process.destroyForcibly().waitFor()
            fail("JVM 25 qualification command timed out")
        }
        return process.exitValue() to Files.readString(log)
    }
    private fun compile(root: Path, source: Path, options: List<String>, home: Path = jdk25(),
                        version: String = "2.4.10", target: String = "25", plugin: Boolean = false): Pair<Int, String> {
        val command = listOf("-no-stdlib", "-no-reflect", "-jdk-home", home.toString(), "-jvm-target", target,
            "-language-version", if (version.startsWith("2.0")) "2.0" else "2.3", "-api-version", if (version.startsWith("2.0")) "2.0" else "2.3",
            "-classpath", classpath, "-d", root.resolve("classes").toString()) +
            (if (plugin) listOf("-Xplugin=${Path.of(FirFactsCompilerPluginRegistrar::class.java.protectionDomain.codeSource.location.toURI())}",
                "-P", "plugin:semantic-thread-facts:output=${root.resolve("facts.jsonl")}") else emptyList()) + options + source.toString()
        if (version != "2.4.10") return run(root, listOf(jdk21.resolve("bin/java").toString(), "-cp",
            checkNotNull(System.getProperty("codeclew.test.optionOracle.$version")), "org.jetbrains.kotlin.cli.jvm.K2JVMCompiler") + command)
        val output = ByteArrayOutputStream()
        val status = PrintStream(output).use { stream -> synchronized(K2JVMCompiler::class.java) {
            K2JVMCompiler().exec(stream, *command.toTypedArray())
        } }
        return status.code to output.toString()
    }
    private fun source(root: Path): Path = Files.writeString(root.resolve("Value.kt"), """
        data class Value private constructor(val value: Int) {
            companion object { fun create() = Value(41) }
            fun next() = copy(value = value + 1)
        }
        fun jdk25Api() = java.lang.classfile.ClassFile.of()
        fun main() { println(Value.create().next().value) }
    """.trimIndent())

    @Test
    fun bothNativeAndPackagedCompilersPreserveFlagsPrivateCopyAndPreviewClassfilesAtTarget25() {
        val root = Files.createTempDirectory("jvm25-options")
        try {
            for (version in listOf("2.3.21", "2.4.10")) for (options in cases) {
                val input = source(root)
                val facts = root.resolve("facts.jsonl")
                Files.deleteIfExists(facts)
                val result = compile(root, input, options, version = version, plugin = version == "2.4.10")
                assertEquals(0, result.first, "$version/$options: ${result.second}")
                assertFalse(result.second.contains("not supported"), result.second)
                val bytes = Files.readAllBytes(root.resolve("classes/Value.class"))
                fun u2(i: Int) = ((bytes[i].toInt() and 255) shl 8) or (bytes[i+1].toInt() and 255)
                assertEquals(69, u2(6), "$version/$options")
                assertEquals(if (previewFlag in options) 65535 else 0, u2(4), "$version/$options")
                var copyAccess: Int? = null
                ClassReader(bytes).accept(object: ClassVisitor(Opcodes.ASM9) {
                    override fun visitMethod(access: Int, name: String, descriptor: String, signature: String?, exceptions: Array<out String>?): MethodVisitor? {
                        if (name == "copy") copyAccess = access
                        return null
                    }
                }, ClassReader.SKIP_CODE)
                assertNotNull(copyAccess)
                assertEquals(copyFlag in options, copyAccess and Opcodes.ACC_PRIVATE != 0, "$version/$options")
                val runtime = listOf(jdk25().resolve("bin/java").toString())
                val args = listOf("-cp", root.resolve("classes").toString() + File.pathSeparator + classpath, "ValueKt")
                val accepted = run(root, runtime + (if (previewFlag in options) listOf("--enable-preview") else emptyList()) + args)
                assertEquals(0, accepted.first, accepted.second)
                assertEquals("42", accepted.second.trim())
                if (previewFlag in options) {
                    val rejected = run(root, runtime + args)
                    assertNotEquals(0, rejected.first)
                    assertTrue(rejected.second.contains("UnsupportedClassVersionError"), rejected.second)
                }
                if (version == "2.4.10") {
                    val records = Files.readAllLines(facts).map { Json.parseToJsonElement(it).jsonObject }
                    validateDirectK2ReceiptClosure(root, listOf(input), listOf(input), records)
                    assertTrue(records.any { it["recordType"]?.jsonPrimitive?.content == "DECLARATION_RELATION" &&
                        it["kind"]?.jsonPrimitive?.content == "REFERENCES" && it["target"]?.jsonPrimitive?.content == "/Value.copy" })
                }
                if (copyFlag in options) {
                    Files.writeString(input, Files.readString(input) + "\nfun forbidden() = Value.create().copy(value = 0)\n")
                    val denied = compile(root, input, options, version = version, plugin = version == "2.4.10")
                    assertNotEquals(0, denied.first)
                    assertTrue(denied.second.contains("copy") && denied.second.contains("private"), denied.second)
                }
            }
        } finally { root.toFile().deleteRecursively() }
    }

    @Test
    fun selectedProjectJdkEnablesNewApisWithoutUpgradingAnalyzerJvm() {
        val root = Files.createTempDirectory("jvm25-jdk-authority")
        try {
            val input = source(root)
            val wrong = compile(root, input, listOf(copyFlag, previewFlag), home = jdk21, plugin = true)
            assertNotEquals(0, wrong.first)
            assertTrue(wrong.second.contains("classfile") && wrong.second.contains("unresolved", ignoreCase = true), wrong.second)
            val correct = compile(root, input, listOf(copyFlag, previewFlag), plugin = true)
            assertEquals(0, correct.first, correct.second)
            assertNotEquals(jdkFingerprint(jdk21), jdkFingerprint(jdk25()))
        } finally { root.toFile().deleteRecursively() }
    }

    @Test
    fun copyVisibilityFlagMatchesItsFirstSupportedCompilerLine() {
        val root = Files.createTempDirectory("copy-visibility-qualification")
        try {
            val input = Files.writeString(root.resolve("Value.kt"), "data class Value private constructor(val value: Int) { companion object { fun create() = Value(1) } }; fun forbidden() = Value.create().copy(value = 0)")
            for (version in listOf("2.0.21", "2.4.10")) {
                val denied = compile(root, input, listOf(copyFlag), version = version, target = "17")
                assertNotEquals(0, denied.first)
                assertTrue(denied.second.contains("copy") && denied.second.contains("private"), denied.second)
            }
        } finally { root.toFile().deleteRecursively() }
    }

    @Test
    fun realGradleModelUsesKotlinTaskToolchain25WithDaemon21AndPreservesOptions() {
        val root = Files.createTempDirectory("gradle-jvm25-authority").toRealPath()
        try {
            val repository = generateSequence(Path.of(System.getProperty("user.dir")).toAbsolutePath(), Path::getParent)
                .first { Files.isRegularFile(it.resolve("gradlew")) }
            Files.writeString(root.resolve("settings.gradle.kts"), "pluginManagement { repositories { gradlePluginPortal(); mavenCentral() } }; dependencyResolutionManagement { repositories { mavenCentral() } }; rootProject.name = \"jvm25-qualification\"")
            Files.writeString(root.resolve("build.gradle.kts"), """
                plugins { kotlin("jvm") version "2.3.21" }
                kotlin { jvmToolchain(25); compilerOptions {
                    jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_25)
                    freeCompilerArgs.addAll("$copyFlag", "$previewFlag")
                } }
            """.trimIndent())
            val input = source(Files.createDirectories(root.resolve("src/main/kotlin")))
            val command = listOf(repository.resolve("gradlew").toString(), "--no-daemon", "-p", root.toString(),
                "-Dorg.gradle.java.installations.paths=${jdk25()}", "-Dorg.gradle.java.installations.auto-download=false",
                "-I", repository.resolve("workers/kotlin/src/main/resources/semantic-thread-model.init.gradle").toString(),
                "semanticThreadModel", "compileKotlin")
            val native = run(root, command)
            assertEquals(0, native.first, native.second)
            val model = Json.parseToJsonElement(native.second.lineSequence().single { it.startsWith("__SEMANTIC_THREAD_MODEL__") }
                .removePrefix("__SEMANTIC_THREAD_MODEL__")).jsonObject
            assertEquals(jdk25(), Path.of(model["jdkHome"]!!.jsonPrimitive.content).toRealPath())
            assertEquals("AVAILABLE_KOTLIN_COMPILE_TOOLCHAIN", model["fieldBoundaries"]!!.jsonObject["jdkHome"]!!.jsonPrimitive.content)
            assertEquals("JVM_25", model["jvmTarget"]!!.jsonPrimitive.content)
            val options = model["freeCompilerArguments"]!!.jsonArray.map { it.jsonPrimitive.content }
            assertEquals(listOf(copyFlag, previewFlag), options)
            assertFalse(model["buildModelBoundaries"]!!.jsonArray.any { it.jsonPrimitive.content == "GRADLE_COMPILE_TOOLCHAIN_UNAVAILABLE_USING_BUILD_JVM" })
            val result = compile(root, input, options, home = Path.of(model["jdkHome"]!!.jsonPrimitive.content), plugin = true)
            assertEquals(0, result.first, result.second)
            val records = Files.readAllLines(root.resolve("facts.jsonl")).map { Json.parseToJsonElement(it).jsonObject }
            validateDirectK2ReceiptClosure(root, listOf(input), listOf(input), records)
            assertTrue(records.any { it["recordType"]?.jsonPrimitive?.content == "DOCUMENTATION_CALL" &&
                it["target"]?.jsonPrimitive?.content == "callable:java/lang/classfile/ClassFile.of#jvm:()Ljava/lang/classfile/ClassFile;" })
        } finally { root.toFile().deleteRecursively() }
    }
}
