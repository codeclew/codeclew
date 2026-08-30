package dev.semanticthread.worker

import dev.semanticthread.worker.gradleModelCommand
import dev.semanticthread.worker.sanitizedProjectModelProcess
import java.nio.file.Files
import java.security.MessageDigest
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream
import kotlin.io.path.createDirectories
import kotlin.io.path.createFile
import kotlin.io.path.exists
import kotlin.io.path.isRegularFile
import kotlin.io.path.writeText
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertNotEquals
import kotlin.test.assertTrue
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlinx.serialization.json.putJsonObject

class ProjectModelCommandTest {
    private fun fixtureSha(bytes: ByteArray): String = "sha256:" +
        MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

    private fun compilerPluginJar(path: java.nio.file.Path, registrar: String) {
        ZipOutputStream(Files.newOutputStream(path)).use { archive ->
            archive.putNextEntry(ZipEntry("META-INF/services/org.jetbrains.kotlin.compiler.plugin.CompilerPluginRegistrar"))
            archive.write(registrar.toByteArray())
            archive.closeEntry()
        }
    }

    private fun gradleFixtureModel(
        repo: java.nio.file.Path,
        source: java.nio.file.Path,
        jvmTarget: String,
        classpath: List<String> = emptyList(),
    ): JsonObject = buildJsonObject {
        put("projectPath", ":")
        put("projectDir", repo.toString())
        put("platform", "JVM")
        put("compileTask", ":compileKotlin")
        putJsonArray("sourceFiles") { add(JsonPrimitive(source.toString())) }
        putJsonArray("analysisSourceFiles") { add(JsonPrimitive(source.toString())) }
        putJsonArray("classpath") { classpath.forEach { add(JsonPrimitive(it)) } }
        putJsonObject("classpathAuthority") {
            put("chosen", "KOTLIN_TASK_LIBRARIES")
            put("orderedDigest", fixtureSha(classpath.joinToString("\u0000", postfix = "\u0000").toByteArray()))
        }
        putJsonArray("friendPaths") {}
        putJsonArray("compilerPlugins") {}
        putJsonArray("compilerPluginOptions") {}
        putJsonArray("dependencyCoordinates") {}
        putJsonArray("repositories") {}
        putJsonArray("reactorPoms") {}
        putJsonArray("buildPlugins") {}
        putJsonObject("generatedSourceConfiguration") {
            putJsonArray("roots") {}
            putJsonArray("producers") {}
            put("status", "NONE_DISCOVERED")
        }
        put("compilerVersion", WORKER_COMPILER_VERSION)
        put("languageVersion", "2.4")
        put("apiVersion", "2.4")
        put("jvmTarget", jvmTarget)
        putJsonArray("freeCompilerArguments") {}
        putJsonArray("optIns") {}
        putJsonArray("tasks") { add(JsonPrimitive("test")) }
        putJsonArray("buildModelBoundaries") {}
        putJsonObject("fieldBoundaries") {
            put("libraries", "AVAILABLE_ORDERED")
            put("friendPaths", "AVAILABLE_ORDERED")
            put("compilerPlugins", "AVAILABLE_ORDERED")
            put("freeCompilerArguments", "AVAILABLE_ORDERED")
            put("optIns", "AVAILABLE_ORDERED")
            put("jdkHome", "AVAILABLE_BUILD_JVM")
        }
        put("gradleVersion", "fixture")
        put("jdkHome", System.getProperty("java.home"))
    }

    @Test
    fun compilerPluginPlanRebindsKotlinPluginsToAnalyzerAbi() {
        val root = Files.createTempDirectory("worker-plugin-plan")
        try {
            val serializationRequested = root.resolve("kotlin-serialization-compiler-plugin-embeddable-2.1.10.jar")
            val serializationEffective = root.resolve("kotlin-serialization-compiler-plugin-embeddable-$WORKER_COMPILER_VERSION.jar")
            val scripting = root.resolve("kotlin-scripting-compiler-embeddable-2.1.10.jar")
            val support = root.resolve("kotlin-stdlib-2.1.10.jar").createFile()
            compilerPluginJar(serializationRequested, "old.SerializationRegistrar")
            compilerPluginJar(serializationEffective, "current.SerializationRegistrar")
            compilerPluginJar(scripting, "old.ScriptingRegistrar")

            val plan = effectiveCompilerPluginPlan(
                listOf(serializationRequested, scripting, support),
                "2.1.10",
                listOf(serializationEffective),
            )
            assertEquals(listOf(serializationEffective.toAbsolutePath().normalize()), plan.plugins)
            assertEquals(
                listOf(
                    "KOTLIN_SCRIPTING_PLUGIN_OMITTED_FOR_KT_ANALYSIS",
                    "KOTLIN_SERIALIZATION_PLUGIN_REBOUND_TO_ANALYZER_PATCH",
                ),
                plan.boundaries,
            )

            val unknown = root.resolve("third-party-compiler-plugin.jar")
            compilerPluginJar(unknown, "vendor.PluginRegistrar")
            val failure = assertFailsWith<WorkerFailure> {
                effectiveCompilerPluginPlan(listOf(unknown), "2.1.10", listOf(serializationEffective))
            }
            assertEquals("UNSUPPORTED_COMPILER_PLUGIN_ABI", failure.code)
        } finally {
            root.toFile().deleteRecursively()
        }
    }

