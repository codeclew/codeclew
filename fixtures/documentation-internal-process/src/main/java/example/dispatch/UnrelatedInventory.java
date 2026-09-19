package example.dispatch;

public final class UnrelatedInventory {
    public int available(int stock, int reserved) { return stock - reserved; }
    public String health() { return "ready"; }
    public String formatCode(String prefix, long number) { return prefix + number; }
}
