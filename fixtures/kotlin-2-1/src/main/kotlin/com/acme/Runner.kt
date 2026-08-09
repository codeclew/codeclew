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

fun mappingContext(): Int = 2

fun applyMappingContext(value: Int, context: Int): Int = value + context

fun valuesAwaitingContext(values: List<Int>): List<Int> = values
