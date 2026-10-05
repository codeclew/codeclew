namespace Orders.Core;

public sealed record Order(int Id, string Customer, decimal Total);

public interface IOrderRepository
{
    Order? Find(int id);
    void Save(Order order);
}

public interface IOrderService
{
    Task<Order?> GetAsync(int id);
    Order Create(string customer, decimal total);
    Order Create(string customer);
}

public abstract class AuditedService
{
    protected virtual string Describe(Order order) => $"order {order.Id}";
}

public sealed partial class OrderService : AuditedService, IOrderService
{
    private readonly IOrderRepository _repository;

    public OrderService(IOrderRepository repository)
    {
        _repository = repository;
    }

    public Task<Order?> GetAsync(int id)
    {
        var order = _repository.Find(id);
        if (order is null)
        {
            return Task.FromResult<Order?>(null);
        }
        return Task.FromResult<Order?>(order);
    }

    public Order Create(string customer, decimal total)
    {
        var order = new Order(NextId(), customer, total);
        _repository.Save(order);
        return order;
    }

    public Order Create(string customer) => Create(customer, 0m);

    protected override string Describe(Order order) => base.Describe(order).ToUpperInvariant();
}

public sealed partial class OrderService
{
    private static int _next;

    private static int NextId() => Interlocked.Increment(ref _next);

    public IEnumerable<int> Ids(IEnumerable<Order> orders) => orders.Select(order => order.Id);
}

public sealed class InMemoryOrderRepository : IOrderRepository
{
    private readonly Dictionary<int, Order> _orders = new();

    public Order? Find(int id) => _orders.GetValueOrDefault(id);

    public void Save(Order order) => _orders[order.Id] = order;
}
