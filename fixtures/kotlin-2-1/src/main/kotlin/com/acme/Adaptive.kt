package com.acme

import kotlinx.serialization.Serializable

@Serializable
data class MigrationJob(val batchSize: Int)

fun MigrationJob.applyAdaptive(limit: Int): MigrationJob =
    copy(batchSize = minOf(batchSize, limit))
