package example;

import org.junit.jupiter.api.Test;
import org.springframework.test.web.servlet.setup.MockMvcBuilders;
import static org.springframework.test.web.servlet.request.MockMvcRequestBuilders.*;
import static org.springframework.test.web.servlet.result.MockMvcResultMatchers.*;

class RouteTest {
    @Test void literalMappingAndMethodAreObserved() throws Exception {
        var mvc = MockMvcBuilders.standaloneSetup(new Orders()).build();
        mvc.perform(post("/orders/reserve").param("quantity", "4"))
            .andExpect(status().isOk()).andExpect(content().string("4"));
        mvc.perform(get("/orders/reserve").param("quantity", "4"))
            .andExpect(status().isMethodNotAllowed());
    }
}