    @Test
    fun gradlePlanUsesProjectNativeEnvironmentByDefaultAndKeepsExternalModeOffline() {
        val repo = Files.createTempDirectory("worker-gradle-plan").toRealPath()
        val externalRoot = Files.createTempDirectory("worker-gradle-external-plan").toRealPath()
        try {
            val wrapper = repo.resolve("gradlew").createFile()
            val init = repo.resolve("model.init.gradle").createFile()
            val home = repo.resolve(".gradle")
            val command = gradleModelCommand(
                wrapper,
                repo,
                home,
                home,
                init,
                "compileKotlin",
                ":semanticThreadModel",
            )
            assertEquals(1, command.count { it == "--no-daemon" })
            val warmCommand = gradleModelCommand(
                wrapper,
                repo,
                home,
                home,
                init,
                "compileKotlin",
                ":semanticThreadModel",
                reuseDaemon = true,
            )
            assertFalse(warmCommand.contains("--no-daemon"))
            assertEquals(command.filterNot { it == "--no-daemon" }, warmCommand)
            assertFalse(command.contains("--offline"))
            assertFalse(command.contains("--gradle-user-home"))
            assertFalse(command.contains("--project-cache-dir"))
            assertFalse(command.any { it.startsWith("-Duser.home=") })
            assertFalse(Files.exists(home))
            assertFalse(Files.exists(repo.resolve(".semantic-thread")))
            val internalKeys = listOf("CODECLEW_K1_BUILD_STATE_ROOT", "CODECLEW_K2_INDEX_ROOT")
            val buildKeys = listOf(
                "GRADLE_OPTS", "GRADLE_USER_HOME", "MAVEN_OPTS", "MAVEN_ARGS", "MAVEN_CONFIG",
                "MAVEN_USER_HOME", "JAVA_OPTS", "JAVA_TOOL_OPTIONS", "JDK_JAVA_OPTIONS",
                "_JAVA_OPTIONS",
            )
            val seededEnvironment =
                (internalKeys + buildKeys).associateWithTo(linkedMapOf<String, String>()) { "/configured/$it" }
            seededEnvironment["CODECLEW_TEST_UNRELATED"] = "preserved"
            val process = sanitizedProjectModelProcess(
                command,
                repo,
                seededEnvironment = seededEnvironment,
            )
            for (key in internalKeys) {
                assertFalse(process.environment().containsKey(key))
            }
            for (key in buildKeys) {
                assertEquals("/configured/$key", process.environment()[key])
            }
            assertEquals("preserved", process.environment()["CODECLEW_TEST_UNRELATED"])

            prepareFixtureBuildState(externalRoot)
            val external = externalBuildStateLayout(repo, externalRoot)
            val externalCommand = gradleModelCommand(
                wrapper,
                repo,
                external.gradleUserHome,
                external.gradleUserHome.resolve("project-cache"),
                init,
                "compileKotlin",
                ":semanticThreadModel",
                preparedState = external,
            )
            assertEquals(1, externalCommand.count { it == "--offline" })
            assertEquals(1, externalCommand.count { it == "--gradle-user-home" })
            assertEquals(1, externalCommand.count { it == "--project-cache-dir" })
            assertTrue(externalCommand.any { it.startsWith("-Duser.home=") })
        } finally {
            repo.toFile().deleteRecursively()
            externalRoot.toFile().deleteRecursively()
        }
    }

    @Test
    fun projectNativeOpenProjectRefreshesModelFromMutableAmbientBuildState() {
        val repo = Files.createTempDirectory("worker-native-model-cache-repo").toRealPath()
        val ambientModel = Files.createTempFile("worker-native-model-cache-ambient", ".json").toRealPath()
        try {
            val source = repo.resolve("src/main/kotlin/p/Answer.kt")
            source.parent.createDirectories()
            source.writeText("package p\nfun answer() = 42\n")
            repo.resolve("settings.gradle.kts").writeText("rootProject.name = \"native-model-cache\"\n")
            repo.resolve("build.gradle.kts").writeText("plugins { kotlin(\"jvm\") version \"2.4.10\" }\n")
            require('\'' !in ambientModel.toString()) { "fixture path cannot be shell-quoted safely" }
            repo.resolve("gradlew").writeText(
                """#!/bin/sh
                |printf '%s' '__SEMANTIC_THREAD_MODEL__'
                |cat '${ambientModel}'
                |printf '\n'
                |""".trimMargin(),
            )
            assertTrue(repo.resolve("gradlew").toFile().setExecutable(true))
            val request = buildJsonObject {
                put("repo", repo.toString())
                put("compilation", ":/main")
            }.toString().toByteArray()

            Worker(null).use { worker ->
                ambientModel.writeText(gradleFixtureModel(repo, source, "17").toString())
                val first = Json.parseToJsonElement(worker.handle(2, request)).jsonObject
                assertEquals("17", first["jvmTarget"]?.jsonPrimitive?.content)
                assertEquals(
                    "EXTRACTED_NOT_PUBLISHED",
                    first["profiling"]?.jsonObject?.get("projectModelCacheStatus")?.jsonPrimitive?.content,
                )

                ambientModel.writeText(gradleFixtureModel(repo, source, "21").toString())
                val second = Json.parseToJsonElement(worker.handle(2, request)).jsonObject
                assertEquals("21", second["jvmTarget"]?.jsonPrimitive?.content)
                assertEquals(
                    "EXTRACTED_NOT_PUBLISHED",
                    second["profiling"]?.jsonObject?.get("projectModelCacheStatus")?.jsonPrimitive?.content,
                )
                assertEquals(0, second["profiling"]?.jsonObject?.get("cacheHits")?.jsonPrimitive?.content?.toInt())
            }
        } finally {
            repo.toFile().deleteRecursively()
            Files.deleteIfExists(ambientModel)
        }
    }

