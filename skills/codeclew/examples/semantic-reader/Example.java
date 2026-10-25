package example;

/** Independently constructed source for semantic-reader acceptance. */
public final class Example {
    public record Vehicle(String reference, int manufactureYear, String make, Integer mileageKm) {}
    public record QuoteInput(Vehicle vehicle, Integer coverageMonths) {}
    public record VehicleWire(int year, String manufacturer, Integer odometerKm) {}
    /** A historical object with copied values, not a second representation of the live vehicle. */
    public record HistoryRecord(String vehicleReference, int manufactureYearSnapshot, Integer mileageKmSnapshot) {}
    /** The meaning of year is deliberately not defined by this legacy interface. */
    public record LegacyInput(int year) {}
    public record ContextData(String vehicleReference, int manufactureYear) {}
    public record Receipt(String token) {}
    public record ErrorResponse(int status, String code, String meaning) {}
    public record Reply(int status, Integer quotedAmount, ErrorResponse error) {}
    public interface ContextSink { Receipt send(ContextData context); }
    public static final class SendFailure extends RuntimeException {}
    public static final class MinimumCoverageException extends RuntimeException {}

    public static VehicleWire toWire(Vehicle vehicle) {
        // These names carry manufacture year, manufacturer and odometer kilometres in this mapping.
        return new VehicleWire(vehicle.manufactureYear(), vehicle.make(), vehicle.mileageKm());
    }
    public static HistoryRecord remember(Vehicle vehicle) {
        return new HistoryRecord(vehicle.reference(), vehicle.manufactureYear(), vehicle.mileageKm());
    }
    public static int ageAt(Vehicle vehicle, int valuationYear) {
        return valuationYear - vehicle.manufactureYear();
    }
    public static int legacyNextYear(LegacyInput input) { return input.year() + 1; }
    public static int quote(QuoteInput request, boolean minimumCoverageEnabled, ContextSink sink) {
        if (minimumCoverageEnabled && request.coverageMonths() != null && request.coverageMonths() < 6) {
            throw new MinimumCoverageException();
        }
        ContextData context = mapContext(request);
        try {
            sink.send(context); // The returned receipt is not used; this is a send attempt.
        } catch (SendFailure ignored) {
            // Only send failure is tolerated. Mapping above is outside this catch.
        }
        return request.vehicle().manufactureYear();
    }
    static ContextData mapContext(QuoteInput request) {
        if (request.vehicle() == null) throw new IllegalArgumentException("Vehicle required for context");
        return new ContextData(request.vehicle().reference(), request.vehicle().manufactureYear());
    }
    public static Reply endpoint(QuoteInput request, boolean enabled, ContextSink sink) {
        try { return new Reply(200, quote(request, enabled, sink), null); }
        catch (MinimumCoverageException failure) { return rejected(failure); }
    }
    // This handler is outside the initial quote-method fragment; resolve it before inventing meaning/status.
    static Reply rejected(MinimumCoverageException failure) {
        return new Reply(409, null, new ErrorResponse(409, "WARRANTY_DURATION_TOO_SHORT",
            "Warranty coverage must last at least six months when this check is enabled."));
    }
    public static void main(String[] args) {
        Vehicle vehicle = new Vehicle("vehicle-local", 2020, "Example make", 42000);
        ContextSink sink = context -> new Receipt("local-receipt");
        if (endpoint(new QuoteInput(vehicle, 5), true, sink).status() != 409) throw new AssertionError();
        if (endpoint(new QuoteInput(vehicle, 6), true, sink).status() != 200) throw new AssertionError();
        if (endpoint(new QuoteInput(vehicle, null), true, sink).status() != 200) throw new AssertionError();
        if (endpoint(new QuoteInput(vehicle, 5), false, sink).status() != 200) throw new AssertionError();
        if (quote(new QuoteInput(vehicle, 6), true, context -> { throw new SendFailure(); }) != 2020) throw new AssertionError();
        boolean mappingEscaped = false;
        try { quote(new QuoteInput(null, 6), false, sink); }
        catch (IllegalArgumentException expected) { mappingEscaped = true; }
        if (!mappingEscaped) throw new AssertionError("Mapping is outside the send catch");
        if (ageAt(vehicle, 2026) != 6 || toWire(vehicle).year() != vehicle.manufactureYear()) throw new AssertionError();
        if (remember(vehicle).manufactureYearSnapshot() != vehicle.manufactureYear()) throw new AssertionError();
    }
}
