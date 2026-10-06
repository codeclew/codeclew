package dev.semanticthread.worker

import java.security.MessageDigest
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlinx.serialization.json.putJsonObject

internal data class LocalCfgSealResult(
    val graph: JsonObject? = null,
    val boundary: JsonObject? = null,
) {
    init {
        require((graph == null) != (boundary == null))
    }
}

private data class SealedEdge(
    val source: Long,
    val target: Long,
    val kind: String,
    val kindRank: Int,
    val label: String?,
)

private data class SealedNode(
    val id: Long,
    val role: String,
    val sourceStart: Long? = null,
    val sourceEnd: Long? = null,
)

private val callableIdentity = Regex("^callable:[^#\\s]+#jvm:\\([^\\s]*\\)[^\\s]+$")

private val compilerNodeKinds = setOf(
    "AnonymousFunctionCaptureNode", "AnonymousFunctionExpressionNode", "AnonymousObjectEnterNode",
    "AnonymousObjectExpressionExitNode", "BlockEnterNode", "BlockExitNode", "BooleanOperatorEnterNode",
    "BooleanOperatorEnterRightOperandNode", "BooleanOperatorExitLeftOperandNode", "BooleanOperatorExitNode",
    "CallableReferenceNode", "CatchClauseEnterNode", "CatchClauseExitNode", "CheckNotNullCallNode",
    "ClassEnterNode", "ClassExitNode", "CodeFragmentEnterNode", "CodeFragmentExitNode",
    "ComparisonExpressionNode", "DelegateExpressionExitNode", "DelegatedConstructorCallNode",
    "ElvisExitNode", "ElvisLhsExitNode", "ElvisLhsIsNotNullNode", "ElvisRhsEnterNode",
    "EnterDefaultArgumentsNode", "EnterSafeCallNode", "EnterValueParameterNode", "EqualityOperatorCallNode",
    "ExitDefaultArgumentsNode", "ExitSafeCallNode", "ExitValueParameterNode", "FakeExpressionEnterNode",
    "FieldInitializerEnterNode", "FieldInitializerExitNode", "FileEnterNode", "FileExitNode",
    "FinallyBlockEnterNode", "FinallyBlockExitNode", "FunctionCallArgumentsEnterNode",
    "FunctionCallArgumentsExitNode", "FunctionCallEnterNode", "FunctionCallExitNode", "FunctionEnterNode",
    "FunctionExitNode", "GetClassCallNode", "InitBlockEnterNode", "InitBlockExitNode", "JumpNode",
    "LiteralExpressionNode", "LocalClassExitNode", "LocalFunctionDeclarationNode", "LoopBlockEnterNode",
    "LoopBlockExitNode", "LoopConditionEnterNode", "LoopConditionExitNode", "LoopEnterNode", "LoopExitNode",
    "MergePostponedLambdaExitsNode", "PostponedLambdaExitNode", "PropertyInitializerEnterNode",
    "PropertyInitializerExitNode", "QualifiedAccessNode", "ResolvedQualifierNode", "ScriptEnterNode",
    "ScriptExitNode", "SmartCastExpressionExitNode", "SplitPostponedLambdasNode", "StringConcatenationCallNode",
    "StubNode", "ThrowExceptionNode", "TryExpressionEnterNode", "TryExpressionExitNode",
    "TryMainBlockEnterNode", "TryMainBlockExitNode", "TypeOperatorCallNode", "VariableAssignmentNode",
    "VariableDeclarationEnterNode", "VariableDeclarationExitNode", "WhenBranchConditionEnterNode",
    "WhenBranchConditionExitNode", "WhenBranchResultEnterNode", "WhenBranchResultExitNode", "WhenEnterNode",
    "WhenExitNode", "WhenSubjectExpressionExitNode", "WhenSyntheticElseBranchNode",
)

private val compilerEdgeLabels = setOf("NormalPath", "Postponed", "onUncaughtException")

private fun compilerNodeRole(kind: String, dead: Boolean): String? {
    if (kind !in compilerNodeKinds) return null
    if (dead && kind !in setOf("FunctionEnterNode", "FunctionExitNode")) return "DEAD"
    return when (kind) {
        "FunctionEnterNode" -> "ENTRY"
        "FunctionExitNode" -> "EXIT"
        "BooleanOperatorEnterNode", "BooleanOperatorEnterRightOperandNode",
        "BooleanOperatorExitLeftOperandNode", "BooleanOperatorExitNode", "ElvisLhsExitNode",
        "ElvisLhsIsNotNullNode", "LoopConditionEnterNode", "LoopConditionExitNode",
        "WhenBranchConditionEnterNode", "WhenBranchConditionExitNode" -> "DECISION"
        "CatchClauseEnterNode", "CatchClauseExitNode" -> "CATCH"
        "FinallyBlockEnterNode", "FinallyBlockExitNode" -> "FINALLY"
        "LoopExitNode" -> "LOOP_EXIT"
        "MergePostponedLambdaExitsNode" -> "MERGE"
        "ThrowExceptionNode" -> "THROW"
        else -> "OPERATION"
    }
}

