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
            fun toolchainProperties(path: Path) = Files.writeString(root.resolve("gradle.properties"),
                "org.gradle.java.installations.paths=$path\norg.gradle.java.installations.auto-detect=false\norg.gradle.java.installations.auto-download=false\n")
            toolchainProperties(jdk25())
            val input = source(Files.createDirectories(root.resolve("src/main/kotlin")))
            val command = listOf(repository.resolve("gradlew").toString(), "--no-daemon", "-p", root.toString(),
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
            toolchainProperties(root.resolve("missing-jdk"))
            val unavailable = run(root, command.dropLast(1))
            assertNotEquals(0, unavailable.first, unavailable.second)
            assertFalse(unavailable.second.contains("__SEMANTIC_THREAD_MODEL__"), unavailable.second)
        } finally { root.toFile().deleteRecursively() }
    }
    @Test
    fun modelPreparesDeclaredGeneratedSourcesWithoutCompilingSelectedTask() {
        val root = Files.createTempDirectory("gradle-jvm25-generated-authority").toRealPath()
        try {
            val repository = generateSequence(Path.of(System.getProperty("user.dir")).toAbsolutePath(), Path::getParent)
                .first { Files.isRegularFile(it.resolve("gradlew")) }
            Files.writeString(root.resolve("settings.gradle.kts"), "pluginManagement { repositories { gradlePluginPortal(); mavenCentral() } }; dependencyResolutionManagement { repositories { mavenCentral() } }; rootProject.name = \"jvm25-generated\"")
            Files.writeString(root.resolve("gradle.properties"), "org.gradle.java.installations.paths=${jdk25()}\norg.gradle.java.installations.auto-detect=false\norg.gradle.java.installations.auto-download=false\n")
            Files.writeString(root.resolve("build.gradle.kts"), """
                plugins { kotlin("jvm") version "2.3.21" }
                kotlin { jvmToolchain(25); compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_25) } }
                val generated = layout.buildDirectory.dir("generated/owned")
                val generateOwned by tasks.registering {
                    outputs.dir(generated)
                    doLast {
                        val output = generated.get().file("GeneratedValue.kt").asFile
                        output.parentFile.mkdirs()
                        output.writeText("class GeneratedValue { fun answer(): Int = 42 }")
                    }
                }
                kotlin.sourceSets.getByName("main").kotlin.srcDir(generated)
                tasks.named("compileKotlin") {
                    dependsOn(generateOwned)
                    doLast { file("compiled.marker").writeText("compiled") }
                }
            """.trimIndent())
            val input = root.resolve("src/main/kotlin/Entry.kt")
            Files.createDirectories(input.parent)
            Files.writeString(input, "fun answer() = GeneratedValue().answer(); fun jdk25Api() = java.lang.classfile.ClassFile.of()")
            val wrapper = listOf(repository.resolve("gradlew").toString(), "--no-daemon", "-p", root.toString())
            val native = run(root, wrapper + "compileKotlin")
            assertEquals(0, native.first, native.second)
            val generated = root.resolve("build/generated/owned/GeneratedValue.kt")
            assertTrue(Files.isRegularFile(generated))
            Files.delete(generated)
            Files.delete(root.resolve("compiled.marker"))
            val modeled = run(root, wrapper + listOf("-I", repository.resolve("workers/kotlin/src/main/resources/semantic-thread-model.init.gradle").toString(), "semanticThreadModel"))
            assertEquals(0, modeled.first, modeled.second)
            assertTrue(Files.isRegularFile(generated), "model must execute the selected compilation's generator")
            assertFalse(Files.exists(root.resolve("compiled.marker")), "model must not execute the selected compiler task")
            val model = Json.parseToJsonElement(modeled.second.lineSequence().single { it.startsWith("__SEMANTIC_THREAD_MODEL__") }.removePrefix("__SEMANTIC_THREAD_MODEL__")).jsonObject
            assertEquals(setOf(input.toString(), generated.toString()), model["sourceFiles"]!!.jsonArray.map { it.jsonPrimitive.content }.toSet())
            assertEquals(jdk25(), Path.of(model["jdkHome"]!!.jsonPrimitive.content).toRealPath())
            assertEquals("JVM_25", model["jvmTarget"]!!.jsonPrimitive.content)
        } finally { root.toFile().deleteRecursively() }
    }


    @Test
    fun generatedJavaInputsResolveOnFirstCleanAnalysisAndChangesCannotReuseOldFacts() {
        val root = Files.createTempDirectory("gradle-generated-java-analysis").toRealPath()
        try {
            val repository = generateSequence(Path.of(System.getProperty("user.dir")).toAbsolutePath(), Path::getParent)
                .first { Files.isRegularFile(it.resolve("gradlew")) }
            val fixture = repository.resolve("fixtures/kotlin-generated-java")
            fun copy(name: String): Path {
                val target = Files.createDirectories(root.resolve(name))
                Files.walk(fixture).use { files -> files.forEach { source ->
                    val destination = target.resolve(fixture.relativize(source))
                    if (Files.isDirectory(source)) Files.createDirectories(destination)
                    else Files.copy(source, destination)
                } }
                Files.writeString(target.resolve("gradle.properties"), "org.gradle.java.installations.paths=${jdk25()}\norg.gradle.java.installations.auto-detect=false\norg.gradle.java.installations.auto-download=false\n")
                return target
            }
            val nativeRoot = copy("native")
            val analysisRoot = copy("analysis")
            val unattachedRoot = copy("unattached-generator")
            val unattachedBuild = unattachedRoot.resolve("build.gradle.kts")
            Files.writeString(unattachedBuild, Files.readString(unattachedBuild)
                .replace("java.srcDir(generateOwnedJava)", "java.srcDir(generatedJava)"))
            val native = run(nativeRoot, listOf(repository.resolve("gradlew").toString(), "--no-daemon", "--offline", "-p", nativeRoot.toString(), "compileKotlin"))
            assertEquals(0, native.first, native.second)
            assertTrue(Files.exists(nativeRoot.resolve("build/kotlin-compiled.marker")))
            assertFalse(Files.exists(analysisRoot.resolve("build")))
            val request = buildJsonObject { put("repo", analysisRoot.toString()); put("compilation", ":/main") }.toString().toByteArray()
            Worker().use { worker ->
                fun inspect() = Json.parseToJsonElement(worker.handle(2, request)).jsonObject
                fun index() = Json.parseToJsonElement(worker.handle(3, request)).jsonObject
                val first = inspect()
                val generated = analysisRoot.resolve("build/generated/sources/owned/main/fixture/generated/OwnedValue.java")
                assertTrue(Files.exists(generated))
                assertTrue(Files.exists(analysisRoot.resolve("build/generator-ran.marker")))
                assertFalse(Files.exists(analysisRoot.resolve("build/kotlin-compiled.marker")), "model extraction must not compile selected Kotlin task")
                val manifest = first["semanticInputManifest"]!!.jsonObject
                fun selected(role: String) = manifest[role]!!.jsonObject["files"]!!.jsonArray.map { it.jsonObject["path"]!!.jsonPrimitive.content }
                assertEquals(listOf("src/main/kotlin/fixture/Entry.kt"), selected("sourceFiles"))
                assertTrue(selected("analysisSourceFiles").contains("build/generated/sources/owned/main/fixture/generated/OwnedValue.java"))
                assertEquals("K2_SEMANTIC", index()["analysisMode"]!!.jsonPrimitive.content)

                Files.writeString(analysisRoot.resolve("input.txt"), "number\n")
                val changed = inspect()
                assertNotEquals(first["semanticInputManifestHash"], changed["semanticInputManifestHash"])
                assertEquals("SYNTAX_DECLARATIONS", index()["analysisMode"]!!.jsonPrimitive.content, "Java return type changed; old successful analysis must not be reused")

                Files.writeString(analysisRoot.resolve("input.txt"), "missing\n")
                inspect()
                assertFalse(Files.exists(generated))
                assertEquals("SYNTAX_DECLARATIONS", index()["analysisMode"]!!.jsonPrimitive.content, "missing Java must fail compiler verification")
            }
            Worker().use { worker ->
                val unattachedRequest = buildJsonObject { put("repo", unattachedRoot.toString()); put("compilation", ":/main") }.toString().toByteArray()
                val indexed = Json.parseToJsonElement(worker.handle(3, unattachedRequest)).jsonObject
                assertEquals("SYNTAX_DECLARATIONS", indexed["analysisMode"]!!.jsonPrimitive.content)
                assertFalse(Files.exists(unattachedRoot.resolve("build/generator-ran.marker")), "unattached generators must not be discovered by a broad scan")
                assertFalse(Files.exists(unattachedRoot.resolve("build/kotlin-compiled.marker")))
            }
        } finally { root.toFile().deleteRecursively() }
    }

}
