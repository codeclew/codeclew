package example;

public final class JavaNormalizer {
    public String normalize(String input) {
        return input.trim();
    }

    public int overloaded(int value) {
        return value * 2;
    }

    public String overloaded(String value) {
        return value.trim();
    }
}
