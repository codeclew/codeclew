package dev.semanticthread.worker

private const val PROTOCOL_MAJOR = 1L
private const val PROTOCOL_MINOR = 0L

fun main() {
    Worker().use { worker ->
        Proto.writeFrame(System.out, response(0, capabilities = true))
        while (true) {
            val frame = Proto.readFrame(System.`in`) ?: break
            val fields = Proto.fields(frame)
            val requestId = fields.firstOrNull { it.number == 1 }?.value ?: 0
            val payloadField = fields.firstOrNull { it.number in 10..17 }
                ?: throw IllegalArgumentException("request has no payload")
            val kind = payloadField.number - 8
            val payload = Proto.fields(payloadField.bytes).firstOrNull { it.number == 1 }?.bytes ?: byteArrayOf()
            if (kind == 9) {
                Proto.writeFrame(System.out, response(requestId, payload = "{\"shutdown\":true}", responseField = 19))
                break
            }
            val encoded = try {
                response(requestId, payload = worker.handle(kind, payload), responseField = kind + 9)
            } catch (e: WorkerFailure) {
                response(requestId, errorCode = e.code, errorMessage = e.message)
            } catch (e: Throwable) {
                response(requestId, errorCode = "INCOMPLETE_SEMANTIC_ANALYSIS", errorMessage = e.message ?: e::class.simpleName.orEmpty())
            }
            Proto.writeFrame(System.out, encoded)
        }
    }
}

private fun version() = Proto.message(Proto.uint(1, PROTOCOL_MAJOR), Proto.uint(2, PROTOCOL_MINOR))

private fun capabilities(): ByteArray {
    val supported = listOf(
        "kotlin.project.inspect", "kotlin.index.declarations", "kotlin.resolve.symbols",
        "kotlin.resolve.expressions", "kotlin.cfg.local", "kotlin.edit.replace_expression",
        "kotlin.edit.replace_function_body", "kotlin.validate.copied_file"
    )
    val features = listOf("functions", "locals", "assignments", "if", "when", "loops", "return", "throw", "calls", "safe_calls", "elvis")
    val unsupported = listOf("android", "multiplatform", "scripts", "expect_actual", "reflection", "compiler_plugins", "precise_coroutine_state_machine")
    return Proto.message(
        Proto.string(1, "kotlin"), Proto.string(2, "0.1.0"), Proto.string(3, "2.4.10"), Proto.bytes(4, version()),
        *supported.map { Proto.string(5, it) }.toTypedArray(), *features.map { Proto.string(6, it) }.toTypedArray(),
        *unsupported.map { Proto.string(7, it) }.toTypedArray()
    )
}

private fun response(requestId: Long, payload: String? = null, capabilities: Boolean = false, errorCode: String? = null, errorMessage: String = "", responseField: Int = 0): ByteArray {
    val fields = mutableListOf(Proto.uint(1, requestId), Proto.bytes(2, version()))
    if (capabilities) fields += Proto.bytes(10, capabilities())
    if (payload != null) fields += Proto.bytes(responseField, Proto.bytes(1, payload.toByteArray()))
    if (errorCode != null) fields += Proto.bytes(18, Proto.message(Proto.string(1, errorCode), Proto.string(2, errorMessage), Proto.uint(3, 0)))
    return Proto.message(*fields.toTypedArray())
}
