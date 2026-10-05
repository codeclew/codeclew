using Microsoft.AspNetCore.Mvc;
using Orders.Core;

namespace Orders.Api.Controllers;

public sealed record CreateOrderRequest(string Customer, decimal Total);

[ApiController]
[Route("api/[controller]")]
public sealed class OrdersController : ControllerBase
{
    private readonly IOrderService _orders;

    public OrdersController(IOrderService orders)
    {
        _orders = orders;
    }

    [HttpGet("{id:int}")]
    public async Task<ActionResult<Order>> Get(int id)
    {
        var order = await _orders.GetAsync(id);
        if (order is null)
        {
            return NotFound();
        }
        return order;
    }

    [HttpPost]
    public ActionResult<Order> Create([FromBody] CreateOrderRequest request)
    {
        var order = _orders.Create(request.Customer, request.Total);
        return CreatedAtAction(nameof(Get), new { id = order.Id }, order);
    }

    [HttpPut("{id}")]
    [Consumes("application/json")]
    public IActionResult Replace(int id, [FromBody] CreateOrderRequest request)
    {
        _orders.Create(request.Customer);
        return NoContent();
    }

    [HttpGet]
    [Route("~/health")]
    public IActionResult Health() => Ok();

    [NonAction]
    public string Describe() => nameof(OrdersController);
}