    @Test
    fun k2MemoryIdentityIncludesLiveAndOverriddenSourceBytes() {
        val repo = Files.createTempDirectory("worker-k2-source-state").toRealPath()
        try {
            val source = repo.resolve("src/main/kotlin/p/Answer.kt")
            source.parent.createDirectories()
            source.writeText("package p\nfun answer() = 42\n")
            Worker(null).use { worker ->
                val baseline = worker.analysisSourceStateDigest(repo, listOf(source), emptyMap())
                val override = worker.analysisSourceStateDigest(
                    repo,
                    listOf(source),
                    mapOf("src/main/kotlin/p/Answer.kt" to "package p\nfun answer() = missing\n"),
                )
                assertNotEquals(baseline, override)

                source.writeText("package p\nfun answer() = 43\n")
                val mutated = worker.analysisSourceStateDigest(repo, listOf(source), emptyMap())
                assertNotEquals(baseline, mutated)
                assertNotEquals(override, mutated)
            }
        } finally {
            repo.toFile().deleteRecursively()
        }
    }

    @Test
    fun invalidCandidateOverrideCannotReuseBaselineK2Analysis() {
        val repo = Files.createTempDirectory("worker-k2-override-cache").toRealPath()
        try {
            val source = repo.resolve("src/main/kotlin/p/Answer.kt")
            source.parent.createDirectories()
            val baseline = "package p\nfun answer() = 42\n"
            source.writeText(baseline)
            repo.resolve("settings.gradle.kts").writeText("rootProject.name = \"k2-override-cache\"\n")
            repo.resolve("build.gradle.kts").writeText("plugins { kotlin(\"jvm\") version \"2.4.10\" }\n")
            val classpath = System.getProperty("java.class.path")
                .split(java.io.File.pathSeparator)
                .map(java.nio.file.Path::of)
                .filter { path ->
                    path.fileName.toString().let { name ->
                        name.startsWith("kotlin-stdlib-") || name.startsWith("annotations-")
                    }
                }
                .map { it.toAbsolutePath().normalize().toString() }
                .distinct()
            assertTrue(classpath.any { it.contains("kotlin-stdlib-") })
            val model = gradleFixtureModel(repo, source, "21", classpath)
            val modelLine = "__SEMANTIC_THREAD_MODEL__$model"
            require('\'' !in modelLine) { "fixture model cannot be shell-quoted safely" }
            repo.resolve("gradlew").writeText("#!/bin/sh\nprintf '%s\\n' '$modelLine'\n")
            assertTrue(repo.resolve("gradlew").toFile().setExecutable(true))

            val request = buildJsonObject {
                put("repo", repo.toString())
                put("file", "src/main/kotlin/p/Answer.kt")
                put("source", baseline)
                put("compilation", ":/main")
                put("kind", "REPLACE_FUNCTION_BODY")
                put("replacement", "{ missingSymbol() }")
                put("ownerSymbolId", "p.answer")
                put("exactTextHash", fixtureSha("42".toByteArray()))
            }.toString().toByteArray()

            Worker(null).use { worker ->
                val failure = assertFailsWith<WorkerFailure> { worker.handle(7, request) }
                assertEquals("NEW_DIAGNOSTICS", failure.code)
            }
        } finally {
            repo.toFile().deleteRecursively()
        }
    }

    @Test
    fun externalGradleEnvironmentPinsWrapperBootstrapToItsRuntimeHome() {
        val repo = Files.createTempDirectory("worker-gradle-env-repo").toRealPath()
        val runtimeHome = Files.createTempDirectory("worker-gradle-env-home").toRealPath()
        try {
            val process = sanitizedProjectModelProcess(
                listOf("true"),
                repo,
                runtimeHome,
                ProjectModelBuildTool.GRADLE,
            )
            assertEquals(runtimeHome.toString(), process.environment()["HOME"])
            assertEquals(runtimeHome.toString(), process.environment()["USERPROFILE"])
            assertEquals(runtimeHome.toString(), process.environment()["GRADLE_USER_HOME"])
            assertEquals(
                runtimeHome.resolve("wrapper/dists"),
                java.nio.file.Path.of(process.environment().getValue("GRADLE_USER_HOME")).resolve("wrapper/dists"),
            )
            assertFalse(process.environment().containsKey("MAVEN_USER_HOME"))
        } finally {
            repo.toFile().deleteRecursively()
            runtimeHome.toFile().deleteRecursively()
        }
    }

