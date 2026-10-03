package example.linked;

public final class Task {
    public final String reference;
    public final String name;
    public final boolean eligible;
    public final int priority;

    public Task(String reference, String name, boolean eligible, int priority) {
        this.reference = reference;
        this.name = name;
        this.eligible = eligible;
        this.priority = priority;
    }
}
