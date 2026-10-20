package example.contour;
import org.springframework.web.bind.annotation.RestController;
import org.springframework.web.bind.annotation.RequestMapping;
@RestController
@RequestMapping("/composed")
public class ComposedController {
    @ShortcutGet
    public String shortcut() { return "shortcut"; }
}