internal fun normalizeRetainedCompilerFirCfg(
    raw: JsonObject,
    ownerSymbolIdentity: String,
    file: String,
    functionStart: Int,
    functionEnd: Int,
    source: String,
): JsonObject {
    val identity = Regex("^callable:(.+)#jvm:(.+)$").matchEntire(ownerSymbolIdentity)
        ?: error("invalid compiler CFG owner identity")
    if (raw["symbol"]?.jsonPrimitive?.contentOrNull != identity.groupValues[1] ||
        raw["jvmDescriptor"]?.jsonPrimitive?.contentOrNull != identity.groupValues[2] ||
        raw["file"]?.jsonPrimitive?.contentOrNull != file ||
        raw["start"]?.jsonPrimitive?.intOrNull != functionStart ||
        raw["end"]?.jsonPrimitive?.intOrNull != functionEnd ||
        functionStart < 0 || functionEnd <= functionStart || functionEnd > source.length
    ) {
        error("compiler CFG owner or source bounds do not match the retained function")
    }
    val coordinates = CompilerUtf16ToUtf8ByteMap.from(source)
        ?: error("compiler CFG source cannot be mapped to UTF-8 bytes")
    val rawNodes = raw["nodes"]?.jsonArray ?: error("compiler CFG nodes are missing")
    val rawEdges = raw["edges"]?.jsonArray ?: error("compiler CFG edges are missing")
    if (rawNodes.isEmpty() || rawNodes.size > 4_096 || rawEdges.size > 8_192) {
        error("compiler CFG exceeds the retained graph budget")
    }
    val nodesById = mutableMapOf<Int, JsonObject>()
    val nodes = rawNodes.map { value ->
        val node = value as? JsonObject ?: error("compiler CFG node is not an object")
        val id = node["id"]?.jsonPrimitive?.intOrNull?.takeIf { it >= 0 }
            ?: error("compiler CFG node id is invalid")
        if (nodesById.put(id, node) != null) error("compiler CFG node id is duplicated")
        val rawKind = node["kind"]?.jsonPrimitive?.contentOrNull ?: error("compiler CFG node kind is missing")
        val dead = node["dead"]?.jsonPrimitive?.booleanOrNull
            ?: error("compiler CFG node dead flag is missing or invalid")
        val role = compilerNodeRole(rawKind, dead) ?: error("compiler CFG node kind is unsupported")
        val rawStart = node["start"]
        val rawEnd = node["end"]
        if ((rawStart == null) != (rawEnd == null)) error("compiler CFG node source range is partial")
        val startValue = rawStart?.jsonPrimitive?.intOrNull
        val endValue = rawEnd?.jsonPrimitive?.intOrNull
        if (rawStart != null && (startValue == null || endValue == null)) {
            error("compiler CFG node source range is malformed")
        }
        val startByte = if (startValue != null && endValue != null) {
            if (startValue < functionStart || endValue > functionEnd || endValue <= startValue) {
                error("compiler CFG node source range is outside the owning function")
            }
            coordinates.offset(startValue)?.toLong() ?: error("compiler CFG node start splits a UTF-16 surrogate pair")
        } else null
        val endByte = if (startValue != null && endValue != null) {
            coordinates.offset(endValue)?.toLong() ?: error("compiler CFG node end splits a UTF-16 surrogate pair")
        } else null
        buildJsonObject {
            put("id", id)
            put("kind", role)
            if (startByte != null && endByte != null) putJsonObject("source") {
                put("start", startByte)
                put("end", endByte)
            }
        }
    }
    val edges = rawEdges.mapNotNull { value ->
        val edge = value as? JsonObject ?: error("compiler CFG edge is not an object")
        val from = edge["from"]?.jsonPrimitive?.intOrNull ?: error("compiler CFG edge source is missing")
        val to = edge["to"]?.jsonPrimitive?.intOrNull ?: error("compiler CFG edge target is missing")
        if (from !in nodesById || to !in nodesById) error("compiler CFG edge has a dangling endpoint")
        val rawEdgeKind = edge["edgeKind"]?.jsonPrimitive?.contentOrNull ?: error("compiler CFG edge kind is missing")
        val label = edge["label"]?.jsonPrimitive?.contentOrNull
            ?.takeIf { it in compilerEdgeLabels }
            ?: error("compiler CFG edge label is unsupported")
        if (rawEdgeKind == "DfgForward" || rawEdgeKind == "DeadDfgForward") return@mapNotNull null
        val normalizedKind = when (rawEdgeKind) {
            "Forward", "CfgForward" -> if (label == "onUncaughtException") "CFG_EXCEPTION" else "CFG_NORMAL"
            "DeadForward", "DeadCfgForward", "DeadCfgBackward" -> "CFG_DEAD"
            "CfgBackward" -> "CFG_BACK"
            else -> error("compiler CFG edge kind is unsupported: $rawEdgeKind")
        }
        buildJsonObject {
            put("from", from)
            put("to", to)
            put("kind", normalizedKind)
            put("label", label)
        }
    }
    return buildJsonObject {
        put("schema", "local-cfg/0.1")
        put("graphSource", "K2_FIR_CFG")
        putJsonArray("nodes") { nodes.forEach(::add) }
        putJsonArray("edges") { edges.forEach(::add) }
    }
}