    @Test
    fun externalMavenEnvironmentUsesItsOwnHomeWithoutGradleLeak() {
        val repo = Files.createTempDirectory("worker-maven-env-repo").toRealPath()
        val runtimeHome = Files.createTempDirectory("worker-maven-env-home").toRealPath()
        try {
            val process = sanitizedProjectModelProcess(
                listOf("true"),
                repo,
                runtimeHome,
                ProjectModelBuildTool.MAVEN,
            )
            assertEquals(runtimeHome.toString(), process.environment()["HOME"])
            assertEquals(runtimeHome.toString(), process.environment()["USERPROFILE"])
            assertEquals(runtimeHome.toString(), process.environment()["MAVEN_USER_HOME"])
            assertFalse(process.environment().containsKey("GRADLE_USER_HOME"))
        } finally {
            repo.toFile().deleteRecursively()
            runtimeHome.toFile().deleteRecursively()
        }
    }

    @Test
    fun mavenPlanUsesProjectNativeEnvironmentByDefaultAndKeepsExternalModeOffline() {
        val repo = Files.createTempDirectory("worker-maven-plan").toRealPath()
        val externalRoot = Files.createTempDirectory("worker-maven-external-plan").toRealPath()
        try {
            val command = mavenModelCommand(listOf(repo.resolve("mvnw").toString()), repo, listOf("help:effective-pom"))
            assertFalse(command.contains("-o"))
            assertFalse(command.any { it.startsWith("-Dmaven.repo.local=") })
            assertFalse(command.any { it.startsWith("-Duser.home=") })
            assertFalse(Files.exists(repo.resolve(".semantic-thread")))

            val selectedPom = repo.resolve("module/pom.xml")
            val orderedArguments = listOf("-f", selectedPom.toString(), "-q", "help:effective-pom", "dependency:build-classpath")
            val selectedCommand = mavenModelCommand(listOf(repo.resolve("mvnw").toString()), repo, orderedArguments)
            assertEquals(orderedArguments, selectedCommand.takeLast(orderedArguments.size))

            prepareFixtureBuildState(externalRoot)
            val external = externalBuildStateLayout(repo, externalRoot)
            val externalCommand = mavenModelCommand(
                listOf(repo.resolve("mvnw").toString()),
                repo,
                listOf("help:effective-pom"),
                external,
            )
            assertEquals(1, externalCommand.count { it == "-o" })
            assertEquals(1, externalCommand.count { it.startsWith("-Dmaven.repo.local=") })
            assertEquals(1, externalCommand.count { it.startsWith("-Duser.home=") })
        } finally {
            repo.toFile().deleteRecursively()
            externalRoot.toFile().deleteRecursively()
        }
    }

    @Test
    fun mavenArtifactCoordinatesUseDependencyGetOrder() {
        val coordinate = mavenArtifactCoordinate(
            "org.jetbrains.kotlin",
            "kotlin-maven-allopen",
            "2.3.0",
            null,
        )
        assertEquals(
            "org.jetbrains.kotlin:kotlin-maven-allopen:2.3.0:jar",
            coordinate,
        )
        assertEquals(
            "example:compiler-plugin:1.2.3:jar:jdk21",
            mavenArtifactCoordinate("example", "compiler-plugin", "1.2.3", "jdk21"),
        )
        assertEquals(
            listOf(
                "-q",
                "dependency:get",
                "-Dartifact=$coordinate",
                "-Dtransitive=false",
            ),
            mavenDependencyGetArguments(coordinate),
        )
    }

    @Test
    fun nativeMavenLocalRepositoryFallsBackToPluginFreeCoreDebugOutput() {
        val repo = Files.createTempDirectory("worker-maven-local-repo").toRealPath()
        val localRepository = Files.createTempDirectory("worker-maven-custom-local-repo").toRealPath()
        try {
            require('\'' !in localRepository.toString()) { "fixture path cannot be shell-quoted safely" }
            val invocation = repo.resolve("maven-invocation.txt")
            require('\'' !in invocation.toString()) { "fixture path cannot be shell-quoted safely" }
            val launcher = repo.resolve("mvnw")
            launcher.writeText(
                """#!/bin/sh
                |printf '%s\n' "${'$'}@" > '$invocation'
                |for argument in "${'$'}@"; do
                |  if [ "${'$'}argument" = 'help:evaluate' ]; then
                |    exit 1
                |  fi
                |done
                |printf '%s\n' '[DEBUG] Using local repository at $localRepository'
                |exit 1
                |""".trimMargin(),
            )
            assertTrue(launcher.toFile().setExecutable(true))

            assertEquals(
                localRepository,
                MavenProjectModelExtractor().localRepositoryForTest(repo, listOf(launcher.toString())),
            )
            assertEquals("-X\n-q\n", invocation.toFile().readText())
        } finally {
            repo.toFile().deleteRecursively()
            localRepository.toFile().deleteRecursively()
        }
    }

