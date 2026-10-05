using System.Runtime.CompilerServices;
using System.Text.Json;
using System.Text.Json.Nodes;
using Microsoft.Build.Locator;

namespace Codeclew.CSharp.Analyzer;

/// <summary>
/// One-shot Roslyn analyzer. Reads a request from stdin, writes NDJSON facts to the
/// requested output file, and prints a single status line on stdout. The target
/// repository is read-only: restore is never run and MSBuild intermediate output is
/// redirected into the private scratch directory.
/// </summary>
public static class Program
{
    public const string Protocol = "codeclew-csharp-analyzer.v1";
    public const string RequestSchema = "codeclew-csharp-analyzer-request/1.0";

    public static string? SdkVersion { get; private set; }
    public static string? MSBuildVersion { get; private set; }

    public static int Main()
    {
        try
        {
            var request = AnalyzerRequest.Read(Console.In.ReadToEnd());
            RegisterSdk(request.Repository);
            var summary = Run(request);
            WriteStatus(new JsonObject
            {
                ["protocol"] = Protocol,
                ["status"] = "OK",
                ["factCount"] = summary.FactCount,
            });
            return 0;
        }
        catch (UnsupportedException unsupported)
        {
            WriteStatus(new JsonObject
            {
                ["protocol"] = Protocol,
                ["status"] = "UNSUPPORTED",
                ["code"] = unsupported.Code,
                ["message"] = unsupported.Message,
            });
            return 2;
        }
        catch (Exception failure)
        {
            // Exception messages can contain private paths; report only the type.
            WriteStatus(new JsonObject
            {
                ["protocol"] = Protocol,
                ["status"] = "FAILED",
                ["code"] = "CSHARP_ANALYZER_INTERNAL_ERROR",
                ["message"] = failure.GetType().Name,
            });
            Console.Error.WriteLine(Bounded(failure.ToString()));
            return 3;
        }
    }

    /// <summary>Selects the SDK that the repository's global.json resolves to.</summary>
    private static void RegisterSdk(string repository)
    {
        var instance = MSBuildLocator.QueryVisualStudioInstances(new VisualStudioInstanceQueryOptions
            {
                DiscoveryTypes = DiscoveryType.DotNetSdk,
                WorkingDirectory = repository,
            })
            .FirstOrDefault()
            ?? throw new UnsupportedException("CSHARP_SDK_UNAVAILABLE", "no .NET SDK resolves for the repository");
        SdkVersion = instance.Version.ToString();
        MSBuildLocator.RegisterInstance(instance);
    }

    // MSBuild types must not be loaded before the locator registers the SDK.
    [MethodImpl(MethodImplOptions.NoInlining)]
    private static AnalysisSummary Run(AnalyzerRequest request)
    {
        MSBuildVersion = typeof(Microsoft.Build.Evaluation.Project).Assembly
            .GetCustomAttributes(typeof(System.Reflection.AssemblyInformationalVersionAttribute), false)
            .OfType<System.Reflection.AssemblyInformationalVersionAttribute>()
            .FirstOrDefault()?.InformationalVersion.Split('+')[0];
        return Analyzer.RunAsync(request).GetAwaiter().GetResult();
    }

    private static void WriteStatus(JsonObject status)
    {
        Console.Out.WriteLine(status.ToJsonString(new JsonSerializerOptions { WriteIndented = false }));
        Console.Out.Flush();
    }

    private static string Bounded(string value) => value.Length <= 16_384 ? value : value[..16_384];
}

public sealed class UnsupportedException(string code, string message) : Exception(message)
{
    public string Code { get; } = code;
}
