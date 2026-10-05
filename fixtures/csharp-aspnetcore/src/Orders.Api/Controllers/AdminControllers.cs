using Asp.Versioning;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Mvc;

namespace Orders.Api.Controllers;

[Area("admin")]
[Route("[area]/[controller]/[action]")]
[Authorize(Policy = "Operators")]
public sealed class ReportsController : Controller
{
    [HttpGet]
    public Task<IActionResult> DailyAsync() => Task.FromResult<IActionResult>(Ok());
}

[ApiController]
[Route("internal/audit")]
internal sealed class AuditController : ControllerBase
{
    [HttpDelete("{id}")]
    [AllowAnonymous]
    public IActionResult Purge(int id) => NoContent();
}

[ApiController]
[ApiVersion("2.0")]
[Route("api/v{version:apiVersion}/quotes")]
public sealed class QuotesController : ControllerBase
{
    [HttpGet]
    public IActionResult List() => Ok();
}
