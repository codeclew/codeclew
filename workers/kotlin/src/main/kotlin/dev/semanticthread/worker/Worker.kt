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

    fun handle(kind: Int, payload: ByteArray): String {
        val request = if (payload.isEmpty()) buildJsonObject {} else json.parseToJsonElement(payload.decodeToString()).jsonObject
        return when (kind) {
            2 -> inspect(Path.of(request.requiredString("repo"))).toString()
            3 -> index(Path.of(request.requiredString("repo"))).toString()
            4 -> resolveSymbol(Path.of(request.requiredString("repo")), request.requiredString("symbol")).toString()
            5 -> resolveExpression(Path.of(request.requiredString("repo")), request.requiredString("file"), request.requiredInt("offset")).toString()
            6 -> localGraph(Path.of(request.requiredString("repo")), request.requiredString("symbol")).toString()
            7 -> applyEdit(request).toString()
            8 -> validateCandidate(request).toString()
            else -> error("unsupported request kind $kind")
        }
    }

    private fun inspect(repo: Path): JsonObject {
        require(repo.isDirectory()) { "repository does not exist: $repo" }
        val modelFiles = projectModelFiles(repo)
        val sourceRoots = Files.walk(repo).use { paths ->
            paths.filter { it.isDirectory() && it.invariantSeparatorsPathString.endsWith("/src/main/kotlin") }
                .map { repo.relativize(it).invariantSeparatorsPathString }.sorted().toList()
        }
        val testRoots = Files.walk(repo).use { paths ->
            paths.filter { it.isDirectory() && it.invariantSeparatorsPathString.endsWith("/src/test/kotlin") }
                .map { repo.relativize(it).invariantSeparatorsPathString }.sorted().toList()
        }
        val digest = MessageDigest.getInstance("SHA-256")
        modelFiles.forEach { p -> digest.update(repo.relativize(p).invariantSeparatorsPathString.toByteArray()); digest.update(p.readBytes()) }
        val modelHash = "sha256:" + digest.digest().hex()
        return buildJsonObject {
            put("schema", "semantic-project/0.1"); put("projectPath", repo.toAbsolutePath().normalize().toString())
            put("module", ":"); put("sourceSet", "main"); put("compilerVersion", "2.4.10"); put("jdk", 21); put("jvmTarget", "21")
            putJsonArray("sourceRoots") { sourceRoots.forEach(::add) }; putJsonArray("testSourceRoots") { testRoots.forEach(::add) }
            putJsonArray("compileClasspath") {}; putJsonArray("friendPaths") {}; putJsonArray("freeCompilerArguments") {}
            putJsonArray("optIns") {}; putJsonArray("compilerPlugins") {}
            putJsonArray("compileTasks") { add("compileKotlin") }; putJsonArray("testTasks") { add("test") }
            put("projectModelHash", modelHash)
        }
    }

    private fun projectModelFiles(repo: Path): List<Path> = Files.walk(repo).use { paths ->
        paths.filter { it.isRegularFile() }.filter {
            val n = it.fileName.toString()
            n == "settings.gradle" || n == "settings.gradle.kts" || n == "build.gradle" || n == "build.gradle.kts" ||
                n == "gradle.properties" || n == "libs.versions.toml" || n == "gradle-wrapper.properties"
        }.sorted().toList()
    }

    private fun sourceFiles(repo: Path): List<Path> = Files.walk(repo).use { paths ->
        paths.filter { it.isRegularFile() && it.extension == "kt" && it.invariantSeparatorsPathString.contains("/src/main/kotlin/") && !it.invariantSeparatorsPathString.contains("/build/") }
            .sorted().toList()
    }

    private fun parse(path: Path): KtFile = factory.createFile(path.fileName.toString(), path.readText())

    private fun index(repo: Path): JsonObject {
        val files = sourceFiles(repo).map { path ->
            val bytes = path.readBytes(); val kt = parse(path); val pkg = kt.packageFqName.asString()
            val declarations = PsiTreeUtil.collectElementsOfType(kt, KtNamedDeclaration::class.java)
                .filter { it is KtNamedFunction || it is KtClassOrObject || it is KtProperty }
                .sortedBy { it.textOffset }.map { declarationJson(repo, path, pkg, it) }
            buildJsonObject {
                put("path", repo.relativize(path).invariantSeparatorsPathString); put("contentHash", sha(bytes)); put("package", pkg)
                put("lineEnding", if (bytes.decodeToString().contains("\r\n")) "CRLF" else "LF"); put("bom", bytes.take(3) == listOf(0xef.toByte(), 0xbb.toByte(), 0xbf.toByte()))
                putJsonArray("imports") { kt.importDirectives.map { it.importPath?.pathStr.orEmpty() }.sorted().forEach(::add) }
                putJsonArray("declarations") { declarations.forEach(::add) }
            }
        }
        val canonical = JsonArray(files)
        return buildJsonObject { put("schema", "semantic-index/0.1"); put("files", canonical); put("indexHash", sha(canonical.toString().toByteArray())) }
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
        val (_, kt, fn) = findFunction(repo, query)
        return buildJsonObject {
            put("schema", "semantic-symbol/0.1"); put("declaration", declaration)
            fn.bodyExpression?.let { put("bodyAnchor", anchor(file, symbolId(kt.packageFqName.asString(), fn), it, kt.text)) }
            putJsonArray("references") { fn.bodyExpression?.let(::usedNames).orEmpty().sorted().forEach(::add) }
            putJsonArray("calls") { PsiTreeUtil.collectElementsOfType(fn, KtCallExpression::class.java).map { it.calleeExpression?.text.orEmpty() }.sorted().forEach(::add) }
            putJsonArray("declaredTypes") { (fn.valueParameters.mapNotNull { it.typeReference?.text } + listOfNotNull(fn.typeReference?.text)).sorted().forEach(::add) }
            putJsonArray("diagnostics") {}
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
        val expression = generateSequence(leaf as PsiElement?) { it.parent }.filterIsInstance<KtExpression>().firstOrNull()
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
            put("anchorId", "anchor:" + sha("$file|$owner|${node::class.simpleName}|${normalizeTokens(node.text)}|${ancestor.joinToString("/")}".toByteArray()).removePrefix("sha256:"))
        }
    }

    private fun localGraph(repo: Path, query: String): JsonObject {
        val (path, kt, fn) = findFunction(repo, query); val relative = repo.relativize(path).invariantSeparatorsPathString
        val expressions = PsiTreeUtil.collectElementsOfType(fn.bodyExpression ?: fn, KtExpression::class.java)
            .filter { e -> e is KtReturnExpression || e is KtProperty || e is KtBinaryExpression || e is KtCallExpression || e is KtIfExpression || e is KtWhenExpression || e is KtLoopExpression || e is KtThrowExpression }
            .sortedWith(compareBy({ it.textRange.startOffset }, { -it.textLength }))
        val nodes = mutableListOf<JsonObject>()
        nodes += graphNode("entry", "ENTRY", null, null)
        fn.valueParameters.forEachIndexed { i, p -> nodes += graphNode("param:$i", "PARAMETER", p.name, anchor(relative, symbolId(kt.packageFqName.asString(), fn), p, kt.text)) }
        expressions.forEachIndexed { i, e -> nodes += graphNode("n:$i", graphKind(e), definedName(e), anchor(relative, symbolId(kt.packageFqName.asString(), fn), e, kt.text), usedNames(e)) }
        nodes += graphNode("exit", "EXIT", null, null)
        val edges = mutableListOf<JsonObject>(); for (i in 0 until nodes.lastIndex) edges += edge(nodes[i]["id"]!!.jsonPrimitive.content, nodes[i + 1]["id"]!!.jsonPrimitive.content, "CFG_NORMAL")
        expressions.forEachIndexed { i, e -> if (e is KtIfExpression || e is KtWhenExpression || e is KtLoopExpression) {
            if (i + 1 < expressions.size) edges += edge("n:$i", "n:${i + 1}", "CFG_TRUE")
            edges += edge("n:$i", if (i + 2 < expressions.size) "n:${i + 2}" else "exit", "CFG_FALSE")
        } }
        return buildJsonObject { put("schema", "local-cfg/0.1"); put("symbol", query); put("file", relative); putJsonArray("nodes") { nodes.forEach(::add) }; putJsonArray("edges") { edges.sortedBy { it.toString() }.forEach(::add) } }
    }

    private fun graphNode(id: String, kind: String, defines: String?, origin: JsonObject?, uses: List<String> = emptyList()) = buildJsonObject {
        put("id", id); put("kind", kind); if (defines != null) put("defines", defines); putJsonArray("uses") { uses.sorted().forEach(::add) }; if (origin != null) put("origin", origin)
    }
    private fun edge(from: String, to: String, kind: String) = buildJsonObject { put("from", from); put("to", to); put("kind", kind) }
    private fun graphKind(e: KtExpression) = when (e) { is KtReturnExpression -> "RETURN"; is KtIfExpression -> "BRANCH"; is KtWhenExpression -> "BRANCH"; is KtLoopExpression -> "LOOP"; is KtThrowExpression -> "THROW"; is KtCallExpression -> "CALL"; is KtProperty -> "DEFINITION"; is KtBinaryExpression -> if (e.operationToken.toString().contains("EQ")) "ASSIGNMENT" else "EXPRESSION"; else -> "EXPRESSION" }
    private fun definedName(e: KtExpression): String? = when (e) { is KtProperty -> e.name; is KtBinaryExpression -> if (e.operationReference.text.contains("=")) (e.left as? KtNameReferenceExpression)?.getReferencedName() else null; else -> null }
    private fun usedNames(e: PsiElement): List<String> = PsiTreeUtil.collectElementsOfType(e, KtNameReferenceExpression::class.java).map { it.getReferencedName() }.distinct()

    private fun applyEdit(request: JsonObject): JsonObject {
        val repo = Path.of(request.requiredString("repo")); val relative = request.requiredString("file"); val path = repo.resolve(relative).normalize()
        val source = request["source"]?.jsonPrimitive?.content ?: path.readText(); val kt = factory.createFile(path.fileName.toString(), source); val ownerQuery = request.requiredString("ownerSymbolId")
        val pkg = kt.packageFqName.asString(); val owner = PsiTreeUtil.collectElementsOfType(kt, KtNamedFunction::class.java).singleOrNull { symbolId(pkg, it) == ownerQuery || symbolId(pkg, it).substringBefore('(') == ownerQuery }
            ?: throw WorkerFailure("STALE_TARGET", "owner no longer resolves uniquely")
        val kind = request.requiredString("kind"); val replacement = request.requiredString("replacement")
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
        if (matches.size > 1) throw WorkerFailure("AMBIGUOUS_TARGET", "target hash resolves to ${matches.size} nodes")
        val oldEffects = effects(matches.single())
        val replacementNode = try { if (kind == "REPLACE_FUNCTION_BODY") factory.createBlock(replacement) else factory.createExpression(replacement) }
            catch (e: Throwable) { throw WorkerFailure("REPLACEMENT_PARSE_ERROR", e.message ?: "replacement parse failed") }
        val range = matches.single().textRange
        val candidate = source.substring(0, range.startOffset) + replacementNode.text + source.substring(range.endOffset)
        val candidateFile = factory.createFile(path.fileName.toString(), candidate)
        val errors = PsiTreeUtil.collectElementsOfType(candidateFile, PsiErrorElement::class.java).map { it.errorDescription }.sorted()
        if (errors.isNotEmpty()) throw WorkerFailure("REPLACEMENT_PARSE_ERROR", errors.joinToString("; "))
        val introducedEffects = effects(replacementNode) - oldEffects
        return buildJsonObject { put("schema", "semantic-candidate/0.1"); put("file", relative); put("originalHash", sha(source.toByteArray())); put("candidateHash", sha(candidate.toByteArray())); put("source", candidate); putJsonArray("diagnostics") {}; putJsonArray("introducedEffects") { introducedEffects.sorted().forEach(::add) } }
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
