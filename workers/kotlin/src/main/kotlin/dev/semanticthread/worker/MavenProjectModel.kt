package dev.semanticthread.worker

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlinx.serialization.json.putJsonObject
import org.w3c.dom.Element
import java.io.File
import java.io.IOException
import java.nio.file.Files
import java.nio.file.Path
import java.security.MessageDigest
import javax.xml.parsers.DocumentBuilderFactory
import kotlin.io.path.exists
import kotlin.io.path.extension
import kotlin.io.path.isExecutable
import kotlin.io.path.isRegularFile
import kotlin.io.path.invariantSeparatorsPathString
import kotlin.io.path.readBytes

internal fun mavenModelCommand(
    launcher: List<String>,
    repo: Path,
    arguments: List<String>,
    preparedState: BuildStateLayout? = null,
): List<String> {
    val state = preparedState ?: buildStateLayout(repo)
    val localRepository = state.mavenLocalRepository
    return launcher + listOf(
        "-o",
        "-Dmaven.repo.local=$localRepository",
        "-Duser.home=$localRepository",
    ) + arguments
}

private fun mavenSha(bytes: ByteArray): String = "sha256:" +
    MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

internal class MavenProjectModelExtractor(
    private val preparedState: BuildStateLayout? = null,
) {
    private fun state(repo: Path): BuildStateLayout = preparedState ?: buildStateLayout(repo)
    internal data class ReactorSelection(
        val projectPath: String,
        val projectDir: Path,
        val selector: String?,
        val poms: List<Path>,
    )

    fun extract(repo: Path, compilation: String?): JsonObject {
        val reactor = selectReactor(repo, compilation)
        val sourceSet = selectedSourceSet(compilation)
        val temporary = Files.createTempDirectory("semantic-thread-maven-model")
        try {
            val effectivePom = temporary.resolve("effective-pom.xml")
            val classpathFile = temporary.resolve("classpath.txt")
            val launcher = launcher(repo)
            val scope = if (sourceSet == "test") "test" else "compile"
            // A reactor-wide `-pl/-am` invocation executes both goals for every
            // selected/upstream project and lets them race on the same output
            // files. Point Maven at the exact selected POM instead; the PREPARE
            // phase must have installed reactor dependencies into the pinned
            // repository before this offline discovery command runs.
            val reactorArguments = listOf("-f", reactor.projectDir.resolve("pom.xml").toString())
            val command = mavenModelCommand(launcher, repo, reactorArguments + listOf(
                "-q",
                "-DskipTests",
                "-Doutput=$effectivePom",
                "-Dmdep.outputFile=$classpathFile",
                "-Dmdep.includeScope=$scope",
                "help:effective-pom",
                "dependency:build-classpath",
            ), state(repo))
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
            val selectedRoots = configuredRoots(project, configuration, reactor.projectDir, sourceSet)
            val selectedSources = selectedRoots.flatMap { kotlinSources(repo, it) }.distinct().sorted()
            val analysisSourceFiles = if (sourceSet == "test") {
                (configuredRoots(project, configuration, reactor.projectDir, "main")
                    .flatMap { kotlinSources(repo, it) } + selectedSources).distinct().sorted()
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
            val javaVersion = project.directChild("properties")?.directText("java.version")
            val jvmTarget = configuration?.directText("jvmTarget") ?: javaVersion ?: "21"
            val languageVersion = configuration?.directText("languageVersion") ?: compilerLine
            val apiVersion = configuration?.directText("apiVersion") ?: compilerLine
            val freeCompilerArguments = configuration
                ?.directChild("args")
                ?.directChildren("arg")
                ?.mapNotNull { it.textValue() }
                .orEmpty()
            val compilerPlugins = compilerPluginArtifacts(
                kotlinPlugin,
                repo,
                mavenModelCommand(launcher, repo, emptyList(), state(repo)),
            )
            val compilerPluginOptions = compilerPluginOptions(configuration)
            val buildPlugins = project.directChild("build")
                ?.directChild("plugins")
                ?.directChildren("plugin")
                .orEmpty()
            val dependencyCoordinates = project.directChild("dependencies")
                ?.directChildren("dependency")
                .orEmpty()
                .map { dependency ->
                    buildJsonObject {
                        put("group", dependency.directText("groupId") ?: "<unavailable>")
                        put("name", dependency.directText("artifactId") ?: "<unavailable>")
                        put("version", dependency.directText("version") ?: "<unavailable>")
                        put("scope", dependency.directText("scope") ?: "compile")
                        put("type", dependency.directText("type") ?: "jar")
                        dependency.directText("classifier")?.let { put("classifier", it) }
                        put("optional", dependency.directText("optional") == "true")
                    }
                }
            val repositories = project.directChild("repositories")
                ?.directChildren("repository")
                .orEmpty()
                .map { repository -> buildJsonObject {
                    put("id", repository.directText("id") ?: "<unavailable>")
                    put("url", repository.directText("url") ?: "<unavailable>")
                    put("layout", repository.directText("layout") ?: "default")
                } }
            val buildPluginCoordinates = buildPlugins.map { plugin -> buildJsonObject {
                put("group", plugin.directText("groupId") ?: "org.apache.maven.plugins")
                put("name", plugin.directText("artifactId") ?: "<unavailable>")
                put("version", plugin.directText("version") ?: "<unavailable>")
            } }
            val generatedRoots = selectedRoots.filter { root ->
                root.normalize().startsWith(reactor.projectDir.resolve("target/generated-sources").normalize()) ||
                    root.normalize().startsWith(reactor.projectDir.resolve("target/generated-test-sources").normalize())
            }
            val hasSurefire = buildPlugins.any {
                it.directText("artifactId") == "maven-surefire-plugin"
            }
            val hasFailsafe = buildPlugins.any {
                it.directText("artifactId") == "maven-failsafe-plugin"
            }
            val mavenTestLifecycle = when {
                hasFailsafe -> "UNSUPPORTED_FAILSAFE"
                hasSurefire -> "SUREFIRE"
                else -> "UNSUPPORTED_AMBIGUOUS"
            }

            return buildJsonObject {
                put("buildSystem", "MAVEN")
                put("buildLauncher", if (launcher.single().endsWith("mvnw")) "./mvnw" else "mvn")
                put("projectPath", reactor.projectPath)
                put("projectDir", reactor.projectDir.toString())
                put("platform", "JVM")
                put("compileTask", if (sourceSet == "test") "test-compile" else "compile")
                putJsonArray("sourceFiles") { selectedSources.forEach { add(JsonPrimitive(it.toString())) } }
                putJsonArray("analysisSourceFiles") {
                    analysisSourceFiles.forEach { add(JsonPrimitive(it.toString())) }
                }
                putJsonArray("classpath") { classpath.forEach { add(JsonPrimitive(it)) } }
                putJsonObject("classpathAuthority") {
                    put("chosen", "MAVEN_DEPENDENCY_BUILD_CLASSPATH")
                    put("orderedDigest", mavenSha(classpath.joinToString("\u0000", postfix = "\u0000").toByteArray()))
                }
                putJsonArray("friendPaths") {
                    if (sourceSet == "test") {
                        add(JsonPrimitive(reactor.projectDir.resolve("target/classes").toString()))
                    }
                }
                putJsonArray("compilerPlugins") {
                    compilerPlugins.forEach { add(JsonPrimitive(it.toString())) }
                }
                putJsonArray("compilerPluginOptions") {
                    compilerPluginOptions.forEach { add(JsonPrimitive(it)) }
                }
                putJsonArray("dependencyCoordinates") { dependencyCoordinates.forEach(::add) }
                putJsonArray("repositories") { repositories.forEach(::add) }
                putJsonArray("reactorPoms") {
                    reactor.poms.forEach { pom -> add(buildJsonObject {
                        put("path", repo.relativize(pom).invariantSeparatorsPathString)
                        put("hash", mavenSha(pom.readBytes()))
                    }) }
                }
                putJsonArray("buildPlugins") { buildPluginCoordinates.forEach(::add) }
                putJsonObject("generatedSourceConfiguration") {
                    putJsonArray("roots") { generatedRoots.forEach { root -> add(JsonPrimitive(repo.relativize(root).invariantSeparatorsPathString)) } }
                    putJsonArray("producers") { buildPluginCoordinates.forEach(::add) }
                    put("status", if (generatedRoots.isEmpty()) "NONE_DISCOVERED" else "ROOTS_AND_BUILD_PLUGIN_SET")
                }
                put("compilerVersion", compilerVersion)
                put("languageVersion", languageVersion)
                put("apiVersion", apiVersion)
                put("jvmTarget", jvmTarget)
                put("freeCompilerArguments", JsonArray(freeCompilerArguments.map(::JsonPrimitive)))
                putJsonArray("optIns") {}
                put("tasks", JsonArray(listOf(JsonPrimitive("test"))))
                put("mavenTestLifecycle", mavenTestLifecycle)
                putJsonArray("buildModelBoundaries") {
                    add(JsonPrimitive("MAVEN_CLASSPATH_ARTIFACT_TO_COORDINATE_BIJECTION_UNAVAILABLE"))
                    add(JsonPrimitive("MAVEN_TOOLCHAIN_SELECTION_USES_BUILD_JVM"))
                    if (generatedRoots.isNotEmpty()) add(JsonPrimitive("MAVEN_GENERATED_SOURCE_EXACT_PRODUCER_MAPPING_UNAVAILABLE"))
                }
                putJsonObject("fieldBoundaries") {
                    put("libraries", "AVAILABLE_ORDERED")
                    put("friendPaths", "AVAILABLE_ORDERED")
                    put("compilerPlugins", "AVAILABLE_ORDERED")
                    put("freeCompilerArguments", "AVAILABLE_ORDERED")
                    put("optIns", "UNAVAILABLE_PROVIDER")
                    put("jdkHome", "BOUNDARY_BUILD_JVM")
                }
                put("mavenVersion", mavenVersion(mavenModelCommand(launcher, repo, emptyList(), state(repo)), repo))
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

    private fun localRepository(repo: Path): Path =
        state(repo).mavenLocalRepository

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
        return (presets + configured).distinct()
    }

    private fun mavenVersion(launcher: List<String>, repo: Path): String {
        return try {
            val state = state(repo)
            val process = sanitizedProjectModelProcess(
                launcher + "-version",
                repo,
                state.mavenLocalRepository.takeIf { state.mode == "EXTERNAL" },
                ProjectModelBuildTool.MAVEN.takeIf { state.mode == "EXTERNAL" },
            ).start()
            val firstLine = process.inputStream.bufferedReader().readLine().orEmpty()
            if (process.waitFor() == 0) firstLine.removePrefix("Apache Maven ").trim().ifBlank { "unknown" } else "unknown"
        } catch (_: Exception) {
            "unknown"
        }
    }

    private fun start(command: List<String>, repo: Path, operation: String) = try {
        state(repo).let { state ->
            sanitizedProjectModelProcess(
                command,
                repo,
                state.mavenLocalRepository.takeIf { state.mode == "EXTERNAL" },
                ProjectModelBuildTool.MAVEN.takeIf { state.mode == "EXTERNAL" },
            ).start()
        }
    } catch (error: IOException) {
        throw WorkerFailure(
            "UNSUPPORTED_PROJECT_CONFIGURATION",
            "$operation could not start: ${error.message}",
        )
    }

    internal fun selectReactor(repo: Path, compilation: String?): ReactorSelection {
        val requested = (compilation ?: ":/main").substringBeforeLast('/').ifBlank { ":" }
        if (!requested.startsWith(':')) {
            throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "Maven compilation module must start with ':'")
        }
        val requestedSelector = requested.removePrefix(":").replace(':', '/').ifBlank { null }
        if (requestedSelector?.split('/')?.any { it.isBlank() || it == "." || it == ".." } == true) {
            throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "Maven reactor module path is invalid")
        }
        val canonicalRepo = repo.toRealPath()
        val ordered = mutableListOf<Path>()
        val visited = mutableSetOf<Path>()
        fun walk(pom: Path) {
            val canonicalPom = pom.toRealPath()
            if (!canonicalPom.startsWith(canonicalRepo) || !visited.add(canonicalPom)) {
                if (!canonicalPom.startsWith(canonicalRepo)) {
                    throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "Maven reactor module escapes repository")
                }
                return
            }
            ordered.add(canonicalPom)
            val project = parse(canonicalPom).documentElement
            project.directChild("modules")?.directChildren("module").orEmpty().forEach { module ->
                val relative = module.textValue()
                    ?: throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "Maven reactor has an empty module")
                val childPom = canonicalPom.parent.resolve(relative).normalize().resolve("pom.xml")
                if (!childPom.isRegularFile()) {
                    throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "Maven reactor module pom is missing")
                }
                walk(childPom)
            }
        }
        walk(canonicalRepo.resolve("pom.xml"))
        val selectedPom = if (requestedSelector == null) canonicalRepo.resolve("pom.xml").toRealPath() else {
            val expected = canonicalRepo.resolve(requestedSelector).normalize().resolve("pom.xml")
            ordered.singleOrNull { it == runCatching { expected.toRealPath() }.getOrNull() }
                ?: throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "requested Maven reactor module is absent or ambiguous")
        }
        return ReactorSelection(
            projectPath = requested,
            projectDir = selectedPom.parent,
            selector = requestedSelector,
            poms = ordered,
        )
    }

    private fun selectedSourceSet(compilation: String?): String {
        val selected = compilation ?: ":/main"
        return if ('/' in selected) selected.substringAfterLast('/')
        else if (selected.contains("test", ignoreCase = true)) "test" else "main"
    }

    private fun configuredRoots(project: Element, configuration: Element?, repo: Path, sourceSet: String): List<Path> {
        val build = project.directChild("build")
        val configured = if (sourceSet == "test") build?.directText("testSourceDirectory")
        else build?.directText("sourceDirectory")
        val fallback = if (sourceSet == "test") "src/test/kotlin" else "src/main/kotlin"
        val configuredPluginRoots = configuration
            ?.directChild(if (sourceSet == "test") "testSourceDirs" else "sourceDirs")
            ?.directChildren("source")
            ?.mapNotNull { it.textValue() }
            .orEmpty()
        return (listOf(configured.orEmpty().ifBlank { fallback }) + configuredPluginRoots)
            .map { value ->
                val expanded = value
                    .replace("${'$'}{project.basedir}", repo.toString())
                    .replace("${'$'}{basedir}", repo.toString())
                val path = Path.of(expanded)
                if (path.isAbsolute) path.normalize() else repo.resolve(path).normalize()
            }
            .distinct()
    }

    private fun kotlinSources(repositoryRoot: Path, root: Path): List<Path> {
        val canonicalRepository = repositoryRoot.toRealPath()
        val normalized = root.toAbsolutePath().normalize()
        if (!normalized.startsWith(canonicalRepository)) {
            throw WorkerFailure(
                "UNSUPPORTED_PROJECT_CONFIGURATION",
                "Maven Kotlin source root is outside the repository",
            )
        }
        var current = canonicalRepository
        canonicalRepository.relativize(normalized).forEach { component ->
            current = current.resolve(component)
            if (!Files.exists(current, java.nio.file.LinkOption.NOFOLLOW_LINKS)) return emptyList()
            if (Files.isSymbolicLink(current) ||
                !Files.isDirectory(current, java.nio.file.LinkOption.NOFOLLOW_LINKS)
            ) {
                throw WorkerFailure(
                    "UNSUPPORTED_PROJECT_CONFIGURATION",
                    "Maven Kotlin source root contains a non-directory or symbolic component",
                )
            }
        }
        return Files.walk(normalized).use { paths ->
            paths.filter {
                Files.isRegularFile(it, java.nio.file.LinkOption.NOFOLLOW_LINKS) && it.extension == "kt"
            }.sorted().toList()
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
