package concurrency

fun first(value: Int): Int = value + 1
fun second(value: Int): Int = value * 2
fun callee(value: Int): Int = value - 1
fun caller(value: Int): Int = callee(value)
