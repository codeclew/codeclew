package example.product

import example.shared.OutboxEvent

@Target(AnnotationTarget.FUNCTION)
annotation class Transactional

data class SaveProductCommand(val sku: String)
data class Product(val sku: String)
class DuplicateProduct(sku: String) : IllegalStateException("duplicate product: $sku")

interface ProductRepository {
    fun existsBySku(sku: String): Boolean
    fun save(product: Product): Product
}

interface OutboxRepository {
    fun save(event: OutboxEvent): OutboxEvent
}

class ProductService(
    private val products: ProductRepository,
    private val outbox: OutboxRepository,
) {
    @Transactional
    fun saveProduct(command: SaveProductCommand): Product {
        if (products.existsBySku(command.sku)) {
            throw DuplicateProduct(command.sku)
        }
        val product = Product(command.sku)
        products.save(product)
        outbox.save(OutboxEvent("product-saved", product.sku))
        return product
    }
}
