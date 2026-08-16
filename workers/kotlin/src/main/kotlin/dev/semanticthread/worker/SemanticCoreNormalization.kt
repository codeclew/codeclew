package dev.semanticthread.worker

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

private val OPTIONAL_DESCRIPTOR_ATTRIBUTE_BOUNDARIES = setOf(
    "UNKNOWN_VISIBILITY",
    "UNKNOWN_EFFECTIVE_VISIBILITY",
    "UNKNOWN_MODALITY",
    "UNRESOLVED_DESCRIPTOR_TYPE",
)

private val RETAINED_CALL_TOPOLOGY_KINDS = setOf(
    "CALLS",
    "CONSTRUCTS",
)

private val PARTIAL_DESCRIPTOR_CORE_FIELDS = setOf(
    "schema",
    "symbolIdentity",
    "declarationKind",
    "ownerIdentity",
    "containment",
    "resolution",
    "provider",
    "compilerCallableId",
    "compilerClassId",
    "jvmDescriptor",
    "isOverride",
    "isPrimary",
)

internal fun isOptionalDescriptorAttributeBoundary(reason: String?): Boolean =
    reason in OPTIONAL_DESCRIPTOR_ATTRIBUTE_BOUNDARIES

internal fun isRetainedCallTopologyKind(kind: String): Boolean =
    kind in RETAINED_CALL_TOPOLOGY_KINDS

internal fun descriptorCorePayload(raw: JsonObject, sourceRowHash: String): JsonObject = buildJsonObject {
    raw.entries.sortedBy { it.key }.forEach { (key, value) ->
        when (key) {
            "schema", "symbolIdentity", "declarationKind", "ownerIdentity", "containment",
            "resolution", "provider", "compilerCallableId", "compilerClassId", "jvmDescriptor" -> put(key, value)
        }
    }
    put("attributeCoverage", "PARTIAL")
    put("sourceRowHash", sourceRowHash)
}

internal fun relationCorePayload(raw: JsonObject, sourceRowHash: String): JsonObject = buildJsonObject {
    raw.entries.sortedBy { it.key }.forEach { (key, value) ->
        when (key) {
            "schema", "kind", "owner", "target", "resolution", "provider" -> put(key, value)
        }
    }
    put("attributeCoverage", "PARTIAL")
    put("sourceRowHash", sourceRowHash)
}
