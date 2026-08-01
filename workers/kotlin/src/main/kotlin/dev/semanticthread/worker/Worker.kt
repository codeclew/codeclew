@file:OptIn(org.jetbrains.kotlin.K1Deprecation::class, org.jetbrains.kotlin.config.CompilerConfiguration.Internals::class)

package dev.semanticthread.worker

import kotlinx.serialization.json.*
import org.jetbrains.kotlin.cli.jvm.compiler.EnvironmentConfigFiles
import org.jetbrains.kotlin.cli.jvm.compiler.KotlinCoreEnvironment
import org.jetbrains.kotlin.com.intellij.openapi.util.Disposer
import org.jetbrains.kotlin.com.intellij.openapi.application.ApplicationManager
import org.jetbrains.kotlin.com.intellij.openapi.extensions.ExtensionPoint
import org.jetbrains.kotlin.com.intellij.psi.PsiElement
import org.jetbrains.kotlin.com.intellij.psi.PsiErrorElement
import org.jetbrains.kotlin.com.intellij.psi.impl.source.tree.TreeCopyHandler
import org.jetbrains.kotlin.com.intellij.psi.util.PsiTreeUtil
import org.jetbrains.kotlin.config.CompilerConfiguration
import org.jetbrains.kotlin.psi.*
import java.io.File
import java.nio.file.Files
import java.nio.file.Path
import java.security.MessageDigest
import kotlin.io.path.*

internal class Worker : AutoCloseable {
    private val disposable = Disposer.newDisposable("semantic-thread-worker")
    private val environment = KotlinCoreEnvironment.createForProduction(
        disposable, CompilerConfiguration(), EnvironmentConfigFiles.JVM_CONFIG_FILES
    )
    init {
        val area = ApplicationManager.getApplication().extensionArea
        if (!area.hasExtensionPoint(TreeCopyHandler.EP_NAME)) {
            area.registerExtensionPoint(TreeCopyHandler.EP_NAME.name, TreeCopyHandler::class.java.name, ExtensionPoint.Kind.INTERFACE)
        }
    }
    private val factory = KtPsiFactory(environment.project, markGenerated = false)
    private val json = Json { ignoreUnknownKeys = false; explicitNulls = false }
    private val analysisCache = mutableMapOf<String, K2Analysis>()
    private val gradleModelCache = mutableMapOf<String, JsonObject>()

    fun handle(kind: Int, payload: ByteArray): String {
        val request = if (payload.isEmpty()) buildJsonObject {} else json.parseToJsonElement(payload.decodeToString()).jsonObject
        return when (kind) {
            2 -> inspect(Path.of(request.requiredString("repo")), request["compilation"]?.takeUnless { it is JsonNull }?.jsonPrimitive?.content).toString()
            3 -> index(Path.of(request.requiredString("repo")), request["compilation"]?.takeUnless { it is JsonNull }?.jsonPrimitive?.content).toString()
            4 -> resolveSymbol(Path.of(request.requiredString("repo")), request.requiredString("symbol")).toString()
            5 -> resolveExpression(Path.of(request.requiredString("repo")), request.requiredString("file"), request.requiredInt("offset")).toString()
            6 -> localGraph(Path.of(request.requiredString("repo")), request.requiredString("symbol")).toString()
            7 -> applyEdit(request).toString()
            8 -> validateCandidate(request).toString()
            else -> error("unsupported request kind $kind")
        }
    }

    private fun inspect(requestedRepo: Path, compilation: String?): JsonObject {
        require(requestedRepo.isDirectory()) { "repository does not exist: $requestedRepo" }
        val repo = requestedRepo.toRealPath()
        val gradle = cachedGradleModel(repo, compilation)
        val modelFiles = projectModelFiles(repo)
        val sourceFiles = gradle["sourceFiles"]?.jsonArray?.map { Path.of(it.jsonPrimitive.content) }.orEmpty()
        val sourceRoots = sourceFiles.mapNotNull { sourceRoot(repo, it) }.distinct().sorted()
        val generatedRoots = sourceFiles.filter { it.normalize().startsWith(repo.resolve("build/generated").normalize()) }.map { repo.relativize(it.parent).invariantSeparatorsPathString }.distinct().sorted()
        val classpath = gradle["classpath"]?.jsonArray?.map { normalizeArtifact(repo, Path.of(it.jsonPrimitive.content)) }.orEmpty().sorted()
        val plugins = gradle["compilerPlugins"]?.jsonArray?.map { normalizeArtifact(repo, Path.of(it.jsonPrimitive.content)) }.orEmpty().sorted()
        val normalized = buildJsonObject {
            put("module", gradle["projectPath"] ?: JsonPrimitive(":")); put("sourceSet", compilation?.substringAfterLast('/') ?: "main")
            putJsonArray("sourceRoots") { sourceRoots.forEach(::add) }; putJsonArray("generatedSourceRoots") { generatedRoots.forEach(::add) }
            putJsonArray("compileClasspath") { classpath.forEach(::add) }; putJsonArray("friendPaths") { gradle["friendPaths"]?.jsonArray?.map { normalizeArtifact(repo, Path.of(it.jsonPrimitive.content)) }?.sorted()?.forEach(::add) }
            put("languageVersion", gradle["languageVersion"]?.takeUnless { it is JsonNull } ?: JsonPrimitive("2.4")); put("apiVersion", gradle["apiVersion"]?.takeUnless { it is JsonNull } ?: JsonPrimitive("2.4")); put("jvmTarget", gradle["jvmTarget"]?.takeUnless { it is JsonNull }?.jsonPrimitive?.content?.removePrefix("JVM_") ?: "21")
            putJsonArray("freeCompilerArguments") { gradle["freeCompilerArguments"]?.jsonArray?.sortedBy { it.toString() }?.forEach(::add) }
            putJsonArray("optIns") { gradle["optIns"]?.jsonArray?.sortedBy { it.toString() }?.forEach(::add) }; putJsonArray("compilerPlugins") { plugins.forEach(::add) }
            put("compileTask", gradle["compileTask"] ?: JsonPrimitive(":compileKotlin")); putJsonArray("testTasks") { gradle["tasks"]?.jsonArray?.map { it.jsonPrimitive.content }?.filter { it == "test" || it.endsWith("Test") }?.sorted()?.forEach(::add) }
            put("gradleVersion", gradle["gradleVersion"] ?: JsonPrimitive("unknown")); put("jdkHome", gradle["jdkHome"] ?: JsonPrimitive(System.getProperty("java.home")))
            putJsonArray("modelInputs") { modelFiles.map { buildJsonObject { put("path", repo.relativize(it).invariantSeparatorsPathString); put("hash", sha(it.readBytes())) } }.sortedBy { it.toString() }.forEach(::add) }
        }
        val modelHash = sha(normalized.toString().toByteArray())
        return buildJsonObject {
            put("schema", "semantic-project/0.1"); put("projectPath", repo.toAbsolutePath().normalize().toString())
            normalized.forEach { (key, value) -> put(key, value) }; put("compilerVersion", "2.4.10"); put("jdk", 21)
            put("projectModelHash", modelHash)
        }
    }

