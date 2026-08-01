package com.acme.archive

data class ProductIdentity(
    val id: String,
    val code: String?,
    val title: String,
)

class ArchiveService {
    fun archiveEvent(product: ProductIdentity): String =
        "${product.id}:${product.code}:${product.title}"
}
