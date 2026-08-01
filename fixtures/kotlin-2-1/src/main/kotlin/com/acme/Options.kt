package com.acme

data class Options(val limit: Int)

fun readOptions(): Options = Options(limit = 10)

fun applyOptions(record: String, options: Options): String =
    record.take(options.limit)
