package example.contour;
public class Validator {
    public String normalize(String value) {
        if (value == null || value.isBlank()) { throw new IllegalArgumentException("empty"); }
        return value.strip();
    }
}
