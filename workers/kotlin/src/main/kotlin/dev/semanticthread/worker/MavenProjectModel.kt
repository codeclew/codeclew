package dev.semanticthread.worker

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import org.w3c.dom.Element
import java.io.File
import java.io.IOException
import java.nio.file.Files
import java.nio.file.Path
import javax.xml.parsers.DocumentBuilderFactory
import kotlin.io.path.exists
import kotlin.io.path.extension
import kotlin.io.path.isExecutable
import kotlin.io.path.isRegularFile
import kotlin.io.path.readText

internal class MavenProjectModelExtractor {
    fun extract(repo: Path, compilation: String?): JsonObject {
        rejectModules(repo)
        val sourceSet = selectedSourceSet(compilation)
        val temporary = Files.createTempDirectory("semantic-thread-maven-model")
        try {
            val effectivePom = temporary.resolve("effective-pom.xml")
            val classpathFile = temporary.resolve("classpath.txt")
            val launcher = launcher(repo)
            val scope = if (sourceSet == "test") "test" else "compile"
            val command = launcher + listOf(
                "-q",
                "-DskipTests",
                "-Doutput=$effectivePom",
                "-Dmdep.outputFile=$classpathFile",
                "-Dmdep.includeScope=$scope",
                "help:effective-pom",
                "dependency:build-classpath",
            )
            val process = start(command, repo, "Maven model extraction")
            val output = process.inputStream.bufferedReader().readText()
            val status = process.waitFor()
            if (status != 0 || !effectivePom.isRegularFile() || !classpathFile.isRegularFile()) {
                throw WorkerFailure(
                    "UNSUPPORTED_PROJECT_CONFIGURATION",
                    "Maven model extraction failed: ${output.takeLast(MAX_MAVEN_ERROR_CHARS)}",
                )
            }

            val project = parse(effectivePom).documentElement
            val kotlinPlugin = project.descendants("plugin").filter { plugin ->
                plugin.directText("groupId").orEmpty().ifBlank { "org.apache.maven.plugins" } == "org.jetbrains.kotlin" &&
                    plugin.directText("artifactId") == "kotlin-maven-plugin"
            }.maxByOrNull { plugin ->
                plugin.directChild("dependencies")?.directChildren("dependency").orEmpty().size * 100 +
                    if (plugin.directChild("configuration") != null) 10 else 0
            } ?: throw WorkerFailure(
                "UNSUPPORTED_PROJECT_CONFIGURATION",
                "Maven Kotlin JVM plugin is required",
            )
            val compilerVersion = kotlinPlugin.directText("version")
                ?: throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "Kotlin Maven plugin version is required")
            val compilerLine = compilerVersion.split('.').take(2).joinToString(".")
            val configuration = kotlinPlugin.directChild("configuration")
            val selectedSources = kotlinSources(configuredRoot(project, repo, sourceSet))
            val analysisSourceFiles = if (sourceSet == "test") {
                (kotlinSources(configuredRoot(project, repo, "main")) + selectedSources).distinct().sorted()
            } else {
                selectedSources
            }
            if (selectedSources.isEmpty()) {
                throw WorkerFailure(
                    "UNSUPPORTED_PROJECT_CONFIGURATION",
                    "Maven Kotlin source set '$sourceSet' is empty",
                )
            }
            val classpathEntries = classpathFile.toFile().readText()
                .trim()
                .split(File.pathSeparator)
                .filter(String::isNotBlank)
                .map(Path::of)
            val missingClasspath = classpathEntries.filterNot(Path::exists)
            if (missingClasspath.isNotEmpty()) {
                throw WorkerFailure(
                    "UNSUPPORTED_PROJECT_CONFIGURATION",
                    "Maven classpath contains missing entries: ${missingClasspath.take(3).joinToString()}",
                )
            }
            val classpath = classpathEntries
                .map { it.toAbsolutePath().normalize().toString() }
                .distinct()
                .sorted()
            val javaVersion = project.directChild("properties")?.directText("java.version")
            val jvmTarget = configuration?.directText("jvmTarget") ?: javaVersion ?: "21"
            val languageVersion = configuration?.directText("languageVersion") ?: compilerLine
            val apiVersion = configuration?.directText("apiVersion") ?: compilerLine
            val freeCompilerArguments = configuration
                ?.directChild("args")
                ?.directChildren("arg")
                ?.mapNotNull { it.textValue() }
                .orEmpty()
                .sorted()
            val compilerPlugins = compilerPluginArtifacts(kotlinPlugin, repo, launcher)
            val compilerPluginOptions = compilerPluginOptions(configuration)

            return buildJsonObject {
                put("buildSystem", "MAVEN")
                put("buildLauncher", if (launcher.single().endsWith("mvnw")) "./mvnw" else "mvn")
                put("projectPath", ":")
                put("compileTask", if (sourceSet == "test") "test-compile" else "compile")
                putJsonArray("sourceFiles") { selectedSources.forEach { add(JsonPrimitive(it.toString())) } }
                putJsonArray("analysisSourceFiles") {
                    analysisSourceFiles.forEach { add(JsonPrimitive(it.toString())) }
                }
                putJsonArray("classpath") { classpath.forEach { add(JsonPrimitive(it)) } }
                putJsonArray("friendPaths") {
                    if (sourceSet == "test") add(JsonPrimitive(repo.resolve("target/classes").toString()))
                }
                putJsonArray("compilerPlugins") {
                    compilerPlugins.forEach { add(JsonPrimitive(it.toString())) }
                }
                putJsonArray("compilerPluginOptions") {
                    compilerPluginOptions.forEach { add(JsonPrimitive(it)) }
                }
                put("compilerVersion", compilerVersion)
                put("languageVersion", languageVersion)
                put("apiVersion", apiVersion)
                put("jvmTarget", jvmTarget)
                put("freeCompilerArguments", JsonArray(freeCompilerArguments.map(::JsonPrimitive)))
                putJsonArray("optIns") {}
                put("tasks", JsonArray(listOf(JsonPrimitive("test"))))
                put("mavenVersion", mavenVersion(launcher, repo))
                put("jdkHome", System.getProperty("java.home"))
            }
        } finally {
            temporary.toFile().deleteRecursively()
        }
    }

    private fun launcher(repo: Path): List<String> {
        val wrapper = repo.resolve("mvnw")
        if (wrapper.isRegularFile() && wrapper.isExecutable()) return listOf(wrapper.toString())
        if (executableOnPath("mvn")) return listOf("mvn")
        throw WorkerFailure(
            "UNSUPPORTED_PROJECT_CONFIGURATION",
            "neither executable ./mvnw nor Maven on PATH is available",
        )
    }

    private fun executableOnPath(name: String): Boolean =
        System.getenv("PATH")
            ?.split(File.pathSeparator)
            ?.map { Path.of(it.ifBlank { "." }).resolve(name) }
            ?.any { it.isRegularFile() && it.isExecutable() }
            ?: false

    private fun compilerPluginArtifacts(
        kotlinPlugin: Element,
        repo: Path,
        launcher: List<String>,
    ): List<Path> {
        val localRepository = localRepository(repo)
        return kotlinPlugin.directChild("dependencies")
            ?.directChildren("dependency")
            .orEmpty()
            .mapNotNull { dependency ->
                val group = dependency.directText("groupId") ?: return@mapNotNull null
                val artifact = dependency.directText("artifactId") ?: return@mapNotNull null
                val version = dependency.directText("version") ?: return@mapNotNull null
                val type = dependency.directText("type") ?: "jar"
                val classifier = dependency.directText("classifier")
                if (type != "jar") return@mapNotNull null
                val suffix = classifier?.let { "-$it" }.orEmpty()
                val path = localRepository
                    .resolve(group.replace('.', '/'))
                    .resolve(artifact)
                    .resolve(version)
                    .resolve("$artifact-$version$suffix.jar")
                if (!path.isRegularFile()) {
                    resolvePluginArtifact(launcher, repo, group, artifact, version, classifier)
                }
                if (!path.isRegularFile()) {
                    throw WorkerFailure(
                        "UNSUPPORTED_PROJECT_CONFIGURATION",
                        "Maven compiler plugin artifact is missing: $group:$artifact:$version",
                    )
                }
                path.toAbsolutePath().normalize()
            }
            .distinct()
            .sorted()
    }

    private fun resolvePluginArtifact(
        launcher: List<String>,
        repo: Path,
        group: String,
        artifact: String,
        version: String,
        classifier: String?,
    ) {
        val coordinate = listOfNotNull(group, artifact, "jar", classifier, version).joinToString(":")
        val process = start(
            launcher + listOf("-q", "dependency:get", "-Dartifact=$coordinate"),
            repo,
            "Maven compiler plugin resolution",
        )
        val output = process.inputStream.bufferedReader().readText()
        if (process.waitFor() != 0) {
            throw WorkerFailure(
                "UNSUPPORTED_PROJECT_CONFIGURATION",
                "Maven compiler plugin resolution failed: ${output.takeLast(MAX_MAVEN_ERROR_CHARS)}",
            )
        }
    }

    private fun localRepository(repo: Path): Path {
        val configured = repo.resolve(".mvn/maven.config")
            .takeIf { it.isRegularFile() }
            ?.readText()
            ?.lineSequence()
            ?.flatMap { it.split(Regex("\\s+")).asSequence() }
            ?.firstOrNull { it.startsWith("-Dmaven.repo.local=") }
            ?.substringAfter('=')
        return configured?.let(Path::of)?.let { if (it.isAbsolute) it else repo.resolve(it).normalize() }
            ?: Path.of(System.getProperty("user.home"), ".m2", "repository")
    }

    private fun compilerPluginOptions(configuration: Element?): List<String> {
        val enabled = configuration
            ?.directChild("compilerPlugins")
            ?.directChildren("plugin")
            ?.mapNotNull { it.textValue() }
            .orEmpty()
        val presets = enabled.mapNotNull { plugin ->
            when (plugin) {
                "spring" -> "plugin:org.jetbrains.kotlin.allopen:preset=spring"
                "jpa" -> "plugin:org.jetbrains.kotlin.noarg:preset=jpa"
                else -> null
            }
        }
        val configured = configuration
            ?.directChild("pluginOptions")
            ?.directChildren("option")
            ?.mapNotNull { it.textValue() }
            .orEmpty()
            .map { option ->
                when {
                    option.startsWith("plugin:") -> option
                    option.startsWith("all-open:") ->
                        "plugin:org.jetbrains.kotlin.allopen:${option.substringAfter(':')}"
                    option.startsWith("no-arg:") ->
                        "plugin:org.jetbrains.kotlin.noarg:${option.substringAfter(':')}"
                    else -> option
                }
            }
        return (presets + configured).distinct().sorted()
    }

    private fun mavenVersion(launcher: List<String>, repo: Path): String {
        return try {
            val process = ProcessBuilder(launcher + "-version")
                .directory(repo.toFile())
                .redirectErrorStream(true)
                .start()
            val firstLine = process.inputStream.bufferedReader().readLine().orEmpty()
            if (process.waitFor() == 0) firstLine.removePrefix("Apache Maven ").trim().ifBlank { "unknown" } else "unknown"
        } catch (_: Exception) {
            "unknown"
        }
    }

    private fun start(command: List<String>, repo: Path, operation: String) = try {
        ProcessBuilder(command)
            .directory(repo.toFile())
            .redirectErrorStream(true)
            .start()
    } catch (error: IOException) {
        throw WorkerFailure(
            "UNSUPPORTED_PROJECT_CONFIGURATION",
            "$operation could not start: ${error.message}",
        )
    }

    private fun rejectModules(repo: Path) {
        val root = parse(repo.resolve("pom.xml")).documentElement
        val modules = root.directChild("modules")?.directChildren("module").orEmpty()
        if (modules.isNotEmpty()) {
            throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "multi-module Maven projects are not supported")
        }
    }

    private fun selectedSourceSet(compilation: String?): String {
        val selected = compilation ?: ":/main"
        return if ('/' in selected) selected.substringAfterLast('/')
        else if (selected.contains("test", ignoreCase = true)) "test" else "main"
    }

    private fun configuredRoot(project: Element, repo: Path, sourceSet: String): Path {
        val build = project.directChild("build")
        val configured = if (sourceSet == "test") build?.directText("testSourceDirectory")
        else build?.directText("sourceDirectory")
        val fallback = if (sourceSet == "test") "src/test/kotlin" else "src/main/kotlin"
        val value = configured.orEmpty().ifBlank { fallback }
            .replace("${'$'}{project.basedir}", repo.toString())
            .replace("${'$'}{basedir}", repo.toString())
        val path = Path.of(value)
        return if (path.isAbsolute) path.normalize() else repo.resolve(path).normalize()
    }

    private fun kotlinSources(root: Path): List<Path> {
        if (!root.exists()) return emptyList()
        return Files.walk(root).use { paths ->
            paths.filter { it.isRegularFile() && it.extension == "kt" }.sorted().toList()
        }
    }

    private fun parse(path: Path) = DocumentBuilderFactory.newInstance().apply {
        isNamespaceAware = true
        setFeature("http://apache.org/xml/features/disallow-doctype-decl", true)
        setFeature("http://xml.org/sax/features/external-general-entities", false)
        setFeature("http://xml.org/sax/features/external-parameter-entities", false)
    }.newDocumentBuilder().parse(path.toFile())

    private fun Element.directChild(name: String): Element? =
        directChildren(name).firstOrNull()

    private fun Element.directChildren(name: String): List<Element> =
        (0 until childNodes.length)
            .map(childNodes::item)
            .filterIsInstance<Element>()
            .filter { it.localName == name || it.nodeName == name }

    private fun Element.descendants(name: String): List<Element> =
        (0 until getElementsByTagNameNS("*", name).length)
            .map { getElementsByTagNameNS("*", name).item(it) }
            .filterIsInstance<Element>()

    private fun Element.directText(name: String): String? = directChild(name)?.textValue()

    private fun Element.textValue(): String? = textContent?.trim()?.takeIf(String::isNotEmpty)

    private companion object {
        const val MAX_MAVEN_ERROR_CHARS = 2_000
    }
}
