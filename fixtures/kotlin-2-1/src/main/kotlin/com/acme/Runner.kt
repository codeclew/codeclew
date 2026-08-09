package com.acme

fun main() {
    loadRecords().forEach(::consume)
}

fun loadRecords(): List<String> = emptyList()

private fun consume(record: String) = println(record)

fun transformAndConsume(input: Int): Int {
    val transformed = input * 2
    return transformed
}
