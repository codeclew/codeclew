package sample

// Synthetic local fixture. Its traces establish only these configured runs.
data class Request(val jobKey: String, val blocked: Boolean = false, val marked: Boolean = false)
data class Artifact(val artifactKey: String?, val lineCount: Int)
data class Reply(val requestKey: String, val version: String, val artifactKey: String, val lines: Int)
class BlockedRequest : RuntimeException()
class MissingVersion : RuntimeException()
class EngineUnavailable : RuntimeException()
class EmptyReport : RuntimeException()
class LocalWriteFailed : RuntimeException()

interface VersionIndex {
    fun getForEntity(key: String): String?
}
class LocalVersionIndex(private val events: MutableList<String>, private val available: Boolean) : VersionIndex {
    override fun getForEntity(key: String): String? {
        events.add("lookup.local")
        return if (available) "V1" else null
    }
}

interface Collector {
    fun collect(request: Request, preview: Boolean = true)
    fun collectForReport(request: Request) = collect(request, preview = false)
}
class AuditCollector(private val events: MutableList<String>, private val failWrite: Boolean) : Collector {
    override fun collect(request: Request, preview: Boolean) {
        if (!preview) {
            try {
                events.add("audit.write.attempt")
                if (failWrite) throw LocalWriteFailed()
                events.add("audit.write.accepted")
            } catch (error: LocalWriteFailed) {
                events.add("audit.write.failed")
            }
        }
    }
}
class HistoryCollector(private val events: MutableList<String>) : Collector {
    override fun collect(request: Request, preview: Boolean) {
        if (preview) events.add("history.read")
    }
}

interface Preparer {
    fun prepare(request: Request, callback: (Request) -> Request): Request
}
class OptionalPreparer(private val events: MutableList<String>, private val invokeCallback: Boolean) : Preparer {
    override fun prepare(request: Request, callback: (Request) -> Request): Request {
        events.add("prepare.call")
        val prepared = if (invokeCallback) callback(request) else request
        events.add("prepare.return")
        return prepared
    }
}

interface Engine {
    fun calculate(request: Request, version: String): Artifact?
}
class LocalEngine(
    private val events: MutableList<String>,
    private val backendAvailable: Boolean,
    private val missingResult: Boolean,
    private val missingId: Boolean,
    private val lines: Int
) : Engine {
    override fun calculate(request: Request, version: String): Artifact? {
        events.add("engine.call")
        events.add("backend.fetch")
        val backend = if (backendAvailable) "local" else null
        if (backend == null) throw EngineUnavailable()
        if (missingResult) return null
        return Artifact(if (missingId) null else "ART-9", lines)
    }
}

class ReportService(
    private val lookupClient: VersionIndex,
    private val collectors: Set<Collector>,
    private val preparer: Preparer,
    private val engine: Engine,
    private val events: MutableList<String>
) {
    fun generate(request: Request, versionId: String? = null): Reply? {
        events.add("request.guard")
        if (request.blocked) throw BlockedRequest()
        val version = versionId ?: lookupClient.getForEntity("active") ?: throw MissingVersion()
        events.add("version.ready")
        collectors.forEach { it.collectForReport(request) }
        val prepared = preparer.prepare(request) { value ->
            events.add("callback.enrich")
            collectors.forEach { it.collect(value, preview = false) }
            value.copy(marked = true)
        }
        collectors.forEach { it.collect(prepared) }
        val result = engine.calculate(prepared, version)
        if (result == null) {
            events.add("result.null")
            return null
        }
        val artifactId = result.artifactKey
        if (artifactId == null) {
            events.add("id.missing")
            return null
        }
        if (result.lineCount < 1) throw EmptyReport()
        events.add("reply.map")
        return Reply(request.jobKey, version, artifactId, result.lineCount)
    }
}

fun main() {
    val scenarios = listOf("success", "blocked", "no-version", "null-result", "missing-id",
        "callback-skipped", "backend-failure", "empty-report", "write-failure", "explicit-version")
    for (scenario in scenarios) {
        val events = mutableListOf<String>()
        val service = ReportService(
            LocalVersionIndex(events, scenario != "no-version"),
            linkedSetOf(AuditCollector(events, scenario == "write-failure"), HistoryCollector(events)),
            OptionalPreparer(events, scenario != "callback-skipped"),
            LocalEngine(events, scenario != "backend-failure", scenario == "null-result",
                scenario == "missing-id", if (scenario == "empty-report") 0 else 12), events
        )
        val request = Request("JOB-7", blocked = scenario == "blocked")
        val outcome = try {
            val reply = if (scenario == "explicit-version") service.generate(request, "MANUAL") else service.generate(request)
            if (reply == null) "null" else "reply:${reply.requestKey},${reply.version},${reply.artifactKey},${reply.lines}"
        } catch (error: RuntimeException) {
            "throw:${error.javaClass.simpleName}"
        }
        println("TRACE\t$scenario\t$outcome\t${events.joinToString("|")}")
    }
}