    private fun gradleModel(repo: Path, compilation: String?): JsonObject {
        val wrapper = repo.resolve("gradlew"); if (!wrapper.isRegularFile()) throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "Gradle Wrapper is required")
        val script = Files.createTempFile("semantic-thread-model", ".init.gradle")
        try {
            val resource = Worker::class.java.getResourceAsStream("/semantic-thread-model.init.gradle") ?: error("project model init script missing")
            script.writeBytes(resource.use { it.readBytes() })
            val selected = compilation ?: ":/main"
            val projectPath = if ('/' in selected) selected.substringBeforeLast('/').ifBlank { ":" } else selected.substringBeforeLast(':', ":").ifBlank { ":" }
            val sourceSet = if ('/' in selected) selected.substringAfterLast('/') else if (selected.contains("compileTest", true)) "test" else "main"
            val compileTask = if ('/' in selected) if (sourceSet == "main") "compileKotlin" else "compile${sourceSet.replaceFirstChar(Char::uppercase)}Kotlin" else selected.substringAfterLast(':').ifBlank { "compileKotlin" }
            val modelTask = if (projectPath == ":") ":semanticThreadModel" else "$projectPath:semanticThreadModel"
            val process = ProcessBuilder(wrapper.toString(), "-p", repo.toString(), "--no-daemon", "--quiet", "-I", script.toString(), "-Dsemantic.thread.compileTask=$compileTask", modelTask)
                .directory(repo.toFile()).redirectErrorStream(true).start()
            val output = process.inputStream.bufferedReader().readText(); val status = process.waitFor()
            if (status != 0) throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "Gradle model extraction failed: ${output.takeLast(2000)}")
            val line = output.lineSequence().lastOrNull { it.startsWith("__SEMANTIC_THREAD_MODEL__") }
                ?: throw WorkerFailure("UNSUPPORTED_PROJECT_CONFIGURATION", "Gradle model marker missing")
            return json.parseToJsonElement(line.removePrefix("__SEMANTIC_THREAD_MODEL__")).jsonObject
        } finally { Files.deleteIfExists(script) }
    }

    private fun cachedGradleModel(repo: Path, compilation: String?): JsonObject {
        val canonicalRepo = repo.toRealPath()
        val inputHash = sha(projectModelFiles(canonicalRepo).flatMap { file -> (canonicalRepo.relativize(file).invariantSeparatorsPathString + ":" + sha(file.readBytes()) + "\n").toByteArray().asIterable() }.toByteArray())
        val key = "$canonicalRepo|${compilation ?: ":/main"}|$inputHash"
        return gradleModelCache.getOrPut(key) { gradleModel(canonicalRepo, compilation) }
    }

    private fun sourceRoot(repo: Path, file: Path): String? {
        val normalized = file.normalize(); val markers = listOf("/src/main/kotlin/", "/src/test/kotlin/")
        val text = normalized.invariantSeparatorsPathString
        val marker = markers.firstOrNull(text::contains) ?: return normalized.parent?.let { repo.relativize(it).invariantSeparatorsPathString }
        val root = Path.of(text.substringBefore(marker) + marker.removeSuffix("/")); return if (root.startsWith(repo)) repo.relativize(root).invariantSeparatorsPathString else root.invariantSeparatorsPathString
    }

    private fun normalizeArtifact(repo: Path, path: Path): String {
        val normalized = path.toAbsolutePath().normalize()
        return if (normalized.startsWith(repo.toAbsolutePath().normalize())) "repo:${repo.relativize(normalized).invariantSeparatorsPathString}:${if (normalized.isRegularFile()) sha(normalized.readBytes()) else "directory"}"
        else "artifact:${normalized.fileName}:${if (normalized.isRegularFile()) sha(normalized.readBytes()) else "directory"}"
    }

    private fun projectModelFiles(repo: Path): List<Path> = Files.walk(repo).use { paths ->
        paths.filter { it.isRegularFile() }.filter {
            val relative = repo.relativize(it).invariantSeparatorsPathString
            if (relative.split('/').any { part -> part == "build" || part == ".gradle" || part == ".git" }) return@filter false
            val n = it.fileName.toString()
            n == "settings.gradle" || n == "settings.gradle.kts" || n == "build.gradle" || n == "build.gradle.kts" ||
                n == "gradle.properties" || n == "libs.versions.toml" || n == "gradle-wrapper.properties" || n == "gradle-wrapper.jar" ||
                relative.startsWith("buildSrc/") || relative.startsWith("build-logic/") || relative.startsWith("gradle/")
        }.sorted().toList()
    }

    private fun sourceFiles(repo: Path): List<Path> = Files.walk(repo).use { paths ->
        paths.filter { it.isRegularFile() && it.extension == "kt" && it.invariantSeparatorsPathString.contains("/src/main/kotlin/") && !it.invariantSeparatorsPathString.contains("/build/") }
            .sorted().toList()
    }

    private fun analyzeWithK2(repo: Path, overrides: Map<String, String> = emptyMap(), compilation: String = ":/main"): K2Analysis {
        val analysisRepo = repo.toRealPath()
        val model = cachedGradleModel(analysisRepo, compilation)
        val sources = model["sourceFiles"]?.jsonArray?.map { Path.of(it.jsonPrimitive.content) }?.filter(Path::isRegularFile)?.sorted().orEmpty()
        val cacheMaterial = buildString {
            sources.forEach { source ->
                val relative = analysisRepo.relativize(source.toRealPath()).invariantSeparatorsPathString
                append(relative).append('\u0000').append(overrides[relative] ?: source.readText()).append('\u0000')
            }
            projectModelFiles(analysisRepo).forEach { input -> append(analysisRepo.relativize(input).invariantSeparatorsPathString).append(':').append(sha(input.readBytes())).append('\u0000') }
        }
        val cacheKey = sha(cacheMaterial.toByteArray())
        if (overrides.isEmpty()) analysisCache["$analysisRepo|$compilation|$cacheKey"]?.let { return it }
        val temp = Files.createTempDirectory("semantic-thread-k2")
        try {
            val sourceArgs = sources.map { original ->
                val relative = analysisRepo.relativize(original.toRealPath()).invariantSeparatorsPathString
                val replacement = overrides[relative]
                if (replacement == null) original else temp.resolve("sources").resolve(relative).also { it.parent.createDirectories(); it.writeText(replacement) }
            }
            val factsFile = temp.resolve("facts.jsonl"); val outputDir = temp.resolve("classes").also(Path::createDirectories)
            val plugin = Path.of(FirFactsCompilerPluginRegistrar::class.java.protectionDomain.codeSource.location.toURI())
            val classpath = model["classpath"]?.jsonArray?.joinToString(File.pathSeparator) { it.jsonPrimitive.content }.orEmpty()
            val java = Path.of(System.getProperty("java.home"), "bin", "java").toString()
            val command = mutableListOf(java, "-cp", System.getProperty("java.class.path"), "org.jetbrains.kotlin.cli.jvm.K2JVMCompiler", "-d", outputDir.toString(), "-classpath", classpath, "-no-stdlib", "-no-reflect", "-jdk-home", model["jdkHome"]!!.jsonPrimitive.content, "-jvm-target", model["jvmTarget"]?.jsonPrimitive?.content?.removePrefix("JVM_") ?: "21", "-Xplugin=$plugin", "-P", "plugin:$FACTS_PLUGIN_ID:output=$factsFile")
            command += sourceArgs.map(Path::toString)
            val process = ProcessBuilder(command).directory(analysisRepo.toFile()).redirectErrorStream(true).start()
            val output = process.inputStream.bufferedReader().readText(); val status = process.waitFor()
            val diagnostics = output.lineSequence().filter { it.isNotBlank() }.map { line ->
                buildJsonObject { put("severity", when { "error:" in line.lowercase() || line.startsWith("e:") -> "ERROR"; "warning:" in line.lowercase() || line.startsWith("w:") -> "WARNING"; else -> "INFO" }); put("message", line) }
            }.toList()
            val facts = if (factsFile.isRegularFile()) factsFile.readLines().filter(String::isNotBlank).map { json.parseToJsonElement(it).jsonObject } else emptyList()
            val result = K2Analysis(status == 0, facts.sortedBy { it.toString() }, diagnostics)
            if (overrides.isEmpty()) analysisCache["$analysisRepo|$compilation|$cacheKey"] = result
            return result
        } finally { temp.toFile().deleteRecursively() }
    }

    private fun parse(path: Path): KtFile = factory.createFile(path.fileName.toString(), path.readText())

    private fun index(repo: Path, compilation: String?): JsonObject {
        val selected = compilation ?: ":/main"
        val model = cachedGradleModel(repo, selected)
        val selectedFiles = model["sourceFiles"]?.jsonArray?.map { Path.of(it.jsonPrimitive.content) }?.filter(Path::isRegularFile)?.sorted().orEmpty()
        val analysis = analyzeWithK2(repo, compilation = selected)
        val files = selectedFiles.map { path ->
            val bytes = path.readBytes(); val kt = parse(path); val pkg = kt.packageFqName.asString()
            val declarations = PsiTreeUtil.collectElementsOfType(kt, KtNamedDeclaration::class.java)
                .filter { it is KtNamedFunction || it is KtClassOrObject || it is KtProperty }
                .sortedBy { it.textOffset }.map { declarationJson(repo, path, pkg, it) }
            buildJsonObject {
                put("path", repo.relativize(path).invariantSeparatorsPathString); put("contentHash", sha(bytes)); put("package", pkg)
                put("lineEnding", if (bytes.decodeToString().contains("\r\n")) "CRLF" else "LF"); put("bom", bytes.take(3) == listOf(0xef.toByte(), 0xbb.toByte(), 0xbf.toByte()))
                putJsonArray("imports") { kt.importDirectives.map { it.importPath?.pathStr.orEmpty() }.sorted().forEach(::add) }
                putJsonArray("declarations") { declarations.forEach(::add) }
                putJsonArray("semanticFacts") { fileFacts(repo, path, analysis).forEach(::add) }
            }
        }
        val canonical = JsonArray(files)
        return buildJsonObject {
            put("schema", "semantic-index/0.1"); put("compilation", selected); put("files", canonical); put("indexHash", sha(canonical.toString().toByteArray()))
            put("k2Validated", analysis.valid); putJsonArray("diagnostics") { analysis.diagnostics.forEach(::add) }
        }
    }

    private fun declarationJson(repo: Path, path: Path, pkg: String, declaration: KtNamedDeclaration): JsonObject {
        val symbol = symbolId(pkg, declaration); val signature = when (declaration) {
            is KtNamedFunction -> declaration.text.substringBefore(declaration.bodyExpression?.text ?: "")
            else -> declaration.name.orEmpty()
        }
        return buildJsonObject {
            put("symbolId", symbol); put("name", declaration.name.orEmpty()); put("kind", declaration::class.simpleName ?: "KtDeclaration")
            put("file", repo.relativize(path).invariantSeparatorsPathString); put("rangeStart", declaration.textRange.startOffset); put("rangeEnd", declaration.textRange.endOffset)
            put("signatureHash", sha(signature.toByteArray())); put("bodyHash", sha((declaration as? KtDeclarationWithBody)?.bodyExpression?.text?.toByteArray() ?: byteArrayOf()))
        }
    }

    private fun symbolId(pkg: String, declaration: KtNamedDeclaration): String {
        val owners = generateSequence(declaration.parent) { it.parent }.filterIsInstance<KtClassOrObject>().toList().asReversed().mapNotNull { it.name }
        val base = (listOf(pkg).filter { it.isNotBlank() } + owners + listOf(declaration.name ?: "<anonymous>")).joinToString(".")
        if (declaration !is KtNamedFunction) return base
        val receiver = declaration.receiverTypeReference?.text?.let { "$it." }.orEmpty()
        val params = declaration.valueParameters.joinToString(",") { it.typeReference?.text ?: "?" }
        return "$receiver$base($params)"
    }

    private fun resolveSymbol(repo: Path, query: String): JsonObject {
        val matches = mutableListOf<JsonObject>()
        sourceFiles(repo).forEach { path ->
            val kt = parse(path); val pkg = kt.packageFqName.asString()
            PsiTreeUtil.collectElementsOfType(kt, KtNamedFunction::class.java).forEach { fn ->
                val symbol = symbolId(pkg, fn); val display = symbol.substringBefore('(')
                if (query == symbol || query == display) matches += declarationJson(repo, path, pkg, fn)
            }
        }
        if (matches.isEmpty()) throw WorkerFailure("SYMBOL_NOT_FOUND", "symbol not found: $query")
        if (matches.size > 1) throw WorkerFailure("AMBIGUOUS_SYMBOL", "symbol is ambiguous: $query")
        val declaration = matches.single()
        val file = declaration["file"]!!.jsonPrimitive.content
        val (path, kt, fn) = findFunction(repo, query)
        val analysis = analyzeWithK2(repo)
        val semanticFacts = fileFacts(repo, path, analysis).filter { fact ->
            val start = fact["start"]?.jsonPrimitive?.intOrNull ?: -1
            val end = fact["end"]?.jsonPrimitive?.intOrNull ?: -1
            start >= fn.textRange.startOffset && end <= fn.textRange.endOffset
        }
        return buildJsonObject {
            put("schema", "semantic-symbol/0.1"); put("declaration", declaration)
            fn.bodyExpression?.let { put("bodyAnchor", anchor(file, symbolId(kt.packageFqName.asString(), fn), it, kt.text)) }
            putJsonArray("references") { fn.bodyExpression?.let(::usedNames).orEmpty().sorted().forEach(::add) }
            putJsonArray("calls") { PsiTreeUtil.collectElementsOfType(fn, KtCallExpression::class.java).map { it.calleeExpression?.text.orEmpty() }.sorted().forEach(::add) }
            putJsonArray("declaredTypes") { (fn.valueParameters.mapNotNull { it.typeReference?.text } + listOfNotNull(fn.typeReference?.text)).sorted().forEach(::add) }
            putJsonArray("semanticFacts") { semanticFacts.forEach(::add) }
            putJsonArray("resolvedCalls") { semanticFacts.filter { "FunctionCall" in (it["kind"]?.jsonPrimitive?.content.orEmpty()) }.forEach(::add) }
            put("semanticSummaryHash", sha(buildJsonObject { put("bodyHash", declaration["bodyHash"]!!); putJsonArray("facts") { semanticFacts.map(::semanticSignature).forEach(::add) } }.toString().toByteArray()))
            put("k2Validated", analysis.valid); putJsonArray("diagnostics") { analysis.diagnostics.forEach(::add) }
        }
    }

    private fun findFunction(repo: Path, query: String): Triple<Path, KtFile, KtNamedFunction> {
        val matches = mutableListOf<Triple<Path, KtFile, KtNamedFunction>>()
        sourceFiles(repo).forEach { path ->
            val kt = parse(path); val pkg = kt.packageFqName.asString()
            PsiTreeUtil.collectElementsOfType(kt, KtNamedFunction::class.java).forEach { fn ->
                val symbol = symbolId(pkg, fn)
                if (query == symbol || query == symbol.substringBefore('(')) matches += Triple(path, kt, fn)
            }
        }
        if (matches.isEmpty()) throw WorkerFailure("SYMBOL_NOT_FOUND", "symbol not found: $query")
        if (matches.size > 1) throw WorkerFailure("AMBIGUOUS_SYMBOL", "symbol is ambiguous: $query")
        return matches.single()
    }

    private fun resolveExpression(repo: Path, relative: String, offset: Int): JsonObject {
        val path = repo.resolve(relative).normalize(); require(path.startsWith(repo.normalize())) { "file escapes repository" }
        val kt = parse(path); val leaf = kt.findElementAt(offset) ?: throw WorkerFailure("EXPRESSION_NOT_FOUND", "no element at offset $offset")
        val expression = generateSequence(leaf as PsiElement?) { it.parent }.filterIsInstance<KtExpression>().firstOrNull { isGraphExpression(it) }
            ?: throw WorkerFailure("EXPRESSION_NOT_FOUND", "no expression at offset $offset")
        val owner = generateSequence(expression.parent) { it.parent }.filterIsInstance<KtNamedFunction>().firstOrNull()
            ?: throw WorkerFailure("EXPRESSION_NOT_FOUND", "expression is outside a function")
        val symbol = symbolId(kt.packageFqName.asString(), owner)
        return buildJsonObject { put("schema", "semantic-anchor/0.1"); put("anchor", anchor(relative, symbol, expression, kt.text)) }
    }

    private fun anchor(file: String, owner: String, node: PsiElement, source: String): JsonObject {
        val start = node.textRange.startOffset; val end = node.textRange.endOffset
        val ancestor = generateSequence(node.parent) { it.parent }.takeWhile { it !is KtNamedFunction }.map { it::class.simpleName.orEmpty() }.toList()
        val sameKind = generateSequence(node.parent) { it.parent }.firstOrNull()?.children?.filter { it::class == node::class } ?: emptyList()
        return buildJsonObject {
            put("fileId", file); put("ownerSymbolId", owner); put("syntaxKind", node::class.simpleName ?: "PsiElement")
            put("normalizedTokenHash", sha(normalizeTokens(node.text).toByteArray())); put("ancestorPathHash", sha(ancestor.joinToString("/").toByteArray()))
            put("localOrdinal", sameKind.indexOf(node).coerceAtLeast(0)); put("leftContextHash", sha(source.substring(maxOf(0, start - 64), start).toByteArray()))
            put("rightContextHash", sha(source.substring(end, minOf(source.length, end + 64)).toByteArray())); put("exactTextHash", sha(node.text.toByteArray()))
            putJsonArray("rangeHint") { add(start); add(end) }; put("sourceText", node.text)
            generateSequence(node as PsiElement?) { it.parent }.filterIsInstance<KtNamedFunction>().firstOrNull()?.let { function ->
                val signature = function.text.substringBefore(function.bodyExpression?.text ?: "")
                put("ownerSignatureHash", sha(signature.toByteArray()))
            }
            put("anchorId", "anchor:" + sha("$file|$owner|${node::class.simpleName}|${normalizeTokens(node.text)}|${ancestor.joinToString("/")}".toByteArray()).removePrefix("sha256:"))
        }
    }

    private fun localGraph(repo: Path, query: String): JsonObject {
        val (path, kt, fn) = findFunction(repo, query); val relative = repo.relativize(path).invariantSeparatorsPathString
        val owner = symbolId(kt.packageFqName.asString(), fn)
        val analysis = analyzeWithK2(repo)
        val firCfg = cfgRecords(repo, path, analysis).firstOrNull {
            it["start"]?.jsonPrimitive?.intOrNull == fn.textRange.startOffset && it["end"]?.jsonPrimitive?.intOrNull == fn.textRange.endOffset
        }
        val graph = if (firCfg == null) LocalCfgBuilder(relative, owner, kt.text).build(fn) else normalizeFirCfg(relative, owner, kt, fn, firCfg)
        return enrichGraph(repo, path, graph, analysis)
    }

    private fun fileFacts(repo: Path, path: Path, analysis: K2Analysis): List<JsonObject> {
        val absolute = path.toAbsolutePath().normalize().invariantSeparatorsPathString
        val relative = repo.relativize(path).invariantSeparatorsPathString
        return analysis.facts.filter { fact ->
            if (fact["recordType"]?.jsonPrimitive?.content != "SEMANTIC_FACT") return@filter false
            val value = fact["file"]?.jsonPrimitive?.content?.replace('\\', '/') ?: return@filter false
            value == absolute || value == relative || value.endsWith("/$relative")
        }.sortedWith(compareBy({ it["start"]?.jsonPrimitive?.intOrNull ?: -1 }, { it["end"]?.jsonPrimitive?.intOrNull ?: -1 }, { it.toString() }))
    }

    private fun cfgRecords(repo: Path, path: Path, analysis: K2Analysis): List<JsonObject> {
        val absolute = path.toAbsolutePath().normalize().invariantSeparatorsPathString
        val relative = repo.relativize(path).invariantSeparatorsPathString
        return analysis.facts.filter { fact ->
            if (fact["recordType"]?.jsonPrimitive?.content != "FIR_CFG") return@filter false
            val value = fact["file"]?.jsonPrimitive?.content?.replace('\\', '/') ?: return@filter false
            value == absolute || value == relative || value.endsWith("/$relative")
        }
    }

    private fun normalizeFirCfg(file: String, owner: String, kt: KtFile, fn: KtNamedFunction, cfg: JsonObject): JsonObject {
        val rawNodes = cfg["nodes"]!!.jsonArray
        val functionLocals = (fn.valueParameters.mapNotNull { it.name } + PsiTreeUtil.collectElementsOfType(fn.bodyExpression ?: fn, KtProperty::class.java).filter { property -> generateSequence(property.parent) { it.parent }.takeWhile { it != fn }.none { it is KtLambdaExpression } }.mapNotNull { it.name }).toSet()
        val hasClassReceiver = generateSequence(fn.parent) { it.parent }.any { it is KtClassOrObject }
        val normalizedNodes = rawNodes.map { value ->
            val raw = value.jsonObject; val rawId = raw["id"]!!.jsonPrimitive.int
            val start = raw["start"]?.jsonPrimitive?.intOrNull; val end = raw["end"]?.jsonPrimitive?.intOrNull
            val psi = if (start != null && end != null && start >= fn.textRange.startOffset && end <= fn.textRange.endOffset) {
                generateSequence(kt.findElementAt(start) as PsiElement?) { it.parent }.firstOrNull { it.textRange.startOffset == start && it.textRange.endOffset == end }
            } else null
            val rawKind = raw["kind"]!!.jsonPrimitive.content
            val expression = (psi as? KtExpression)?.takeUnless { it is KtNamedFunction || it is KtBlockExpression || it is KtWhenExpression || it is KtIfExpression || it is KtLoopExpression }
            val kind = when {
                "FunctionEnter" in rawKind -> "ENTRY"
                "FunctionExit" in rawKind -> "EXIT"
                rawKind == "FunctionCallEnterNode" || rawKind == "FunctionCallExitNode" || expression is KtCallExpression -> "CALL"
                "Enter" in rawKind -> "ENTRY"
                "Exit" in rawKind -> "EXPRESSION"
                "Throw" in rawKind -> "THROW"
                "Jump" in rawKind || "Return" in rawKind -> "RETURN"
                "Condition" in rawKind || "Branch" in rawKind || "When" in rawKind -> "BRANCH"
                "Loop" in rawKind -> "LOOP"
                expression != null -> normalizedKind(expression)
                else -> "EXPRESSION"
            }
            val defines = when {
                "VariableDeclarationExit" in rawKind -> expression?.let(::definedName)
                "VariableAssignment" in rawKind -> expression?.let(::definedName)
                else -> null
            }
            var uses = when {
                "QualifiedAccess" in rawKind -> expression?.let(::usedNames).orEmpty()
                "FunctionCallExit" in rawKind || "VariableAssignment" in rawKind || "VariableDeclarationExit" in rawKind -> expression?.let(::normalizedUses).orEmpty()
                "ConditionExit" in rawKind -> expression?.let(::usedNames).orEmpty()
                "Jump" in rawKind || "Throw" in rawKind -> expression?.let(::usedNames).orEmpty()
                else -> emptyList()
            }
            if (uses.isEmpty() && start != null && end != null && "ConditionExit" in rawKind) {
                val conditionText = kt.text.substring(start.coerceAtLeast(0), end.coerceAtMost(kt.text.length)).trim()
                if (conditionText.matches(Regex("[A-Za-z_][A-Za-z0-9_]*"))) uses = listOf(conditionText)
            }
            graphNode("fir:$rawId", kind, defines, psi?.let { anchor(file, owner, it, kt.text) }, uses).let { node ->
                buildJsonObject { node.forEach { (key, item) -> put(key, item) }; putJsonObject("attributes") {
                    put("firNodeKind", rawKind); put("firDead", raw["dead"] ?: JsonPrimitive(false)); put("analysis", "K2_FIR_CFG")
                    if (hasClassReceiver && ((defines != null && defines !in functionLocals) || uses.any { it !in functionLocals })) {
                        putJsonArray("effects") {
                            if (uses.any { it !in functionLocals }) add("READ_STATE")
                            if (defines != null && defines !in functionLocals) add("WRITE_STATE")
                        }
                        val property = defines?.takeIf { it !in functionLocals } ?: uses.first { it !in functionLocals }
                        put("memoryKind", "THIS_PROPERTY"); put("memoryLocation", "THIS_PROPERTY:$owner#$property")
                    }
                } }
            }
        }
        val rawKinds = rawNodes.associate { it.jsonObject["id"]!!.jsonPrimitive.int to it.jsonObject["kind"]!!.jsonPrimitive.content }
        val rawEdges = cfg["edges"]!!.jsonArray.map { value ->
            val raw = value.jsonObject; val label = raw["label"]!!.jsonPrimitive.content; val edgeKind = raw["edgeKind"]!!.jsonPrimitive.content
            val from = raw["from"]!!.jsonPrimitive.int; val to = raw["to"]!!.jsonPrimitive.int
            val fromKind = rawKinds[from].orEmpty(); val toKind = rawKinds[to].orEmpty()
            val kind = when {
                "True" in label -> "CFG_TRUE"
                "False" in label -> "CFG_FALSE"
                "Exception" in label || "Exception" in edgeKind -> "CFG_EXCEPTION"
                "ConditionExit" in fromKind && ("SyntheticElse" in toKind || "Exit" in toKind) -> "CFG_FALSE"
                "ConditionExit" in fromKind -> "CFG_TRUE"
                "Throw" in fromKind -> "THROW"
                "Jump" in fromKind && "FunctionExit" in toKind -> "RETURN"
                "Backward" in edgeKind || to <= from && "Loop" in toKind -> "CFG_BACK"
                else -> "CFG_NORMAL"
            }
            edge("fir:$from", "fir:$to", kind)
        }
        val entry = rawKinds.entries.firstOrNull { "FunctionEnter" in it.value }?.key
        val entryEdges = if (entry == null || fn.valueParameters.isEmpty()) emptyList() else rawEdges.filter { it["from"]?.jsonPrimitive?.content == "fir:$entry" }
        val normalizedEdges = rawEdges.filterNot { it in entryEdges }.toMutableList()
        val parameterNodes = fn.valueParameters.mapIndexed { index, parameter -> graphNode("param:$index", "PARAMETER", parameter.name, anchor(file, owner, parameter, kt.text)) }
        if (entry != null && parameterNodes.isNotEmpty()) {
            normalizedEdges += edge("fir:$entry", "param:0", "CFG_NORMAL")
            for (index in 0 until parameterNodes.lastIndex) normalizedEdges += edge("param:$index", "param:${index + 1}", "CFG_NORMAL")
            entryEdges.forEach { original -> normalizedEdges += edge("param:${parameterNodes.lastIndex}", original["to"]!!.jsonPrimitive.content, original["kind"]!!.jsonPrimitive.content) }
        }
        val outerNames = functionLocals + fn.valueParameters.mapNotNull { it.name }
        val captureNodes = mutableListOf<JsonObject>()
        PsiTreeUtil.collectElementsOfType(fn.bodyExpression ?: fn, KtLambdaExpression::class.java).sortedBy { it.textOffset }.forEachIndexed { lambdaIndex, lambda ->
            val localNames = lambda.valueParameters.mapNotNull { it.name }.toSet() + PsiTreeUtil.collectElementsOfType(lambda, KtProperty::class.java).mapNotNull { it.name }
            val captured = usedNames(lambda).filter { it in outerNames && it !in localNames }.distinct().sorted()
            captured.forEach { name ->
                val id = "capture:$lambdaIndex:$name"
                captureNodes += graphNode(id, "CAPTURE", null, anchor(file, owner, lambda, kt.text), listOf(name))
                val host = normalizedNodes.filter { node ->
                    node["kind"]?.jsonPrimitive?.content == "CALL" && node["origin"]?.jsonObject?.get("rangeHint")?.jsonArray?.let { range ->
                        (range[0].jsonPrimitive.int <= lambda.textRange.startOffset && range[1].jsonPrimitive.int >= lambda.textRange.endOffset)
                    } == true
                }.minByOrNull { node -> node["origin"]!!.jsonObject["rangeHint"]!!.jsonArray.let { it[1].jsonPrimitive.int - it[0].jsonPrimitive.int } }
                host?.get("id")?.jsonPrimitive?.content?.let { normalizedEdges += edge(it, id, "CAPTURE") }
            }
        }
        return buildJsonObject {
            put("schema", "local-cfg/0.1"); put("symbol", owner); put("file", file); put("graphSource", "K2_FIR_CFG")
            putJsonArray("nodes") { (normalizedNodes + parameterNodes + captureNodes).forEach(::add) }; putJsonArray("edges") { normalizedEdges.forEach(::add) }; putJsonArray("boundaries") {}
            put("firCfgHash", sha(cfg.toString().toByteArray()))
        }
    }

    private fun enrichGraph(repo: Path, path: Path, graph: JsonObject, analysis: K2Analysis): JsonObject {
        val facts = fileFacts(repo, path, analysis)
        val nodes = graph["nodes"]!!.jsonArray.map { raw ->
            val node = raw.jsonObject
            val range = node["origin"]?.jsonObject?.get("rangeHint")?.jsonArray
            val start = range?.getOrNull(0)?.jsonPrimitive?.intOrNull
            val end = range?.getOrNull(1)?.jsonPrimitive?.intOrNull
            val candidates = if (start == null || end == null) emptyList() else facts.filter {
                val fs = it["start"]?.jsonPrimitive?.intOrNull ?: -1; val fe = it["end"]?.jsonPrimitive?.intOrNull ?: -1
                fs >= start && fe <= end || fs <= start && fe >= end
            }
            val firKind = node["attributes"]?.jsonObject?.get("firNodeKind")?.jsonPrimitive?.content.orEmpty()
            val preferred = candidates.filter { fact ->
                val factKind = fact["kind"]?.jsonPrimitive?.content.orEmpty()
                when {
                    "FunctionCall" in firKind -> "FunctionCall" in factKind
                    "QualifiedAccess" in firKind -> "PropertyAccess" in factKind || "QualifiedAccess" in factKind
                    "Literal" in firKind -> "Literal" in factKind
                    "Jump" in firKind -> "Return" in factKind
                    "Throw" in firKind -> "Throw" in factKind
                    else -> false
                }
            }
            val matching = preferred.minByOrNull { kotlin.math.abs((it["end"]?.jsonPrimitive?.intOrNull ?: end!!) - (it["start"]?.jsonPrimitive?.intOrNull ?: start!!)) }
            buildJsonObject {
                node.forEach { (key, value) -> put(key, value) }
                putJsonObject("attributes") {
                    node["attributes"]?.jsonObject?.forEach { (key, value) -> put(key, value) }
                    put("analysis", "K2_FIR")
                    matching?.let { fact -> listOf("kind", "type", "symbol", "returnType", "receiverType", "argumentToParameter").forEach { key -> fact[key]?.let { put(key, it) } } }
                    val mergedEffects = (node["attributes"]?.jsonObject?.get("effects")?.jsonArray?.map { it.jsonPrimitive.content }.orEmpty() + matching?.get("effects")?.jsonArray?.map { it.jsonPrimitive.content }.orEmpty()).distinct().sorted()
                    if (mergedEffects.isNotEmpty()) putJsonArray("effects") { mergedEffects.forEach(::add) }
                    matching?.get("symbol")?.jsonPrimitive?.content?.let { symbol -> calleeSummaryHash(repo, analysis, symbol)?.let { put("calleeSummaryHash", it) } }
                }
            }
        }
        return buildJsonObject {
            graph.forEach { (key, value) -> if (key != "nodes" && key != "boundaries") put(key, value) }
            put("graphSource", graph["graphSource"] ?: JsonPrimitive("K2_FIR_VALIDATED_STRUCTURED_CFG"))
            putJsonArray("nodes") { nodes.forEach(::add) }
            putJsonArray("boundaries") {
                graph["boundaries"]?.jsonArray?.forEach(::add)
                if (!analysis.valid) add(buildJsonObject { put("kind", "INCOMPLETE_SEMANTIC_ANALYSIS"); putJsonArray("diagnostics") { analysis.diagnostics.forEach(::add) } })
            }
        }
    }

    private fun calleeSummaryHash(repo: Path, analysis: K2Analysis, symbol: String): String? {
        val cfg = analysis.facts.firstOrNull { it["recordType"]?.jsonPrimitive?.content == "FIR_CFG" && it["symbol"]?.jsonPrimitive?.content == symbol } ?: return null
        val fileName = cfg["file"]?.jsonPrimitive?.content ?: return null
        val path = Path.of(fileName)
        if (!path.isRegularFile()) return null
        val start = cfg["start"]?.jsonPrimitive?.intOrNull ?: return null; val end = cfg["end"]?.jsonPrimitive?.intOrNull ?: return null
        val text = path.readText()
        val bodyHash = sha(normalizeTokens(text.substring(start.coerceAtLeast(0), end.coerceAtMost(text.length))).toByteArray())
        val facts = analysis.facts.filter {
            it["recordType"]?.jsonPrimitive?.content == "SEMANTIC_FACT" && it["file"]?.jsonPrimitive?.content == fileName && (it["start"]?.jsonPrimitive?.intOrNull ?: -1) >= start && (it["end"]?.jsonPrimitive?.intOrNull ?: -1) <= end
        }.map(::semanticSignature)
        return sha(buildJsonObject { put("symbol", symbol); put("bodyHash", bodyHash); putJsonArray("facts") { facts.forEach(::add) } }.toString().toByteArray())
    }

    private fun graphNode(id: String, kind: String, defines: String?, origin: JsonObject?, uses: List<String> = emptyList()) = buildJsonObject {
        put("id", id); put("kind", kind); if (defines != null) put("defines", defines); putJsonArray("uses") { uses.sorted().forEach(::add) }; if (origin != null) put("origin", origin)
    }
    private fun edge(from: String, to: String, kind: String) = buildJsonObject { put("from", from); put("to", to); put("kind", kind) }
    private fun definedName(e: KtExpression): String? = when (e) { is KtProperty -> e.name; is KtBinaryExpression -> if (e.operationReference.text.contains("=")) (e.left as? KtNameReferenceExpression)?.getReferencedName() else null; else -> null }
    private fun usedNames(e: PsiElement): List<String> = (listOfNotNull((e as? KtNameReferenceExpression)?.getReferencedName()) + PsiTreeUtil.collectElementsOfType(e, KtNameReferenceExpression::class.java).map { it.getReferencedName() }).distinct()
    private fun normalizedKind(e: KtExpression) = when (e) { is KtCallExpression -> "CALL"; is KtProperty -> "DEFINITION"; is KtBinaryExpression -> if (e.operationReference.text.contains("=")) "ASSIGNMENT" else "EXPRESSION"; else -> "EXPRESSION" }
    private fun normalizedUses(e: KtExpression): List<String> {
        val names = usedNames(e).toMutableList()
        if (e is KtProperty) e.name?.let(names::remove)
        if (e is KtBinaryExpression && e.operationReference.text == "=") (e.left as? KtNameReferenceExpression)?.getReferencedName()?.let(names::remove)
        return names.distinct()
    }
    private fun isGraphExpression(e: KtExpression) = e is KtReturnExpression || e is KtProperty || e is KtBinaryExpression || e is KtCallExpression || e is KtIfExpression || e is KtWhenExpression || e is KtLoopExpression || e is KtThrowExpression || e is KtTryExpression || e is KtBreakExpression || e is KtContinueExpression || e is KtSafeQualifiedExpression

    private inner class LocalCfgBuilder(private val file: String, private val owner: String, private val source: String) {
        private val nodes = mutableListOf<JsonObject>()
        private val edges = mutableListOf<JsonObject>()
        private val boundaries = mutableListOf<JsonObject>()
        private var counter = 0

        fun build(fn: KtNamedFunction): JsonObject {
            nodes += graphNode("entry", "ENTRY", null, null)
            nodes += graphNode("exit", "EXIT", null, null)
            nodes += graphNode("exception_exit", "EXCEPTION_EXIT", null, null)
            val body = fn.bodyExpression
            var bodyEntry = "exit"
            if (body == null) boundaries += boundary("MISSING_BODY", fn)
            else if (body is KtBlockExpression) bodyEntry = buildElement(body, CfgNext("exit"), null, CfgNext("exception_exit", "CFG_EXCEPTION"))
            else {
                val implicitReturn = addNode(body, "RETURN", null, usedNames(body))
                edges += edge(implicitReturn, "exit", "CFG_NORMAL")
                bodyEntry = if (needsOwnNode(body)) {
                    val valueEntry = buildElement(body, CfgNext(implicitReturn, "RETURN"), null, CfgNext("exception_exit", "CFG_EXCEPTION"))
                    edges += edge(valueEntry, implicitReturn, "RETURN")
                    valueEntry
                } else implicitReturn
            }
            var previous = "entry"
            fn.valueParameters.forEachIndexed { index, parameter ->
                val id = "param:$index"
                nodes += graphNode(id, "PARAMETER", parameter.name, anchor(file, owner, parameter, source))
                edges += edge(previous, id, "CFG_NORMAL"); previous = id
            }
            edges += edge(previous, bodyEntry, "CFG_NORMAL")
            val unsupported = PsiTreeUtil.collectElementsOfType(body ?: fn, KtExpression::class.java).filter {
                it is KtObjectLiteralExpression || it is KtCallableReferenceExpression
            }
            unsupported.forEach { boundaries += boundary("UNSUPPORTED_CONTROL_FLOW", it) }
            return buildJsonObject {
                put("schema", "local-cfg/0.1"); put("symbol", owner); put("file", file)
                putJsonArray("nodes") { nodes.sortedBy { it["id"]!!.jsonPrimitive.content }.forEach(::add) }
                putJsonArray("edges") { edges.distinctBy { it.toString() }.sortedBy { it.toString() }.forEach(::add) }
                putJsonArray("boundaries") { boundaries.sortedBy { it.toString() }.forEach(::add) }
            }
        }

        private fun buildElement(element: KtExpression, next: CfgNext, loop: CfgLoopContext?, exception: CfgNext): String = when (element) {
            is KtBlockExpression -> element.statements.asReversed().fold(next) { continuation, statement -> CfgNext(buildElement(statement, continuation, loop, exception)) }.id
            is KtIfExpression -> {
                val id = addNode(element, "BRANCH", null, element.condition?.let(::usedNames).orEmpty())
                val yes = element.then?.let { buildElement(it, next, loop, exception) } ?: next.id
                val no = element.`else`?.let { buildElement(it, next, loop, exception) } ?: next.id
                edges += edge(id, yes, "CFG_TRUE"); edges += edge(id, no, "CFG_FALSE"); id
            }
            is KtWhenExpression -> {
                val id = addNode(element, "BRANCH", null, element.subjectExpression?.let(::usedNames).orEmpty() + element.entries.flatMap { entry -> entry.conditions.flatMap(::usedNames) })
                if (element.entries.isEmpty()) edges += edge(id, next.id, "CFG_FALSE")
                element.entries.forEach { entry ->
                    val target = entry.expression?.let { buildElement(it, next, loop, exception) } ?: next.id
                    edges += edge(id, target, if (entry.isElse) "CFG_FALSE" else "CFG_TRUE")
                }; id
            }
            is KtWhileExpression -> buildWhile(element, next, exception)
            is KtDoWhileExpression -> buildDoWhile(element, next, exception)
            is KtForExpression -> buildFor(element, next, exception)
            is KtBreakExpression -> addNode(element, "BREAK", null, emptyList()).also { edges += edge(it, loop?.breakTarget?.id ?: exception.id, loop?.breakTarget?.edge ?: "CFG_EXCEPTION") }
            is KtContinueExpression -> addNode(element, "CONTINUE", null, emptyList()).also { edges += edge(it, loop?.continueTarget?.id ?: exception.id, loop?.continueTarget?.edge ?: "CFG_EXCEPTION") }
            is KtReturnExpression -> addNode(element, "RETURN", null, element.returnedExpression?.let(::usedNames).orEmpty()).also { edges += edge(it, "exit", "CFG_NORMAL"); edges += edge(it, "exit", "RETURN") }
            is KtThrowExpression -> addNode(element, "THROW", null, element.thrownExpression?.let(::usedNames).orEmpty()).also { edges += edge(it, exception.id, "CFG_EXCEPTION"); edges += edge(it, exception.id, "THROW") }
            is KtTryExpression -> buildTry(element, next, loop, exception)
            is KtBinaryExpression -> buildBinary(element, next, loop, exception)
            is KtSafeQualifiedExpression -> {
                val id = addNode(element, "BRANCH", null, usedNames(element.receiverExpression))
                val selector = element.selectorExpression?.let { buildElement(it, next, loop, exception) } ?: next.id
                edges += edge(id, selector, "CFG_TRUE"); edges += edge(id, next.id, "CFG_FALSE"); id
            }
            else -> addNode(element, kind(element), definedName(element), usedNamesForNode(element)).also { edges += edge(it, next.id, next.edge) }
        }

        private fun buildWhile(element: KtWhileExpression, next: CfgNext, exception: CfgNext): String {
            val condition = addNode(element, "LOOP", null, element.condition?.let(::usedNames).orEmpty())
            val context = CfgLoopContext(next, CfgNext(condition, "CFG_BACK"))
            val body = element.body?.let { buildElement(it, CfgNext(condition, "CFG_BACK"), context, exception) } ?: condition
            edges += edge(condition, body, "CFG_TRUE"); edges += edge(condition, next.id, "CFG_FALSE"); return condition
        }
        private fun buildDoWhile(element: KtDoWhileExpression, next: CfgNext, exception: CfgNext): String {
            val condition = addNode(element, "LOOP", null, element.condition?.let(::usedNames).orEmpty())
            val context = CfgLoopContext(next, CfgNext(condition, "CFG_NORMAL"))
            val body = element.body?.let { buildElement(it, CfgNext(condition), context, exception) } ?: condition
            edges += edge(condition, body, "CFG_BACK"); edges += edge(condition, next.id, "CFG_FALSE"); return body
        }
        private fun buildFor(element: KtForExpression, next: CfgNext, exception: CfgNext): String {
            val condition = addNode(element, "LOOP", element.loopParameter?.name, element.loopRange?.let(::usedNames).orEmpty())
            val context = CfgLoopContext(next, CfgNext(condition, "CFG_BACK"))
            val body = element.body?.let { buildElement(it, CfgNext(condition, "CFG_BACK"), context, exception) } ?: condition
            edges += edge(condition, body, "CFG_TRUE"); edges += edge(condition, next.id, "CFG_FALSE"); return condition
        }
        private fun buildTry(element: KtTryExpression, next: CfgNext, loop: CfgLoopContext?, exception: CfgNext): String {
            val finallyEntry = element.finallyBlock?.finalExpression?.let { buildElement(it, next, loop, exception) } ?: next.id
            val catches = element.catchClauses.map { clause ->
                val body = clause.catchBody?.let { buildElement(it, CfgNext(finallyEntry), loop, exception) } ?: finallyEntry
                clause.catchParameter?.let { parameter ->
                    addNode(parameter, "DEFINITION", parameter.name, emptyList()).also { edges += edge(it, body, "CFG_NORMAL") }
                } ?: body
            }
            val catchTarget = catches.firstOrNull()?.let { CfgNext(it, "CFG_EXCEPTION") } ?: exception
            val tryEntry = buildElement(element.tryBlock, CfgNext(finallyEntry), loop, catchTarget)
            val id = addNode(element, "TRY", null, emptyList())
            edges += edge(id, tryEntry, "CFG_NORMAL"); catches.forEach { edges += edge(id, it, "CFG_EXCEPTION") }; return id
        }
        private fun buildBinary(element: KtBinaryExpression, next: CfgNext, loop: CfgLoopContext?, exception: CfgNext): String {
            val op = element.operationReference.text
            if (op == "&&" || op == "||" || op == "?:") {
                val id = addNode(element, "BRANCH", null, element.left?.let(::usedNames).orEmpty())
                val rhs = element.right?.let { buildElement(it, next, loop, exception) } ?: next.id
                when (op) {
                    "&&" -> { edges += edge(id, rhs, "CFG_TRUE"); edges += edge(id, next.id, "CFG_FALSE") }
                    else -> { edges += edge(id, next.id, "CFG_TRUE"); edges += edge(id, rhs, "CFG_FALSE") }
                }; return id
            }
            return addNode(element, kind(element), definedName(element), usedNamesForNode(element)).also { edges += edge(it, next.id, next.edge) }
        }
        private fun needsOwnNode(e: KtExpression) = e is KtIfExpression || e is KtWhenExpression || e is KtLoopExpression || e is KtTryExpression || e is KtCallExpression || e is KtSafeQualifiedExpression || (e is KtBinaryExpression && e.operationReference.text in setOf("&&", "||", "?:"))
        private fun kind(e: PsiElement) = when (e) { is KtCallExpression -> "CALL"; is KtProperty -> "DEFINITION"; is KtBinaryExpression -> if (e.operationReference.text.contains("=")) "ASSIGNMENT" else "EXPRESSION"; else -> "EXPRESSION" }
        private fun usedNamesForNode(e: KtExpression): List<String> {
            val names = usedNames(e).toMutableList()
            if (e is KtProperty) e.name?.let(names::remove)
            if (e is KtBinaryExpression && e.operationReference.text == "=") (e.left as? KtNameReferenceExpression)?.getReferencedName()?.let(names::remove)
            return names.distinct()
        }
        private fun addNode(element: PsiElement, kind: String, defines: String?, uses: List<String>): String {
            val id = "n:${counter++}"; nodes += graphNode(id, kind, defines, anchor(file, owner, element, source), uses); return id
        }
        private fun boundary(kind: String, element: PsiElement) = buildJsonObject { put("kind", kind); put("syntaxKind", element::class.simpleName ?: "PsiElement"); putJsonArray("range") { add(element.textRange.startOffset); add(element.textRange.endOffset) } }
    }

    private fun applyEdit(request: JsonObject): JsonObject {
        val repo = Path.of(request.requiredString("repo")); val relative = request.requiredString("file"); val path = repo.resolve(relative).normalize()
        val source = request["source"]?.jsonPrimitive?.content ?: path.readText()
        val kind = request.requiredString("kind"); val replacement = request.requiredString("replacement")
        if (kind == "ADD_IMPORT" || kind == "REMOVE_IMPORT") return applyImportEdit(repo, relative, path, source, kind, replacement)
        val kt = factory.createFile(path.fileName.toString(), source); val ownerQuery = request.requiredString("ownerSymbolId")
        val pkg = kt.packageFqName.asString(); val owner = PsiTreeUtil.collectElementsOfType(kt, KtNamedFunction::class.java).singleOrNull { symbolId(pkg, it) == ownerQuery || symbolId(pkg, it).substringBefore('(') == ownerQuery }
            ?: throw WorkerFailure("STALE_TARGET", "owner no longer resolves uniquely")
        val expectedHash = request.requiredString("exactTextHash")
        val syntaxKind = request["syntaxKind"]?.jsonPrimitive?.content
        val tokenHash = request["normalizedTokenHash"]?.jsonPrimitive?.content
        var matches: List<PsiElement> = if (kind == "REPLACE_FUNCTION_BODY") listOfNotNull(owner.bodyExpression) else
            PsiTreeUtil.collectElementsOfType(owner, KtExpression::class.java).toList()
        if (syntaxKind != null) matches = matches.filter { it::class.simpleName == syntaxKind }
        if (tokenHash != null) matches = matches.filter { sha(normalizeTokens(it.text).toByteArray()) == tokenHash }
        // Exact text is a precondition, not the identity. Context/path fields are
        // only tie-breakers: a unique token target survives neighboring edits.
        matches = matches.filter { sha(it.text.toByteArray()) == expectedHash }
        if (matches.isEmpty()) throw WorkerFailure("STALE_TARGET", "target hash no longer exists")
        if (matches.size > 1) {
            val requested = listOf("ancestorPathHash", "localOrdinal", "leftContextHash", "rightContextHash")
            requested.forEach { field ->
                val expected = request[field] ?: return@forEach
                val narrowed = matches.filter { candidate -> anchor(relative, ownerQuery, candidate, source)[field] == expected }
                if (narrowed.isNotEmpty()) matches = narrowed
            }
        }
        if (matches.size > 1) throw WorkerFailure("AMBIGUOUS_TARGET", "target hash resolves to ${matches.size} nodes")
        val oldEffects = effects(matches.single())
        val replacementNode = try {
            if (kind == "REPLACE_FUNCTION_BODY") {
                if (replacement.trimStart().startsWith("{")) factory.createFunction("fun __semantic_thread_candidate__() $replacement").bodyExpression as? KtBlockExpression ?: throw IllegalArgumentException("function body must be a Kotlin block")
                else factory.createBlock(replacement)
            } else factory.createExpression(replacement)
        }
            catch (e: Throwable) { throw WorkerFailure("REPLACEMENT_PARSE_ERROR", e.message ?: "replacement parse failed") }
        val matchedRange = matches.single().textRange
        val editStart = if (kind == "REPLACE_FUNCTION_BODY" && !owner.hasBlockBody()) owner.equalsToken?.textRange?.startOffset ?: matchedRange.startOffset else matchedRange.startOffset
        val editEnd = matchedRange.endOffset
        val candidate = source.substring(0, editStart) + replacementNode.text + source.substring(editEnd)
        val candidateFile = factory.createFile(path.fileName.toString(), candidate)
        val errors = PsiTreeUtil.collectElementsOfType(candidateFile, PsiErrorElement::class.java).map { it.errorDescription }.sorted()
        if (errors.isNotEmpty()) throw WorkerFailure("REPLACEMENT_PARSE_ERROR", errors.joinToString("; "))
        val syntacticIntroducedEffects = effects(replacementNode) - oldEffects
        val baselineOverrides = if (source == path.readText()) emptyMap() else mapOf(relative to source)
        val baseline = analyzeWithK2(repo, baselineOverrides)
        val candidateAnalysis = analyzeWithK2(repo, mapOf(relative to candidate))
        val baselineFacts = fileFacts(repo, path, baseline)
        val candidateFacts = fileFacts(repo, path, candidateAnalysis)
        val oldSemanticEffects = semanticEffects(baselineFacts, matchedRange.startOffset, matchedRange.endOffset)
        val newSemanticEffects = semanticEffects(candidateFacts, editStart, editStart + replacementNode.text.length)
        val introducedEffects = syntacticIntroducedEffects + (newSemanticEffects - oldSemanticEffects)
        val baselineErrors = baseline.diagnostics.filter(::isErrorDiagnostic).map(::diagnosticIdentity).toSet()
        val allowedDiagnostics = request["postconditions"]?.jsonObject?.get("allowedDiagnostics")?.jsonArray?.map { it.jsonPrimitive.content }.orEmpty()
        val newErrors = candidateAnalysis.diagnostics.filter(::isErrorDiagnostic).filter { diagnostic ->
            val identity = diagnosticIdentity(diagnostic)
            identity !in baselineErrors && allowedDiagnostics.none { it in identity }
        }
        if (!candidateAnalysis.valid || newErrors.isNotEmpty()) {
            throw WorkerFailure("NEW_DIAGNOSTICS", (newErrors.ifEmpty { candidateAnalysis.diagnostics }).joinToString("; ") { it["message"]?.jsonPrimitive?.content.orEmpty() })
        }
        val delta = replacementNode.text.length - (editEnd - editStart)
        val changedBinding = baselineFacts.firstOrNull { fact ->
            val start = fact["start"]?.jsonPrimitive?.intOrNull ?: return@firstOrNull false
            val end = fact["end"]?.jsonPrimitive?.intOrNull ?: return@firstOrNull false
            if (start < editEnd && end > editStart) return@firstOrNull false
            val expectedStart = if (start >= editEnd) start + delta else start
            val expectedEnd = if (end > editEnd) end + delta else end
            candidateFacts.none { candidateFact ->
                candidateFact["start"]?.jsonPrimitive?.intOrNull == expectedStart && candidateFact["end"]?.jsonPrimitive?.intOrNull == expectedEnd && semanticSignature(candidateFact) == semanticSignature(fact)
            }
        }
        if (changedBinding != null) throw WorkerFailure("BINDING_CHANGED", "protected K2 FIR fact changed: ${semanticSignature(changedBinding)}")
        val preconditions = request["preconditions"]?.jsonObject ?: buildJsonObject {}
        val postconditions = request["postconditions"]?.jsonObject ?: buildJsonObject {}
        val protectedFacts = baselineFacts.filter {
            val start = it["start"]?.jsonPrimitive?.intOrNull ?: -1; val end = it["end"]?.jsonPrimitive?.intOrNull ?: -1
            start >= owner.textRange.startOffset && end <= owner.textRange.endOffset && !(start < editEnd && end > editStart)
        }
        preconditions["scopeBindingsHash"]?.jsonPrimitive?.content?.let { expected ->
            val actual = sha(JsonArray(protectedFacts.map(::semanticSignature)).toString().toByteArray())
            if (actual != expected) throw WorkerFailure("BINDING_CHANGED", "scopeBindingsHash precondition failed")
        }
        val originalType = expressionType(baselineFacts, matchedRange.startOffset, matchedRange.endOffset)
        val replacementEnd = editStart + replacementNode.text.length
        val replacementType = expressionType(candidateFacts, editStart, replacementEnd)
        preconditions["expectedType"]?.jsonPrimitive?.content?.let { expected ->
            if (!sameType(originalType, expected)) throw WorkerFailure("TYPE_MISMATCH", "expected target type $expected, resolved ${originalType ?: "<unknown>"}")
        }
        postconditions["typeAssignableTo"]?.jsonPrimitive?.content?.let { expected ->
            if (kind == "REPLACE_EXPRESSION") {
                val probe = "run { val __semantic_thread_type_probe: $expected = $replacement; $replacement }"
                val probeSource = source.substring(0, matchedRange.startOffset) + probe + source.substring(matchedRange.endOffset)
                val probeAnalysis = analyzeWithK2(repo, mapOf(relative to probeSource))
                if (!probeAnalysis.valid || probeAnalysis.diagnostics.any(::isErrorDiagnostic)) throw WorkerFailure("TYPE_MISMATCH", "replacement type ${replacementType ?: "<unknown>"} is not assignable to $expected: ${probeAnalysis.diagnostics.filter(::isErrorDiagnostic).joinToString("; ") { it["message"]?.jsonPrimitive?.content.orEmpty() }}")
            } else if (!sameType(replacementType, expected)) throw WorkerFailure("TYPE_MISMATCH", "replacement type ${replacementType ?: "<unknown>"} is not assignable to $expected")
        }
        if (postconditions["preserveEffects"]?.jsonPrimitive?.booleanOrNull == true && introducedEffects.isNotEmpty()) throw WorkerFailure("EFFECT_CHANGED", "replacement changes effects: ${introducedEffects.sorted()}")
        return buildJsonObject {
            put("schema", "semantic-candidate/0.1"); put("file", relative); put("originalHash", sha(source.toByteArray())); put("candidateHash", sha(candidate.toByteArray())); put("source", candidate)
            putJsonArray("diagnostics") { candidateAnalysis.diagnostics.forEach(::add) }; putJsonArray("introducedEffects") { introducedEffects.sorted().forEach(::add) }
            originalType?.let { put("originalType", it) }; replacementType?.let { put("replacementType", it) }
            put("protectedBindingsHash", sha(JsonArray(protectedFacts.map(::semanticSignature)).toString().toByteArray())); put("k2Validated", candidateAnalysis.valid)
        }
    }

    private fun applyImportEdit(repo: Path, relative: String, path: Path, source: String, kind: String, replacement: String): JsonObject {
        val fqName = replacement.trim().removePrefix("import ").trim()
        if (!fqName.matches(Regex("[A-Za-z_][A-Za-z0-9_.]*(?:\\.\\*)?"))) throw WorkerFailure("REPLACEMENT_PARSE_ERROR", "invalid import: $fqName")
        val lineEnding = if ("\r\n" in source) "\r\n" else "\n"
        val line = "import $fqName"
        val existing = source.lineSequence().any { it.trim() == line }
        val candidate = when {
            kind == "ADD_IMPORT" && existing -> source
            kind == "REMOVE_IMPORT" && !existing -> source
            kind == "ADD_IMPORT" -> {
                val kt = factory.createFile(path.fileName.toString(), source)
                val imports = kt.importDirectives
                if (imports.isEmpty()) {
                    val insertion = kt.packageDirective?.textRange?.endOffset ?: 0
                    source.substring(0, insertion) + lineEnding + lineEnding + line + source.substring(insertion)
                } else {
                    val names = (imports.mapNotNull { it.importPath?.pathStr } + fqName).distinct().sorted()
                    val start = imports.first().textRange.startOffset; val end = imports.last().textRange.endOffset
                    source.substring(0, start) + names.joinToString(lineEnding) { "import $it" } + source.substring(end)
                }
            }
            else -> {
                val pattern = Regex("(?m)^import\\s+${Regex.escape(fqName)}\\s*(?:\\r?\\n)?")
                source.replaceFirst(pattern, "")
            }
        }
        val baselineOverrides = if (source == path.readText()) emptyMap() else mapOf(relative to source)
        val baseline = analyzeWithK2(repo, baselineOverrides); val candidateAnalysis = analyzeWithK2(repo, mapOf(relative to candidate))
        if (!candidateAnalysis.valid || candidateAnalysis.diagnostics.any(::isErrorDiagnostic)) throw WorkerFailure("NEW_DIAGNOSTICS", candidateAnalysis.diagnostics.joinToString("; ") { it["message"]?.jsonPrimitive?.content.orEmpty() })
        val before = fileFacts(repo, path, baseline).map(::semanticSignature).groupingBy { it.toString() }.eachCount()
        val after = fileFacts(repo, path, candidateAnalysis).map(::semanticSignature).groupingBy { it.toString() }.eachCount()
        if (before != after) throw WorkerFailure("BINDING_CHANGED", "import operation changes protected K2 bindings")
        return buildJsonObject {
            put("schema", "semantic-candidate/0.1"); put("file", relative); put("originalHash", sha(source.toByteArray())); put("candidateHash", sha(candidate.toByteArray())); put("source", candidate)
            putJsonArray("diagnostics") { candidateAnalysis.diagnostics.forEach(::add) }; putJsonArray("introducedEffects") {}; put("k2Validated", candidateAnalysis.valid)
        }
    }

    private fun isErrorDiagnostic(diagnostic: JsonObject) = diagnostic["severity"]?.jsonPrimitive?.content == "ERROR"
    private fun diagnosticIdentity(diagnostic: JsonObject) = diagnostic["message"]?.jsonPrimitive?.content.orEmpty().replace(Regex("/[^ :]+/semantic-thread-k2[^ :]+"), "<candidate>")
    private fun semanticSignature(fact: JsonObject) = buildJsonObject {
        listOf("kind", "type", "symbol", "returnType", "receiverType", "effects").forEach { key -> fact[key]?.let { put(key, it) } }
        fact["argumentToParameter"]?.jsonArray?.let { mapping ->
            putJsonArray("argumentToParameter") {
                mapping.forEach { item -> add(buildJsonObject { item.jsonObject["parameter"]?.let { put("parameter", it) }; item.jsonObject["parameterType"]?.let { put("parameterType", it) } }) }
            }
        }
    }
    private fun semanticEffects(facts: List<JsonObject>, start: Int, end: Int): Set<String> = facts.filter {
        val fs = it["start"]?.jsonPrimitive?.intOrNull ?: -1; val fe = it["end"]?.jsonPrimitive?.intOrNull ?: -1
        fs >= start && fe <= end
    }.flatMap { it["effects"]?.jsonArray?.map { effect -> effect.jsonPrimitive.content }.orEmpty() }.toSet()
    private fun expressionType(facts: List<JsonObject>, start: Int, end: Int): String? = facts.filter {
        val fs = it["start"]?.jsonPrimitive?.intOrNull ?: -1; val fe = it["end"]?.jsonPrimitive?.intOrNull ?: -1
        fs >= start && fe <= end && it["type"]?.jsonPrimitive?.content?.let { type -> type != "kotlin/Nothing" } == true
    }.minByOrNull { (it["end"]!!.jsonPrimitive.int - it["start"]!!.jsonPrimitive.int) }?.get("type")?.jsonPrimitive?.content
    private fun sameType(actual: String?, expected: String): Boolean {
        if (actual == null) return false
        fun normalize(value: String) = value.removePrefix("kotlin/").replace('/', '.')
        return normalize(actual) == normalize(expected) || normalize(actual).substringAfterLast('.') == normalize(expected).substringAfterLast('.')
    }

    private fun validateCandidate(request: JsonObject): JsonObject {
        val source = request.requiredString("source"); val kt = factory.createFile(request["file"]?.jsonPrimitive?.content ?: "Candidate.kt", source)
        val errors = PsiTreeUtil.collectElementsOfType(kt, PsiErrorElement::class.java).map { it.errorDescription }.sorted()
        return buildJsonObject { put("valid", errors.isEmpty()); putJsonArray("diagnostics") { errors.forEach(::add) } }
    }

    private fun effects(element: PsiElement): Set<String> {
        val result = mutableSetOf<String>()
        if (element is KtThrowExpression || PsiTreeUtil.collectElementsOfType(element, KtThrowExpression::class.java).isNotEmpty()) result += "THROW"
        PsiTreeUtil.collectElementsOfType(element, KtCallExpression::class.java).forEach { call ->
            val name = call.calleeExpression?.text.orEmpty()
            if (name in setOf("print", "println", "readLine")) result += "IO" else result += "READ_STATE"
        }
        PsiTreeUtil.collectElementsOfType(element, KtBinaryExpression::class.java).forEach { binary ->
            if (binary.operationReference.text.contains("=") && (binary.left is KtDotQualifiedExpression || binary.left?.text?.startsWith("this.") == true)) result += "WRITE_STATE"
        }
        return result
    }

    override fun close() = Disposer.dispose(disposable)
}

