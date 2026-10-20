package example.contour;
import org.springframework.web.bind.annotation.GetMapping;
public class BaseRoutes {
    @GetMapping("/base")
    public String inherited() { return "base"; }
}
