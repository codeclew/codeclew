package example.contour;
import org.springframework.web.bind.annotation.RestController;
import org.springframework.web.bind.annotation.RequestMapping;
@RestController
@RequestMapping("/inherited")
public class InheritedController extends BaseRoutes {}
