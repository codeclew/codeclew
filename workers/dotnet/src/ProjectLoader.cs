using Microsoft.Build.Evaluation;
using Microsoft.Build.Execution;
using Microsoft.Build.Framework;

namespace Codeclew.CSharp.Analyzer;

/// <summary>Exact compiler inputs of one project, captured from a design-time build.</summary>
public sealed record LoadedProject(
    string ProjectPath,
    string? TargetFramework,
    string TargetPath,
    string TargetRefPath,
    string AssemblyName,
    IReadOnlyList<string> CompilerArguments,
    IReadOnlyList<string> ProjectReferences,
    IReadOnlyList<string> Errors);

/// <summary>
/// Runs the SDK's design-time <c>Compile</c> target in-process with compiler execution
/// skipped and command-line arguments provided. No out-of-process MSBuild nodes or
/// build hosts are started, restore is never run, and every MSBuild write is
/// redirected into the private scratch directory by the wrapper props.
/// </summary>
public sealed class ProjectLoader : IDisposable
{
    private readonly ProjectCollection _collection;
    private readonly Dictionary<string, string> _globalProperties;
    private readonly ErrorLogger _logger = new();

    public ProjectLoader(Dictionary<string, string> globalProperties)
    {
        _globalProperties = globalProperties;
        _collection = new ProjectCollection(globalProperties);
    }

    /// <summary>Target frameworks a project declares when it is multi-targeted, otherwise empty.</summary>
    public IReadOnlyList<string> TargetFrameworks(string projectPath)
    {
        var project = _collection.LoadProject(projectPath);
        var single = project.GetPropertyValue("TargetFramework");
        var multiple = project.GetPropertyValue("TargetFrameworks");
        _collection.UnloadProject(project);
        if (!string.IsNullOrWhiteSpace(single) || string.IsNullOrWhiteSpace(multiple))
        {
            return [];
        }
        return multiple.Split(';', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
    }

    public LoadedProject Load(string projectPath, string? targetFramework)
    {
        var properties = new Dictionary<string, string>(_globalProperties);
        if (targetFramework is not null)
        {
            properties["TargetFramework"] = targetFramework;
        }
        var project = _collection.LoadProject(projectPath, properties, toolsVersion: null);
        var instance = project.CreateProjectInstance();
        _logger.Errors.Clear();
        var parameters = new BuildParameters(_collection)
        {
            Loggers = [_logger],
            MaxNodeCount = 1,
            EnableNodeReuse = false,
            DisableInProcNode = false,
            ShutdownInProcNodeOnBuildFinish = false,
        };
        var request = new BuildRequestData(instance, ["Compile"], hostServices: null,
            BuildRequestDataFlags.ProvideProjectStateAfterBuild | BuildRequestDataFlags.IgnoreMissingEmptyAndInvalidImports);
        var result = BuildManager.DefaultBuildManager.Build(parameters, request);
        var errors = new List<string>(_logger.Errors);
        if (result.OverallResult != BuildResultCode.Success && errors.Count == 0)
        {
            errors.Add("MSBUILD_DESIGN_TIME_BUILD_FAILED");
        }
        var arguments = instance.GetItems("CscCommandLineArgs").Select(item => item.EvaluatedInclude).ToList();
        var references = instance.GetItems("ProjectReference")
            .Select(item => item.GetMetadataValue("FullPath"))
            .Where(path => path.EndsWith(".csproj", StringComparison.OrdinalIgnoreCase))
            .Distinct(StringComparer.Ordinal)
            .Order(StringComparer.Ordinal)
            .ToList();
        var loaded = new LoadedProject(
            projectPath,
            NullIfEmpty(instance.GetPropertyValue("TargetFramework")),
            instance.GetPropertyValue("TargetPath"),
            instance.GetPropertyValue("TargetRefPath"),
            instance.GetPropertyValue("AssemblyName"),
            arguments,
            references,
            errors);
        _collection.UnloadProject(project);
        return loaded;
    }

    private static string? NullIfEmpty(string value) => string.IsNullOrWhiteSpace(value) ? null : value;

    public void Dispose()
    {
        // No out-of-process nodes are started; ShutdownAllNodes would probe the MSBuild server mutex.
        _collection.UnloadAllProjects();
        _collection.Dispose();
    }

    /// <summary>Keeps only error codes; MSBuild messages can contain private paths.</summary>
    private sealed class ErrorLogger : ILogger
    {
        public List<string> Errors { get; } = [];
        public LoggerVerbosity Verbosity { get; set; } = LoggerVerbosity.Quiet;
        public string? Parameters { get; set; }

        public void Initialize(IEventSource eventSource)
        {
            eventSource.ErrorRaised += (_, error) =>
            {
                Errors.Add(string.IsNullOrEmpty(error.Code) ? "MSBUILD_ERROR" : error.Code);
                Console.Error.WriteLine($"msbuild: {error.Code}: {Bound(error.Message)}");
            };
        }

        public void Shutdown()
        {
        }

        private static string Bound(string? value) => value is null ? "" : value.Length <= 1024 ? value : value[..1024];
    }
}
