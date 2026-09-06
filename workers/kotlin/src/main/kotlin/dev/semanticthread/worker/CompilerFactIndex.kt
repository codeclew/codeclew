package dev.semanticthread.worker

import java.nio.file.Path
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonPrimitive

/** Per-analysis lookup tables. Paths are exact repository-relative identities. */
internal class CompilerFactIndex(private val repo: Path, facts: List<JsonObject>) {
    private val semanticByFile: Map<String, List<JsonObject>>
    private val cfgByFile: Map<String, List<JsonObject>>
    private val cfgByRange: Map<String, Map<Pair<Int?, Int?>, JsonObject>>

    init {
        val semantic = mutableMapOf<String, MutableList<JsonObject>>()
        val cfg = mutableMapOf<String, MutableList<JsonObject>>()
        for (fact in facts) {
            val destination = when (fact["recordType"]?.jsonPrimitive?.contentOrNull) {
                "SEMANTIC_FACT" -> semantic
                "FIR_CFG" -> cfg
                else -> continue
            }
            val raw = fact["file"]?.jsonPrimitive?.contentOrNull ?: continue
            val file = repositoryRelativeCompilerPath(repo, raw.replace('\\', '/')) ?: continue
            destination.getOrPut(file) { mutableListOf() }.add(fact)
        }
        semanticByFile = semantic.mapValues { (_, rows) ->
            // Serialization is only the final tie-breaker, computed once per row.
            rows.map { it to it.toString() }.sortedWith(compareBy(
                { it.first["start"]?.jsonPrimitive?.intOrNull ?: -1 },
                { it.first["end"]?.jsonPrimitive?.intOrNull ?: -1 },
                { it.second },
            )).map { it.first }
        }
        cfgByFile = cfg
        cfgByRange = cfg.mapValues { (_, rows) ->
            buildMap {
                for (row in rows) {
                    val key = row["start"]?.jsonPrimitive?.intOrNull to row["end"]?.jsonPrimitive?.intOrNull
                    // Match the previous firstOrNull behavior for duplicate ranges.
                    if (key !in this) put(key, row)
                }
            }
        }
    }

    private fun relative(path: Path): String? = repositoryRelativeCompilerPath(repo, path.toString())

    fun semanticFacts(path: Path): List<JsonObject> = semanticByFile[relative(path)].orEmpty()

    fun cfgRecords(path: Path): List<JsonObject> = cfgByFile[relative(path)].orEmpty()

    fun cfgAt(path: Path, start: Int, end: Int): JsonObject? = cfgByRange[relative(path)]?.get(start to end)

    fun containedSemanticFacts(path: Path, start: Int, end: Int): List<JsonObject> {
        val rows = semanticFacts(path)
        var low = 0
        var high = rows.size
        while (low < high) {
            val middle = low + (high - low) / 2
            if ((rows[middle]["start"]?.jsonPrimitive?.intOrNull ?: -1) < start) low = middle + 1
            else high = middle
        }
        // Keep the original containment predicate, including malformed end offsets.
        return rows.subList(low, rows.size).filter {
            (it["end"]?.jsonPrimitive?.intOrNull ?: -1) <= end
        }
    }
}
