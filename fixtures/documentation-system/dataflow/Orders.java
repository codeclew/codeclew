public class Orders {
    private Jdbc jdbc;
    public int reserve(int quantity) { return normalize(quantity); }
    private int normalize(int quantity) { return quantity; }
    public QuantityDto dto(int quantity) { return new QuantityDto(normalize(quantity)); }
    public QuantityMessage message(int quantity) { return new QuantityMessage(quantity); }
    public void write(QuantityDto dto) {
        jdbc.update("insert into quantity_records(quantity) values (?)", dto.quantity);
    }
}
class QuantityDto {
    final int quantity;
    QuantityDto(int quantity) { this.quantity = quantity; }
}
class QuantityMessage {
    final int quantity;
    QuantityMessage(int quantity) { this.quantity = quantity; }
}
interface Jdbc { int update(String statement, int quantity); }
