package example
import org.springframework.web.bind.annotation.RestController
import org.springframework.web.bind.annotation.RequestMapping
import org.springframework.web.bind.annotation.PostMapping as Reserve
@RestController
@RequestMapping("/orders")
class Orders {
    @Reserve(path = ["/reserve"])
    fun reserve(quantity: Int): Int { return quantity }
}
