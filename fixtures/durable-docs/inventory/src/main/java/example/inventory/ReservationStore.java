package example.inventory;
public class ReservationStore {
    private ReservationRequest lastReservation;
    public void save(ReservationRequest request) { lastReservation = request; }
}