private fun canonicalLocalCfgJson(value: JsonElement): String = when (value) {
    is JsonObject -> value.entries.sortedBy { it.key }.joinToString(separator = ",", prefix = "{", postfix = "}") { (key, child) ->
        "${JsonPrimitive(key)}:${canonicalLocalCfgJson(child)}"
    }
    is JsonArray -> value.joinToString(separator = ",", prefix = "[", postfix = "]", transform = ::canonicalLocalCfgJson)
    else -> value.toString()
}

private fun localCfgDigest(value: JsonElement): String = "sha256:" +
    MessageDigest.getInstance("SHA-256")
        .digest(canonicalLocalCfgJson(value).toByteArray(Charsets.UTF_8))
        .joinToString("") { "%02x".format(it) }

private fun safeLocalCfgFile(file: String): Boolean =
    file.isNotBlank() && !file.startsWith('/') && !file.contains('\\') &&
        file.split('/').all { it.isNotBlank() && it != "." && it != ".." }

private fun localCfgBoundary(
    code: String,
    raw: JsonElement,
    file: String?,
    owner: String?,
    compilerGraphName: String?,
): LocalCfgSealResult = LocalCfgSealResult(
    boundary = buildJsonObject {
        put("schema", "local-cfg-boundary/0.1")
        file?.takeIf(::safeLocalCfgFile)?.let { put("file", it) }
        owner?.takeIf(callableIdentity::matches)?.let { put("ownerSymbolIdentity", it) }
        compilerGraphName?.takeIf { it.isNotBlank() && it.length <= 512 }?.let { put("compilerGraphName", it) }
        put("stage", "NORMALIZE")
        put("code", code)
        put("resolution", "UNKNOWN")
        put("provider", "K2_FIR_CFG")
        put("sourceProvenance", "COMPILER_UTF16_RANGE_TO_UTF8_BYTES")
        put("rawRowHash", localCfgDigest(raw))
    },
)

internal fun unknownCompilerLocalCfg(
    code: String,
    raw: JsonElement,
    file: String? = null,
    owner: String? = null,
    compilerGraphName: String? = null,
): LocalCfgSealResult = localCfgBoundary(code, raw, file, owner, compilerGraphName)

private fun nodeRole(kind: String): String? = when (kind) {
    "ENTRY" -> "ENTRY"
    "EXIT", "EXCEPTION_EXIT" -> "EXIT"
    "CALL", "CALL_RESULT", "EXPRESSION", "DEFINITION", "ASSIGNMENT", "OPERATION" -> "OPERATION"
    "BRANCH", "DECISION" -> "DECISION"
    "MERGE" -> "MERGE"
    "RETURN" -> "RETURN"
    "THROW" -> "THROW"
    "CATCH" -> "CATCH"
    "FINALLY" -> "FINALLY"
    "LOOP", "LOOP_CONDITION" -> "LOOP_CONDITION"
    "LOOP_EXIT" -> "LOOP_EXIT"
    "DEAD" -> "DEAD"
    else -> null
}