    @Test
    fun mavenReactorSelectsExactlyRequestedModuleAndBindsDeclaredPomOrder() {
        val repo = Files.createTempDirectory("worker-maven-reactor").toRealPath()
        try {
            fun pom(directory: String, modules: List<String> = emptyList()) {
                val root = repo.resolve(directory).createDirectories()
                root.resolve("pom.xml").writeText(
                    "<project><modelVersion>4.0.0</modelVersion>" +
                        if (modules.isEmpty()) "</project>" else
                            "<modules>${modules.joinToString("") { "<module>$it</module>" }}</modules></project>",
                )
            }
            pom("", listOf("lib", "app"))
            pom("lib")
            pom("app")

            val selection = MavenProjectModelExtractor().selectReactor(repo, ":app/main")
            assertEquals(":app", selection.projectPath)
            assertEquals("app", selection.selector)
            assertEquals(repo.resolve("app").toRealPath(), selection.projectDir)
            assertEquals(
                listOf("pom.xml", "lib/pom.xml", "app/pom.xml"),
                selection.poms.map { repo.relativize(it).toString().replace('\\', '/') },
            )
            assertFailsWith<WorkerFailure> {
                MavenProjectModelExtractor().selectReactor(repo, ":../outside/main")
            }
        } finally {
            repo.toFile().deleteRecursively()
        }
    }

    @Test
    fun repoOwnedStateRejectsSymlinkAndPathEscape() {
        val repo = Files.createTempDirectory("worker-state-containment").toRealPath()
        val external = Files.createTempDirectory("worker-state-external").toRealPath()
        try {
            Files.createSymbolicLink(repo.resolve(".gradle"), external)
            assertFailsWith<WorkerFailure> {
                repoOwnedStateDirectory(repo, ".gradle")
            }
            assertFailsWith<IllegalArgumentException> {
                repoOwnedStateDirectory(repo, "..", "outside")
            }
        } finally {
            repo.toFile().deleteRecursively()
            external.toFile().deleteRecursively()
        }
    }

    @Test
    fun unrelatedTrackedSymlinkIsToleratedButKotlinSourceSymlinksCannotBeIngested() {
        val repo = Files.createTempDirectory("worker-source-symlink").toRealPath()
        val outside = Files.createTempFile("worker-source-symlink-outside", ".kt").toRealPath()
        try {
            val realSource = repo.resolve("src/main/kotlin/p/Real.kt")
            realSource.parent.createDirectories()
            realSource.writeText("package p\nfun real() = 1\n")
            repo.resolve("AGENTS.md").writeText("instructions")
            Files.createSymbolicLink(repo.resolve("CLAUDE.md"), java.nio.file.Path.of("AGENTS.md"))

            val valid = validateProjectModelSourceFiles(repo, buildJsonObject {
                putJsonArray("sourceFiles") { add(JsonPrimitive(realSource.toString())) }
            })
            assertEquals(realSource.toString(), valid["sourceFiles"]?.jsonArray?.single()?.jsonPrimitive?.content)

            val linkedInside = realSource.parent.resolve("LinkedInside.kt")
            Files.createSymbolicLink(linkedInside, java.nio.file.Path.of("Real.kt"))
            assertFailsWith<WorkerFailure> {
                repositorySourceFile(repo, linkedInside.toString())
            }

            val linkedOutside = realSource.parent.resolve("LinkedOutside.kt")
            Files.createSymbolicLink(linkedOutside, outside)
            assertFailsWith<WorkerFailure> {
                repositorySourceFile(repo, linkedOutside.toString())
            }
            assertFailsWith<WorkerFailure> {
                repositorySourceFile(repo, outside.toString())
            }
        } finally {
            repo.toFile().deleteRecursively()
            Files.deleteIfExists(outside)
        }
    }

