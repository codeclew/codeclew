package calls

fun String.decorate(prefix: String = "["): String = "$prefix$this]"
fun overloaded(value: Int): Int = value * 2
fun overloaded(value: String): Int = value.length
fun selected(value: String): String = value.decorate(prefix = "{")
fun captured(values: List<Int>): Int {
    var sum = 0
    values.forEach { sum += it }
    return sum
}
fun javaBoundary(value: String): Int = java.util.Objects.hash(value)
suspend fun suspendBoundary(value: Int): Int = suspendIdentity(value)
suspend fun suspendIdentity(value: Int): Int = value

class KotlinFormatter {
    fun format(input: String): String = input.trim()
}
