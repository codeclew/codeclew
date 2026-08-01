package flow

fun branches(value: Int, enabled: Boolean): Int {
    var result = value
    if (enabled) result *= 2 else result--
    result = when {
        result < 0 -> 0
        result == 0 -> 1
        else -> result
    }
    return result
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

fun shortCircuit(left: Boolean, right: () -> Boolean) = left && right()
