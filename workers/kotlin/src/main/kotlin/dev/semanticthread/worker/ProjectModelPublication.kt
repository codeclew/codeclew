package dev.semanticthread.worker

internal enum class ProjectModelInvalidReason {
    MISSING_SEMANTIC_INPUT_MANIFEST_HASH,
    INVALID_SEMANTIC_INPUT_MANIFEST_HASH,
    SEMANTIC_INPUT_MANIFEST_HASH_MISMATCH,
    MISSING_SEMANTIC_INPUT_MANIFEST,
    MODEL_INPUTS_MANIFEST_MISMATCH,
    JDK_FINGERPRINT_MANIFEST_MISMATCH,
    MODEL_INPUTS_INVALID,
    RESOURCE_IDENTITIES_INVALID,
    JDK_HOME_INVALID,
    JDK_HOME_MISMATCH,
    JDK_FINGERPRINT_MISSING,
    JDK_FINGERPRINT_INVALID,
}

internal data class ProjectModelPublishResult(
    val outcome: PersistentProjectModelCache.PublishOutcome,
    val invalidReason: ProjectModelInvalidReason? = null,
)