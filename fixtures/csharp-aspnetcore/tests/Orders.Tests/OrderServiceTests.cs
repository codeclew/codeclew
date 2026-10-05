using Orders.Core;
using Xunit;

namespace Orders.Tests;

public sealed class OrderServiceTests
{
    [Fact]
    public void CreateSavesTheOrder()
    {
        var service = new OrderService(new InMemoryOrderRepository());
        Assert.Equal("ada", service.Create("ada").Customer);
    }
}
