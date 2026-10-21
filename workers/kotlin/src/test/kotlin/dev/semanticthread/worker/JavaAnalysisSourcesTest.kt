package dev.semanticthread.worker

import java.nio.file.Files
import java.nio.file.Path
import kotlinx.serialization.json.*
import kotlin.test.*
import org.junit.jupiter.api.Assumptions.assumeTrue

class JavaAnalysisSourcesTest {
    private fun model(kotlin: Path, java: Path) = buildJsonObject {
        putJsonArray("sourceFiles") { add(kotlin.toString()) }
        putJsonArray("javaSourceFiles") { add(java.toString()) }
    }
    private fun paths(model: JsonObject, key: String) = model[key]!!.jsonArray.map { it.jsonPrimitive.content }

    @Test
    fun javaAnalysisInputsAreHashedWithoutBecomingKotlinIndexFiles() {
        val repo = Files.createTempDirectory("java-analysis-authority").toRealPath()
        try {
            val kotlin = Files.writeString(repo.resolve("Entry.kt"), "fun entry() = Owned.value()")
            val java = Files.writeString(repo.resolve("Owned.java"), "class Owned { static int value() { return 1; } }")
            val raw = model(kotlin, java)
            val selected = withJavaAnalysisSources(raw)
            assertEquals(listOf(kotlin.toString()), paths(selected, "sourceFiles"))
            assertEquals(setOf(kotlin.toString(), java.toString()), paths(selected, "analysisSourceFiles").toSet())
            val first = sourceSelectionAuthority(repo, validateProjectModelSourceFiles(repo, selected))
            Files.writeString(java, "class Owned { static String value() { return \"changed\"; } }")
            val changed = sourceSelectionAuthority(repo, selected)
            assertEquals(first["sourceFiles"], changed["sourceFiles"])
            assertNotEquals(first["analysisSourceFiles"], changed["analysisSourceFiles"])
            Files.delete(java)
            assertFailsWith<WorkerFailure> { sourceSelectionAuthority(repo, selected) }
        } finally { repo.toFile().deleteRecursively() }
    }

    @Test
    fun testCompositionIncludesMainJavaAnalysisButIndexesOnlyKotlin() {
        val repo = Files.createTempDirectory("java-test-composition").toRealPath()
        try {
            val main = Files.writeString(repo.resolve("Main.kt"), "internal fun value() = Owned.value()")
            val test = Files.writeString(repo.resolve("Test.kt"), "fun checkValue() = value()")
            val mainJava = Files.writeString(repo.resolve("Owned.java"), "class Owned {}")
            val testJava = Files.writeString(repo.resolve("TestOnly.java"), "class TestOnly {}")
            val mainModel = withJavaAnalysisSources(model(main, mainJava))
            Worker().use { worker ->
                val selected = withJavaAnalysisSources(worker.sourceComposedTestProjectModel(repo, ":/test", model(test, testJava)) { mainModel })
                assertEquals(setOf(main.toString(), test.toString()), paths(selected, "sourceFiles").toSet())
                assertEquals(setOf(main.toString(), test.toString(), mainJava.toString(), testJava.toString()), paths(selected, "analysisSourceFiles").toSet())
                assertFalse(paths(mainModel, "analysisSourceFiles").contains(testJava.toString()))
            }
        } finally { repo.toFile().deleteRecursively() }
    }
    @Test
    fun selectedJavaInputsResolveWithDefaultPackagedCompiler() {
        assumeTrue(WORKER_COMPILER_VERSION == "2.4.10", "The 2.1 engine has no qualified default project-analysis route")
        val root = Files.createTempDirectory("java-analysis-engine").toRealPath()
        try {
            val repository = generateSequence(Path.of(System.getProperty("user.dir")).toAbsolutePath(), Path::getParent)
                .first { Files.isRegularFile(it.resolve("gradlew")) }
            val fixture = repository.resolve("fixtures/kotlin-generated-java")
            Files.walk(fixture).use { files -> files.forEach { source ->
                val target = root.resolve(fixture.relativize(source))
                if (Files.isDirectory(source)) Files.createDirectories(target) else Files.copy(source, target)
            } }
            val build = root.resolve("build.gradle.kts")
            Files.writeString(build, Files.readString(build).replace("2.3.21", WORKER_COMPILER_VERSION)
                .replace("jvmToolchain(25)", "jvmToolchain(21)").replace("JvmTarget.JVM_25", "JvmTarget.JVM_21"))
            Files.writeString(root.resolve("src/main/kotlin/fixture/Entry.kt"), "package fixture\nimport fixture.generated.OwnedValue\nfun generatedLabel(): String = OwnedValue.label()\n")
            val request = buildJsonObject { put("repo", root.toString()); put("compilation", ":/main") }.toString().toByteArray()
            Worker().use { worker ->
                val indexed = Json.parseToJsonElement(worker.handle(3, request)).jsonObject
                assertEquals("K2_SEMANTIC", indexed["analysisMode"]!!.jsonPrimitive.content)
                assertEquals(listOf("src/main/kotlin/fixture/Entry.kt"), indexed["files"]!!.jsonArray.map { it.jsonObject["path"]!!.jsonPrimitive.content })
                assertFalse(Files.exists(root.resolve("build/kotlin-compiled.marker")))
            }
        } finally { root.toFile().deleteRecursively() }
    }

}
