package dev.semanticthread.worker

internal data class ConstructorOwnerAuthority(
    val compilerClassId: String,
    val ownerIdentity: String,
    val containment: List<String>,
)

internal fun constructorOwnerAuthority(
    compilerClassId: String,
    observedContainment: List<String>,
): ConstructorOwnerAuthority {
    val ownerIdentity = "class:$compilerClassId"
    return ConstructorOwnerAuthority(
        compilerClassId = compilerClassId,
        ownerIdentity = ownerIdentity,
        containment = observedContainment.filterNot { it == ownerIdentity } + ownerIdentity,
    )
}
