package example

import example.missing.ExternalNotifier

data class Request(val sku: String?, val quantity: Int)

object Defaults {
    const val LIMIT = 100
}

fun String.normalizedSku(): String = trim().lowercase()

class Reservations(private val notifier: ExternalNotifier) {
    private val values = mutableMapOf<String, Int>()

    @Deprecated("Synthetic documentation fixture")
    fun reserve(request: Request): String {
        val sku = request.sku?.normalizedSku()?.takeIf { it.isNotEmpty() }
            ?: return "missing sku"
        val quantity = when {
            request.quantity <= 0 -> return "invalid quantity"
            request.quantity > Defaults.LIMIT -> return "insufficient stock"
            else -> request.quantity
        }
        values[sku] = quantity
        notifier.defer { values[sku] = 0 }
        return "reserved: café"
    }

    fun batch(requests: List<Request>): List<String> {
        val results = mutableListOf<String>()
        for (request in requests) {
            results.add(reserve(request))
        }
        return results
    }

    suspend fun submit(request: Request): String = reserve(request)
}