private fun edgeKind(kind: String, label: String?): Triple<String, Int, String?>? {
    val rankAndKind = when (kind) {
        "CFG_NORMAL" -> "NEXT" to 0
        "CFG_TRUE" -> "TRUE" to 1
        "CFG_FALSE" -> "FALSE" to 2
        "CFG_WHEN_CASE" -> "WHEN_CASE" to 3
        "CFG_EXCEPTION" -> "EXCEPTION" to 4
        "CFG_RETURN" -> "RETURN" to 5
        "CFG_BACK" -> "LOOP_BACK" to 6
        "CFG_BREAK" -> "BREAK" to 7
        "CFG_CONTINUE" -> "CONTINUE" to 8
        "CFG_FINALLY" -> "FINALLY" to 9
        "CFG_DEAD" -> "DEAD" to 10
        else -> return null
    }
    return Triple(rankAndKind.first, rankAndKind.second, label)
}

internal fun sealCompilerLocalCfg(
    interactive: JsonObject,
    ownerSymbolIdentity: String?,
    file: String,
    compilerGraphName: String?,
): LocalCfgSealResult {
    if (interactive["schema"]?.jsonPrimitive?.contentOrNull != "local-cfg/0.1" ||
        interactive["graphSource"]?.jsonPrimitive?.contentOrNull != "K2_FIR_CFG" ||
        ownerSymbolIdentity == null || !callableIdentity.matches(ownerSymbolIdentity) ||
        !safeLocalCfgFile(file) || compilerGraphName.isNullOrBlank() || compilerGraphName.length > 512
    ) {
        return localCfgBoundary(
            "INVALID_LOCAL_CFG_IDENTITY",
            interactive,
            file,
            ownerSymbolIdentity,
            compilerGraphName,
        )
    }

    val rawNodes = interactive["nodes"]?.jsonArray
        ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_NODE", interactive, file, ownerSymbolIdentity, compilerGraphName)
    val rawEdges = interactive["edges"]?.jsonArray
        ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_EDGE", interactive, file, ownerSymbolIdentity, compilerGraphName)
    if (rawNodes.size > 4_096 || rawEdges.size > 8_192) {
        return localCfgBoundary("LOCAL_CFG_BUDGET_EXCEEDED", interactive, file, ownerSymbolIdentity, compilerGraphName)
    }

    val nodes = mutableListOf<SealedNode>()
    for (rawValue in rawNodes) {
        val raw = rawValue as? JsonObject
            ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_NODE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        val idText = raw["id"]?.jsonPrimitive?.contentOrNull
            ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_NODE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        val id = idText.toLongOrNull()
            ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_NODE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        if (id < 0) {
            return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_NODE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        }
        val role = raw["kind"]?.jsonPrimitive?.contentOrNull?.let(::nodeRole)
            ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_NODE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        val sourceValue = raw["source"]
        val source = sourceValue as? JsonObject
        if (sourceValue != null && source == null) {
            return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_NODE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        }
        val sourceStart = source?.get("start")?.jsonPrimitive?.longOrNull
        val sourceEnd = source?.get("end")?.jsonPrimitive?.longOrNull
        if (source != null && (sourceStart == null || sourceEnd == null || sourceEnd <= sourceStart)) {
            return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_NODE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        }
        nodes += SealedNode(id, role, sourceStart, sourceEnd)
    }
    nodes.sortBy(SealedNode::id)
    if (nodes.isEmpty() || nodes.zipWithNext().any { (left, right) -> left.id >= right.id }) {
        return localCfgBoundary("INVALID_LOCAL_CFG_TOPOLOGY", interactive, file, ownerSymbolIdentity, compilerGraphName)
    }
    val known = nodes.mapTo(mutableSetOf()) { it.id }

    val edges = mutableListOf<SealedEdge>()
    for (rawValue in rawEdges) {
        val raw = rawValue as? JsonObject
            ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_EDGE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        val sourceText = raw["from"]?.jsonPrimitive?.contentOrNull
            ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_EDGE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        val targetText = raw["to"]?.jsonPrimitive?.contentOrNull
            ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_EDGE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        val source = sourceText.toLongOrNull()
            ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_EDGE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        val target = targetText.toLongOrNull()
            ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_EDGE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        if (source !in known || target !in known) {
            return localCfgBoundary("INVALID_LOCAL_CFG_TOPOLOGY", interactive, file, ownerSymbolIdentity, compilerGraphName)
        }
        val rawKind = raw["kind"]?.jsonPrimitive?.contentOrNull
            ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_EDGE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        val label = raw["label"]?.jsonPrimitive?.contentOrNull
        val (kind, rank, normalizedLabel) = edgeKind(rawKind, label)
            ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_EDGE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        if (label != null && (label.isBlank() || label.length > 512)) {
            return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_EDGE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        }
        edges += SealedEdge(source, target, kind, rank, normalizedLabel)
    }
    val canonicalEdges = edges.distinct().sortedWith(
        compareBy<SealedEdge>({ it.source }, { it.target }, { it.kindRank }, { it.label ?: "" }),
    )

    val entries = nodes.filter { it.role == "ENTRY" }
    val terminals = nodes.filter { it.role == "EXIT" || it.role == "RETURN" || it.role == "THROW" }
    if (entries.size != 1 || terminals.isEmpty()) {
        return localCfgBoundary("INVALID_LOCAL_CFG_TOPOLOGY", interactive, file, ownerSymbolIdentity, compilerGraphName)
    }
    val adjacency = canonicalEdges.groupBy(SealedEdge::source)
    val reachable = mutableSetOf(entries.single().id)
    val queue = ArrayDeque<Long>().apply { add(entries.single().id) }
    while (queue.isNotEmpty()) {
        adjacency[queue.removeFirst()].orEmpty().forEach { edge ->
            if (reachable.add(edge.target)) queue.add(edge.target)
        }
    }
    if (nodes.any { it.role != "DEAD" && it.id !in reachable }) {
        return localCfgBoundary("INVALID_LOCAL_CFG_TOPOLOGY", interactive, file, ownerSymbolIdentity, compilerGraphName)
    }

    fun payload(graphId: String) = buildJsonObject {
        put("schema", "local-cfg/0.1")
        put("graphId", graphId)
        put("ownerSymbolIdentity", ownerSymbolIdentity)
        put("file", file)
        put("compilerGraphName", compilerGraphName)
        put("provider", "K2_FIR_CFG")
        put("sourceProvenance", "COMPILER_UTF16_RANGE_TO_UTF8_BYTES")
        putJsonArray("nodes") {
            nodes.forEach { node ->
                add(buildJsonObject {
                    put("nodeId", node.id)
                    put("role", node.role)
                    if (node.sourceStart != null && node.sourceEnd != null) putJsonObject("source") {
                        put("start", node.sourceStart)
                        put("end", node.sourceEnd)
                    }
                })
            }
        }
        putJsonArray("edges") {
            canonicalEdges.forEach { edge ->
                add(buildJsonObject {
                    put("sourceNodeId", edge.source)
                    put("targetNodeId", edge.target)
                    put("kind", edge.kind)
                    edge.label?.let { put("label", it) }
                })
            }
        }
    }
    val unsigned = payload("")
    return LocalCfgSealResult(graph = payload(localCfgDigest(unsigned)))
}

