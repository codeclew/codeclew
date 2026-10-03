package example.linked;

/** A bounded driver execution whose static call graph contains a cycle. */
public final class CycleProbe {
    public int first(int remaining) {
        if (remaining <= 0) {
            return 0;
        }
        return 1 + second(remaining - 1);
    }

    public int second(int remaining) {
        if (remaining <= 0) {
            return 0;
        }
        return 1 + first(remaining - 1);
    }
}
