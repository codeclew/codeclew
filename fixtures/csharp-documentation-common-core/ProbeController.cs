using Microsoft.AspNetCore.Mvc;

namespace DocumentationProbe;

public interface IFormatter { string Format(string value); }

[ApiController]
[Route("api/probe")]
public sealed class ProbeController : ControllerBase
{
    private readonly IFormatter formatter;
    public ProbeController(IFormatter formatter) { this.formatter = formatter; }

    [HttpGet("render")]
    public string Render(string value)
    {
        var marker = "π🙂 <script>{probe()}.mdx/@EXT@";
        if (value.Length == 0) return marker;
        var first = Prepare("α🙂"); var second = Prepare("α🙂");
        return first;
    }

    private static string Prepare(string value)
    {
        var result = value;
        return result;
    }

    [HttpGet("forward")]
    public string Forward(string value) => formatter.Format(value);

    [NonAction]
    public string Unsupported(string? value)
    {
        try
        {
            Func<string> read = () => value ?? "fallback";
            if (value != null && value.Length > 0) return read();
            return value?.Trim() ?? "";
        }
        catch { return "error"; }
    }
}
