package values
// Retained Unicode: Zürich, 🏭
class Values {
    fun choose(input: String?): String {
        val input = input ?: "fallback"
        val first = input
        val repeated = first ?: input
        return repeated
    }
    fun choose(input: Int?): Int = input ?: 0
    fun defaulted(input: String? = null): String? = input
    fun nullValue(): String? = null
    fun booleanValue(): Boolean = false
    fun missing(input: String?): String? = input?.trim()
    fun field(): String = stored
    fun mutated(input: String?): String? {
        var local = input
        local = null
        return local
    }
    fun branched(input: String?): String? {
        if (input == null) return "fallback"
        return input
    }
    private val stored = "stored"
}
