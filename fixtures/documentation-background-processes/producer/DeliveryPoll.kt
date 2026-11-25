import org.springframework.kafka.core.KafkaTemplate
import org.springframework.scheduling.annotation.Scheduled
import org.springframework.transaction.support.TransactionTemplate

class DeliveryPoll(
    private val tx: TransactionTemplate,
    private val broker: KafkaTemplate<String, String>,
    private val db: QueryClient
) {
    @Scheduled(fixedDelay = 250)
    fun poll() {
        val rows = db.query("SELECT id, payload FROM deliveries WHERE sent_at IS NULL")
        for (row in rows) {
            tx.executeWithoutResult {
                broker.send("delivery.pending", row.payload).get(5, java.util.concurrent.TimeUnit.SECONDS)
                db.execute("UPDATE deliveries SET sent_at = CURRENT_TIMESTAMP WHERE id = :id", mapOf("id" to row.id))
            }
        }
    }
}

data class PendingDelivery(val id: String, val payload: String)

interface QueryClient {
    fun query(sql: String): List<PendingDelivery>
    fun execute(sql: String, parameters: Map<String, String>): Int
}
