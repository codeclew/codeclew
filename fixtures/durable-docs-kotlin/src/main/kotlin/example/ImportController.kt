package example

import org.springframework.kafka.annotation.KafkaListener
import org.springframework.kafka.core.KafkaTemplate
import org.springframework.web.bind.annotation.PostMapping
import org.springframework.web.bind.annotation.RequestBody
import org.springframework.web.bind.annotation.RequestMapping
import org.springframework.web.bind.annotation.RestController

data class StockImport(val warehouse: String, val quantity: Int)

@RestController
@RequestMapping("/imports")
class ImportController(private val publisher: StockPublisher) {
    @PostMapping("/stock")
    fun importStock(@RequestBody request: StockImport): String {
        if (request.quantity < 0) return "Rejected: stock cannot be negative"
        publisher.publish(request)
        return "Accepted for warehouse processing"
    }
}

class StockPublisher(private val kafka: KafkaTemplate<String, StockImport>) {
    fun publish(request: StockImport) {
        kafka.send("stock-import", request.warehouse, request)
    }
}

class StockConsumer {
    @KafkaListener(topics = ["stock-import"], groupId = "warehouses")
    fun receive(request: StockImport): String = request.warehouse
}