    @Test
    fun externalBuildStateIsBoundedAndLeavesRepositoryUntouched() {
        val containing = Files.createTempDirectory("worker-external-state-containing").toRealPath()
        val repo = containing.resolve("repo").createDirectories().toRealPath()
        val root = Files.createTempDirectory("worker-external-state-root").toRealPath()
        val undeclaredGradle = Files.createTempDirectory("worker-external-state-undeclared-gradle").toRealPath()
        val undeclaredMaven = Files.createTempDirectory("worker-external-state-undeclared-maven").toRealPath()
        val symlink = root.parent.resolve("${root.fileName}-link")
        try {
            prepareFixtureBuildState(undeclaredGradle)
            undeclaredGradle.resolve("gradle-user-home/init.d").createDirectories()
                .resolve("unmanifested.init.gradle").writeText("throw new RuntimeException('injected')")
            assertFailsWith<WorkerFailure> { externalBuildStateLayout(repo, undeclaredGradle) }

            prepareFixtureBuildState(undeclaredMaven)
            undeclaredMaven.resolve("maven-repository/settings.xml").writeText("<settings/>")
            assertFailsWith<WorkerFailure> { externalBuildStateLayout(repo, undeclaredMaven) }

            prepareFixtureBuildState(root)
            val state = externalBuildStateLayout(repo, root)
            assertEquals("EXTERNAL", state.mode)
            assertTrue(state.gradleUserHome.startsWith(root))
            assertTrue(state.mavenLocalRepository.startsWith(root))
            assertFalse(Files.exists(repo.resolve(".gradle")))
            assertFalse(Files.exists(repo.resolve(".semantic-thread")))
            assertTrue(state.semanticIdentity()["seedDigest"].toString().contains("sha256:"))
            assertTrue(state.semanticIdentity()["manifestDigest"].toString().contains("sha256:"))

            // A second verifier (including a new worker process) must reject a
            // clone that has already been mutated by repository build code.
            state.gradleUserHome.resolve("mutable-project-cache.lock").writeText("runtime")
            assertFailsWith<WorkerFailure> { externalBuildStateLayout(repo, root) }

            root.resolve(K1_BUILD_STATE_SEED_FILE).writeText("sha256:${"0".repeat(64)}\n")
            assertFailsWith<WorkerFailure> { externalBuildStateLayout(repo, root) }

            Files.createSymbolicLink(symlink, root)
            assertFailsWith<WorkerFailure> { externalBuildStateLayout(repo, symlink) }

            val inside = repo.resolve("state").createDirectories()
            inside.resolve(K1_BUILD_STATE_SEED_FILE).writeText("inside")
            assertFailsWith<WorkerFailure> { externalBuildStateLayout(repo, inside) }

            containing.resolve(K1_BUILD_STATE_SEED_FILE).writeText("ancestor")
            try {
                assertFailsWith<WorkerFailure> { externalBuildStateLayout(repo, containing) }
            } finally {
                Files.deleteIfExists(containing.resolve(K1_BUILD_STATE_SEED_FILE))
            }
        } finally {
            Files.deleteIfExists(symlink)
            containing.toFile().deleteRecursively()
            root.toFile().deleteRecursively()
            undeclaredGradle.toFile().deleteRecursively()
            undeclaredMaven.toFile().deleteRecursively()
        }
    }

