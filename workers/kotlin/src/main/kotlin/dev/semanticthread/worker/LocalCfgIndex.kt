package dev.semanticthread.worker

import java.security.MessageDigest
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray

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

private val callableIdentity = Regex("^callable:[^#\\s]+#jvm:\\([^\\s]*\\)[^\\s]+$")

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
    "CALL", "CALL_RESULT", "EXPRESSION", "DEFINITION", "ASSIGNMENT" -> "OPERATION"
    "BRANCH" -> "DECISION"
    "MERGE" -> "MERGE"
    "RETURN" -> "RETURN"
    "THROW" -> "THROW"
    "CATCH" -> "CATCH"
    "FINALLY" -> "FINALLY"
    "LOOP" -> "LOOP_CONDITION"
    "LOOP_EXIT" -> "LOOP_EXIT"
    "DEAD" -> "DEAD"
    else -> null
}

private fun edgeKind(kind: String): Triple<String, Int, String?>? {
    val normalized = kind.uppercase()
    return when {
        "EXCEPTION" in normalized || "THROW" in normalized -> Triple("EXCEPTION", 4, kind)
        "RETURN" in normalized -> Triple("RETURN", 5, kind)
        "LOOP_BACK" in normalized || normalized == "BACK" || "BACKWARD" in normalized ->
            Triple("LOOP_BACK", 6, kind)
        "BREAK" in normalized -> Triple("BREAK", 7, kind)
        "CONTINUE" in normalized -> Triple("CONTINUE", 8, kind)
        "FINALLY" in normalized -> Triple("FINALLY", 9, kind)
        "DEAD" in normalized -> Triple("DEAD", 10, kind)
        "TRUE" in normalized -> Triple("TRUE", 1, kind)
        "FALSE" in normalized -> Triple("FALSE", 2, kind)
        "WHEN" in normalized || "CASE" in normalized || "NULL" in normalized ->
            Triple("WHEN_CASE", 3, kind)
        normalized == "CFG_NORMAL" || normalized == "NORMAL" || "FORWARD" in normalized ||
            "POSTPONED" in normalized -> Triple("NEXT", 0, null)
        else -> null
    }
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

    val nodes = mutableListOf<Pair<Long, String>>()
    for (rawValue in rawNodes) {
        val raw = rawValue as? JsonObject
            ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_NODE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        val idText = raw["id"]?.jsonPrimitive?.contentOrNull
            ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_NODE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        val id = idText.toLongOrNull() ?: continue
        if (id < 0) {
            return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_NODE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        }
        val role = raw["kind"]?.jsonPrimitive?.contentOrNull?.let(::nodeRole)
            ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_NODE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        nodes += id to role
    }
    nodes.sortBy { it.first }
    if (nodes.isEmpty() || nodes.zipWithNext().any { (left, right) -> left.first >= right.first }) {
        return localCfgBoundary("INVALID_LOCAL_CFG_TOPOLOGY", interactive, file, ownerSymbolIdentity, compilerGraphName)
    }
    val known = nodes.mapTo(mutableSetOf()) { it.first }

    val edges = mutableListOf<SealedEdge>()
    for (rawValue in rawEdges) {
        val raw = rawValue as? JsonObject
            ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_EDGE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        val sourceText = raw["from"]?.jsonPrimitive?.contentOrNull
            ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_EDGE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        val targetText = raw["to"]?.jsonPrimitive?.contentOrNull
            ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_EDGE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        val source = sourceText.toLongOrNull()
        val target = targetText.toLongOrNull()
        if (source == null || target == null) continue
        if (source !in known || target !in known) {
            return localCfgBoundary("INVALID_LOCAL_CFG_TOPOLOGY", interactive, file, ownerSymbolIdentity, compilerGraphName)
        }
        val rawKind = raw["kind"]?.jsonPrimitive?.contentOrNull
            ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_EDGE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        val (kind, rank, label) = edgeKind(rawKind)
            ?: return localCfgBoundary("UNSUPPORTED_LOCAL_CFG_EDGE", interactive, file, ownerSymbolIdentity, compilerGraphName)
        edges += SealedEdge(source, target, kind, rank, label)
    }
    val canonicalEdges = edges.distinct().sortedWith(
        compareBy<SealedEdge>({ it.source }, { it.target }, { it.kindRank }, { it.label ?: "" }),
    )

    val entries = nodes.filter { it.second == "ENTRY" }
    val terminals = nodes.filter { it.second == "EXIT" || it.second == "RETURN" || it.second == "THROW" }
    if (entries.size != 1 || terminals.isEmpty()) {
        return localCfgBoundary("INVALID_LOCAL_CFG_TOPOLOGY", interactive, file, ownerSymbolIdentity, compilerGraphName)
    }
    val adjacency = canonicalEdges.groupBy(SealedEdge::source)
    val reachable = mutableSetOf(entries.single().first)
    val queue = ArrayDeque<Long>().apply { add(entries.single().first) }
    while (queue.isNotEmpty()) {
        adjacency[queue.removeFirst()].orEmpty().forEach { edge ->
            if (reachable.add(edge.target)) queue.add(edge.target)
        }
    }
    if (nodes.any { (id, role) -> role != "DEAD" && id !in reachable }) {
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
            nodes.forEach { (nodeId, role) ->
                add(buildJsonObject {
                    put("nodeId", nodeId)
                    put("role", role)
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