internal class WorkerFailure(val code: String, override val message: String) : RuntimeException(message)
private data class CfgNext(val id: String, val edge: String = "CFG_NORMAL")
private data class CfgLoopContext(val breakTarget: CfgNext, val continueTarget: CfgNext)
private data class K2Analysis(val valid: Boolean, val facts: List<JsonObject>, val diagnostics: List<JsonObject>)
private fun JsonObject.requiredString(name: String) = this[name]?.jsonPrimitive?.content ?: error("missing field $name")
private fun JsonObject.requiredInt(name: String) = this[name]?.jsonPrimitive?.int ?: error("missing field $name")
private fun ByteArray.hex() = joinToString("") { "%02x".format(it) }
private fun sha(bytes: ByteArray) = "sha256:" + MessageDigest.getInstance("SHA-256").digest(bytes).hex()
private fun normalizeTokens(text: String): String {
    val out = StringBuilder(); var i = 0; var line = false; var block = false
    while (i < text.length) {
        val c = text[i]; val n = text.getOrNull(i + 1)
        if (line) { if (c == '\n') line = false; i++; continue }
        if (block) { if (c == '*' && n == '/') { block = false; i += 2 } else i++; continue }
        if (c == '/' && n == '/') { line = true; i += 2; continue }
        if (c == '/' && n == '*') { block = true; i += 2; continue }
        if (!c.isWhitespace()) out.append(c); i++
    }
    return out.toString()
}