    @Test
    fun modelMayMutateFreshRuntimeCloneBeforeSubsequentK2Analysis() {
        val repo = Files.createTempDirectory("worker-external-state-k2-repo").toRealPath()
        val root = Files.createTempDirectory("worker-external-state-k2-root").toRealPath()
        val runtimeEvidence = Files.createTempFile("worker-external-state-runtime", ".txt").toRealPath()
        try {
            val source = repo.resolve("src/main/kotlin/p/Answer.kt")
            source.parent.createDirectories()
            source.writeText("package p\nfun answer() = 42\n")
            repo.resolve("settings.gradle.kts").writeText("rootProject.name = \"external-state-k2\"\n")
            repo.resolve("build.gradle.kts").writeText("plugins { kotlin(\"jvm\") version \"2.4.10\" }\n")

            val classpath = System.getProperty("java.class.path")
                .split(java.io.File.pathSeparator)
                .map(java.nio.file.Path::of)
                .filter { it.fileName.toString().let { name -> name.startsWith("kotlin-stdlib-") || name.startsWith("annotations-") } }
                .map { it.toAbsolutePath().normalize().toString() }
                .distinct()
            assertTrue(classpath.any { it.contains("kotlin-stdlib-") })
            val model = buildJsonObject {
                put("projectPath", ":")
                put("projectDir", repo.toString())
                put("platform", "JVM")
                put("compileTask", ":compileKotlin")
                putJsonArray("sourceFiles") { add(JsonPrimitive(source.toString())) }
                putJsonArray("analysisSourceFiles") { add(JsonPrimitive(source.toString())) }
                putJsonArray("classpath") { classpath.forEach { add(JsonPrimitive(it)) } }
                putJsonObject("classpathAuthority") {
                    put("chosen", "KOTLIN_TASK_LIBRARIES")
                    put("orderedDigest", fixtureSha(classpath.joinToString("\u0000", postfix = "\u0000").toByteArray()))
                }
                putJsonArray("friendPaths") {}
                putJsonArray("compilerPlugins") {}
                putJsonArray("compilerPluginOptions") {}
                putJsonArray("dependencyCoordinates") {}
                putJsonArray("repositories") {}
                putJsonArray("reactorPoms") {}
                putJsonArray("buildPlugins") {}
                putJsonObject("generatedSourceConfiguration") {
                    putJsonArray("roots") {}
                    putJsonArray("producers") {}
                    put("status", "NONE_DISCOVERED")
                }
                put("compilerVersion", WORKER_COMPILER_VERSION)
                put("languageVersion", "2.4")
                put("apiVersion", "2.4")
                put("jvmTarget", "21")
                putJsonArray("freeCompilerArguments") {}
                putJsonArray("optIns") {}
                putJsonArray("tasks") { add(JsonPrimitive("test")) }
                putJsonArray("buildModelBoundaries") {}
                putJsonObject("fieldBoundaries") {
                    put("libraries", "AVAILABLE_ORDERED")
                    put("friendPaths", "AVAILABLE_ORDERED")
                    put("compilerPlugins", "AVAILABLE_ORDERED")
                    put("freeCompilerArguments", "AVAILABLE_ORDERED")
                    put("optIns", "AVAILABLE_ORDERED")
                    put("jdkHome", "AVAILABLE_BUILD_JVM")
                }
                put("gradleVersion", "fixture")
                put("jdkHome", System.getProperty("java.home"))
            }
            val modelLine = "__SEMANTIC_THREAD_MODEL__$model"
            require('\'' !in modelLine && '\'' !in runtimeEvidence.toString()) {
                "fixture path cannot be shell-quoted safely"
            }
            repo.resolve("gradlew").writeText(
                """#!/bin/sh
                |previous=''
                |for argument in "${'$'}@"; do
                |  if [ "${'$'}previous" = 'project-cache' ]; then
                |    mkdir -p "${'$'}argument"
                |    printf '%s' runtime > "${'$'}argument/model-runtime.lock"
                |    printf '%s' "${'$'}argument" > '$runtimeEvidence'
                |  fi
                |  if [ "${'$'}argument" = '--project-cache-dir' ]; then
                |    previous='project-cache'
                |  else
                |    previous=''
                |  fi
                |done
                |printf '%s\n' '$modelLine'
                |""".trimMargin(),
            )
            assertTrue(repo.resolve("gradlew").toFile().setExecutable(true))
            prepareFixtureBuildState(root)
            val manifestBefore = root.resolve(K1_BUILD_STATE_MANIFEST_FILE).toFile().readBytes()
            val markerBefore = root.resolve(K1_BUILD_STATE_SEED_FILE).toFile().readBytes()
            val request = buildJsonObject {
                put("repo", repo.toString())
                put("compilation", ":/main")
            }.toString().toByteArray()

            var runtimeProjectCache = root
            Worker(root).use { worker ->
                worker.handle(2, request)
                val reopened = Json.parseToJsonElement(worker.handle(2, request)).jsonObject
                assertEquals(
                    "MEMORY_HIT",
                    reopened["profiling"]?.jsonObject?.get("projectModelCacheStatus")?.jsonPrimitive?.content,
                )
                runtimeProjectCache = java.nio.file.Path.of(runtimeEvidence.toFile().readText())
                assertTrue(runtimeProjectCache.resolve("model-runtime.lock").isRegularFile())
                assertFalse(runtimeProjectCache.startsWith(repo))
                assertFalse(runtimeProjectCache.startsWith(root))
                assertFalse(root.resolve("gradle-user-home/project-cache").exists())
                val indexed = Json.parseToJsonElement(worker.handle(3, request)).jsonObject
                assertEquals(true, indexed["k2Validated"]?.jsonPrimitive?.boolean)
            }
            assertFalse(runtimeProjectCache.exists())

            // A selected-compiler worker may start after the discovery worker.
            // It must get another clean runtime from the unchanged authority.
            Worker(root).use { worker ->
                val indexed = Json.parseToJsonElement(worker.handle(3, request)).jsonObject
                assertEquals(true, indexed["k2Validated"]?.jsonPrimitive?.boolean)
                runtimeProjectCache = java.nio.file.Path.of(runtimeEvidence.toFile().readText())
                assertTrue(runtimeProjectCache.resolve("model-runtime.lock").isRegularFile())
            }
            assertFalse(runtimeProjectCache.exists())
            assertTrue(manifestBefore.contentEquals(root.resolve(K1_BUILD_STATE_MANIFEST_FILE).toFile().readBytes()))
            assertTrue(markerBefore.contentEquals(root.resolve(K1_BUILD_STATE_SEED_FILE).toFile().readBytes()))
            assertFalse(repo.resolve(".gradle").exists())
            assertFalse(repo.resolve(".semantic-thread").exists())
            assertFalse(repo.resolve(".kotlin").exists())
        } finally {
            repo.toFile().deleteRecursively()
            root.toFile().deleteRecursively()
            Files.deleteIfExists(runtimeEvidence)
        }
    }

