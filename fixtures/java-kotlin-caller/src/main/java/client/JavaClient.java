package client;

import calls.KotlinFormatter;

public final class JavaClient {
    public String format(KotlinFormatter formatter, String input) {
        return formatter.format(input);
    }
}
