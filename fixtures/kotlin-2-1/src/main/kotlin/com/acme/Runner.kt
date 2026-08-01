package com.acme

fun main() {
    loadRecords().forEach(::consume)
}

fun loadRecords(): List<String> = emptyList()

private fun consume(record: String) = println(record)
