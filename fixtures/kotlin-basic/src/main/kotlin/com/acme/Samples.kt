package com.acme

fun total(base: Int, premium: Boolean): Int {
    var value = base
    if (premium) {
        value *= 2
    }
    return value
}

fun classify(value: Int): String = when {
    value < 0 -> "negative"
    value == 0 -> "zero"
    else -> "positive"
}

fun loops(limit: Int): Int {
    var result = 0
    var index = 0
    while (index < limit) {
        index++
        if (index == 2) continue
        if (index > 10) break
        result += index
    }
    do { result-- } while (result > 100)
    for (item in 0 until limit) result += item
    return result
}

fun guarded(text: String?): Int = try {
    text?.length ?: throw IllegalArgumentException("missing")
} catch (_: IllegalArgumentException) {
    0
} finally {
    Unit
}

fun String.decorate(prefix: String = "["): String = "$prefix$this]"
fun overloaded(value: Int): Int = value * 2
fun overloaded(value: String): Int = value.length
fun namedCall(value: String): String = value.decorate(prefix = "{")

fun capture(values: List<Int>): Int {
    var sum = 0
    values.forEach { sum += it }
    return sum
}

suspend fun boundary(value: Int): Int = suspendIdentity(value)
suspend fun suspendIdentity(value: Int): Int = value

class Counter(var value: Int) {
    fun increment(): Int {
        value += 1
        return value
    }
}