    @Test
    fun compilerUtf16OffsetsAreConvertedToUtf8BytesWithoutSplittingEmoji() {
        val source = "val marker = \"😀\"\nfun answer() = 42\n"
        val functionStart = source.indexOf("fun")
        val functionEnd = source.length
        val bytes = compilerRangeToUtf8Bytes(source, functionStart, functionEnd)
        assertEquals(source.substring(0, functionStart).toByteArray().size, bytes?.first)
        assertEquals(source.toByteArray().size, bytes?.last?.plus(1))
        val provenBytes = requireNotNull(bytes)
        assertEquals(
            2..2,
            utf8ByteRangeToOneBasedLines(
                utf8LineIndex(source),
                provenBytes.first,
                provenBytes.last + 1,
            ),
        )

        val multiline = "val marker = \"😀\"\r\nfun answer() =\r\n    42\r\n"
        val multilineStart = multiline.indexOf("fun")
        val multilineBytes = compilerRangeToUtf8Bytes(multiline, multilineStart, multiline.length)!!
        val multilineIndex = utf8LineIndex(multiline)
        assertEquals(
            2..3,
            utf8ByteRangeToOneBasedLines(
                multilineIndex,
                multilineBytes.first,
                multilineBytes.last + 1,
            ),
        )
        assertEquals(
            4..4,
            utf8ByteRangeToOneBasedLines(
                multilineIndex,
                multilineIndex.byteSize,
                multilineIndex.byteSize,
            ),
        )
        assertNull(utf8ByteRangeToOneBasedLines(multilineIndex, -1, 0))
        assertNull(
            utf8ByteRangeToOneBasedLines(
                multilineIndex,
                0,
                multilineIndex.byteSize + 1,
            ),
        )

        val emojiStart = source.indexOf("😀")
        assertNull(compilerRangeToUtf8Bytes(source, emojiStart + 1, emojiStart + 2))
        assertNull(compilerRangeToUtf8Bytes(source, -1, 0))
        assertNull(compilerRangeToUtf8Bytes(source, 0, source.length + 1))

        val repo = Files.createTempDirectory("worker-relation-path").toRealPath()
        try {
            assertEquals("src/A.kt", repositoryRelativeCompilerPath(repo, "src/A.kt"))
            assertNull(repositoryRelativeCompilerPath(repo, "../A.kt"))
            assertNull(repositoryRelativeCompilerPath(repo, repo.parent.resolve("outside.kt").toString()))
        } finally {
            repo.toFile().deleteRecursively()
        }
    }

    @Test
    fun futureCompilerDescriptorValuesBecomeTypedBoundaries() {
        val source = "fun answer() = 42"
        fun descriptor(change: JsonObject.() -> JsonObject = { this }): JsonObject {
            val base = buildJsonObject {
                put("recordType", "DECLARATION_DESCRIPTOR")
                put("file", "/repo/A.kt")
                put("start", 0)
                put("end", source.length)
                put("symbolIdentity", "callable:p/answer#jvm:()I")
                put("ownerIdentity", "package:p")
                put("declarationKind", "FUNCTION")
                put("visibility", "public")
                put("effectiveVisibility", "public")
                put("modality", "FINAL")
                put("returnType", "kotlin/Int")
                putJsonArray("parameterTypes") {}
            }
            return base.change()
        }
        fun changed(field: String, value: String) = descriptor {
            val original = this
            buildJsonObject {
                original.forEach(::put)
                put(field, value)
            }
        }

        assertNull(descriptorUnsupportedReason(descriptor(), "A.kt", source))
        assertEquals("UNKNOWN_DECLARATION_KIND", descriptorUnsupportedReason(changed("declarationKind", "FUTURE_KIND"), "A.kt", source))
        assertEquals("UNKNOWN_VISIBILITY", descriptorUnsupportedReason(changed("visibility", "package"), "A.kt", source))
        assertEquals("UNKNOWN_EFFECTIVE_VISIBILITY", descriptorUnsupportedReason(changed("effectiveVisibility", "local"), "A.kt", source))
        assertEquals("UNKNOWN_MODALITY", descriptorUnsupportedReason(changed("modality", "FUTURE_MODALITY"), "A.kt", source))
        assertEquals("UNRESOLVED_DESCRIPTOR_TYPE", descriptorUnsupportedReason(changed("returnType", "<ERROR TYPE>"), "A.kt", source))
        assertEquals("INVALID_DESCRIPTOR_IDENTITY", descriptorUnsupportedReason(changed("symbolIdentity", ""), "A.kt", source))
        assertEquals("INVALID_DESCRIPTOR_SOURCE_RANGE", descriptorUnsupportedReason(buildJsonObject {
            descriptor().forEach(::put)
            put("end", source.length + 1)
        }, "A.kt", source))
        assertEquals("UNKNOWN_VISIBILITY", descriptorUnsupportedReason(buildJsonObject {
            descriptor().forEach(::put)
            put("visibility", buildJsonObject { put("future", true) })
        }, "A.kt", source))
        assertEquals("UNRESOLVED_DESCRIPTOR_TYPE", descriptorUnsupportedReason(buildJsonObject {
            descriptor().forEach(::put)
            put("returnType", buildJsonObject { put("future", "shape") })
        }, "A.kt", source))
    }

    @Test
    fun malformedCompilerFactRowIsRetainedAsBothTypedGraphBoundaries() {
        val rows = parseCompilerFactLines(listOf("{not-json", "[]", "{}", ""))
        assertEquals(6, rows.size)
        assertEquals(
            setOf("DECLARATION_DESCRIPTOR_BOUNDARY", "DECLARATION_RELATION_BOUNDARY"),
            rows.map { it["recordType"]?.toString()?.trim('"') }.toSet(),
        )
        assertTrue(rows.all { it["code"]?.toString() == "\"MALFORMED_COMPILER_FACT_ROW\"" })
        assertTrue(rows.all { it["rawRowHash"]?.toString()?.trim('"')?.startsWith("sha256:") == true })
    }
}