internal fun attachCompilerLocalCfgSnapshot(
    index: JsonObject,
    results: List<LocalCfgSealResult>,
): JsonObject {
    val boundaries = results.mapNotNull(LocalCfgSealResult::boundary).toMutableList()
    val grouped = results.mapNotNull(LocalCfgSealResult::graph)
        .groupBy { it["ownerSymbolIdentity"]!!.jsonPrimitive.content }
    val graphs = mutableListOf<JsonObject>()
    grouped.toSortedMap().forEach { (owner, rows) ->
        if (rows.size == 1) {
            graphs += rows.single()
        } else {
            rows.forEach { row ->
                boundaries += localCfgBoundary(
                    "DUPLICATE_LOCAL_CFG_OWNER",
                    row,
                    row["file"]?.jsonPrimitive?.contentOrNull,
                    owner,
                    row["compilerGraphName"]?.jsonPrimitive?.contentOrNull,
                ).boundary!!
            }
        }
    }
    val sortedGraphs = graphs.sortedBy(::canonicalLocalCfgJson)
    val sortedBoundaries = boundaries.distinctBy(::canonicalLocalCfgJson).sortedBy(::canonicalLocalCfgJson)
    val snapshot = buildJsonObject {
        putJsonArray("graphs") { sortedGraphs.forEach(::add) }
        putJsonArray("boundaries") { sortedBoundaries.forEach(::add) }
    }
    return buildJsonObject {
        index.forEach { (key, value) ->
            if (key !in setOf("localCfgs", "localCfgBoundaries", "localCfgHash")) put(key, value)
        }
        putJsonArray("localCfgs") { sortedGraphs.forEach(::add) }
        putJsonArray("localCfgBoundaries") { sortedBoundaries.forEach(::add) }
        put("localCfgHash", localCfgDigest(snapshot))
    }
}
