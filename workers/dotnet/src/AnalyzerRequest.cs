using System.Text.Json.Nodes;

namespace Codeclew.CSharp.Analyzer;

public sealed record AnalyzerRequest(
    string Repository,
    CompilationSelector Selector,
    string Output,
    string Scratch,
    int MaxFacts)
{
    public static AnalyzerRequest Read(string text)
    {
        var root = JsonNode.Parse(text) as JsonObject
            ?? throw new UnsupportedException("CSHARP_REQUEST_INVALID", "request must be a JSON object");
        if ((string?)root["schema"] != Program.RequestSchema)
        {
            throw new UnsupportedException("CSHARP_REQUEST_INVALID", "request schema is unsupported");
        }
        var repository = AbsoluteDirectory(root, "repository");
        var scratch = AbsoluteDirectory(root, "scratch");
        var output = (string?)root["output"];
        if (string.IsNullOrEmpty(output) || !Path.IsPathRooted(output) || File.Exists(output))
        {
            throw new UnsupportedException("CSHARP_REQUEST_INVALID", "output must be a new absolute file path");
        }
        var maxFacts = (int?)root["maxFacts"] ?? 1_048_576;
        if (maxFacts is < 1 or > 4_194_304)
        {
            throw new UnsupportedException("CSHARP_REQUEST_INVALID", "maxFacts is out of range");
        }
        var selector = CompilationSelector.Parse((string?)root["compilation"] ?? "");
        return new AnalyzerRequest(repository, selector, output, scratch, maxFacts);
    }

    private static string AbsoluteDirectory(JsonObject root, string key)
    {
        var value = (string?)root[key];
        if (string.IsNullOrEmpty(value) || !Path.IsPathRooted(value) || !Directory.Exists(value))
        {
            throw new UnsupportedException("CSHARP_REQUEST_INVALID", $"{key} must be an existing absolute directory");
        }
        return Path.GetFullPath(value).TrimEnd(Path.DirectorySeparatorChar);
    }
}

/// <summary><c>csproj:&lt;path&gt;[@tfm]</c> or <c>sln:&lt;path&gt;</c>, repository-relative.</summary>
public sealed record CompilationSelector(string Kind, string Path, string? TargetFramework)
{
    public string Canonical => Kind == "csproj" && TargetFramework is not null
        ? $"csproj:{Path}@{TargetFramework}"
        : $"{Kind}:{Path}";

    public static CompilationSelector Parse(string value)
    {
        string kind;
        string rest;
        if (value.StartsWith("csproj:", StringComparison.Ordinal))
        {
            kind = "csproj";
            rest = value["csproj:".Length..];
        }
        else if (value.StartsWith("sln:", StringComparison.Ordinal))
        {
            kind = "sln";
            rest = value["sln:".Length..];
        }
        else
        {
            throw new UnsupportedException("CSHARP_SELECTOR_INVALID", "compilation selector must start with csproj: or sln:");
        }
        string? tfm = null;
        var at = rest.LastIndexOf('@');
        if (kind == "csproj" && at > 0)
        {
            tfm = rest[(at + 1)..];
            rest = rest[..at];
            if (tfm.Length == 0 || tfm.Length > 64 || !tfm.All(c => char.IsAsciiLetterOrDigit(c) || c is '.' or '-'))
            {
                throw new UnsupportedException("CSHARP_SELECTOR_INVALID", "target framework is invalid");
            }
        }
        var extensionValid = kind == "csproj"
            ? rest.EndsWith(".csproj", StringComparison.Ordinal)
            : rest.EndsWith(".sln", StringComparison.Ordinal) || rest.EndsWith(".slnx", StringComparison.Ordinal);
        if (rest.Length == 0 || rest.Length > 512 || !extensionValid || System.IO.Path.IsPathRooted(rest)
            || rest.Contains('\\') || rest.Split('/').Any(part => part is "" or "." or ".."))
        {
            throw new UnsupportedException("CSHARP_SELECTOR_INVALID", "compilation selector path is invalid");
        }
        return new CompilationSelector(kind, rest, tfm);
    }
}
