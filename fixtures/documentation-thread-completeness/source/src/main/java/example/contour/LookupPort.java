package example.contour;
public interface LookupPort { String lookup(String value); }
class MemoryLookup implements LookupPort {
    public String lookup(String value) { return "memory:" + value; }
}
class RemoteLookup implements LookupPort {
    public String lookup(String value) { return "remote:" + value; }
}
