using System.Security.Cryptography;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using System.Xml.Linq;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

namespace Codeclew.CSharp.Analyzer;

public sealed record AnalysisSummary(int FactCount);

public static class Analyzer
{
    private const int MaxProjects = 512;
    private const int MaxSourceFiles = 65_536;
    private const long MaxSourceFileBytes = 8L * 1024 * 1024;

    public static Task<AnalysisSummary> RunAsync(AnalyzerRequest request)
    {
        var selected = Path.Combine(request.Repository, request.Selector.Path);
        if (!File.Exists(selected) || new FileInfo(selected).LinkTarget is not null)
        {
            throw new UnsupportedException("CSHARP_SELECTOR_UNAVAILABLE", "selected project or solution is not a regular file");
        }
        var msbuildScratch = Path.Combine(request.Scratch, "msbuild") + Path.DirectorySeparatorChar;
        Directory.CreateDirectory(msbuildScratch);
        var wrapper = Path.Combine(msbuildScratch, "Codeclew.Directory.Build.props");
        File.WriteAllText(wrapper, WrapperProps);
        var properties = new Dictionary<string, string>
        {
            ["CodeclewScratch"] = msbuildScratch,
            ["DirectoryBuildPropsPath"] = wrapper,
            ["Configuration"] = "Debug",
            ["DesignTimeBuild"] = "true",
            ["BuildingProject"] = "false",
            ["BuildProjectReferences"] = "false",
            ["SkipCompilerExecution"] = "true",
            ["ProvideCommandLineArgs"] = "true",
            ["ContinueOnError"] = "ErrorAndContinue",
            ["NuGetInteractive"] = "false",
            ["GenerateDocumentationFile"] = "false",
            ["RunAnalyzers"] = "false",
            ["TreatWarningsAsErrors"] = "false",
        };

        var boundaries = new SortedDictionary<string, int>(StringComparer.Ordinal);
        void Count(string code) => boundaries[code] = boundaries.GetValueOrDefault(code) + 1;

        var candidates = request.Selector.Kind == "csproj"
            ? [selected]
            : SolutionProjects(selected);
        var unrestored = new SortedSet<string>(StringComparer.Ordinal);
        var roots = new List<string>();
        foreach (var project in candidates)
        {
            if (!Inside(request.Repository, project))
            {
                Count("CSHARP_PROJECT_OUTSIDE_REPOSITORY");
                continue;
            }
            if (!File.Exists(project))
            {
                Count("CSHARP_SOLUTION_PROJECT_MISSING");
                continue;
            }
            if (!IsRestored(project))
            {
                unrestored.Add(Relative(request.Repository, project));
                continue;
            }
            roots.Add(project);
        }
        if (roots.Count == 0)
        {
            throw request.Selector.Kind == "csproj"
                ? new UnsupportedException("CSHARP_PROJECT_UNRESTORED",
                    "selected project has no restore output (obj/project.assets.json); run dotnet restore in the repository first")
                : new UnsupportedException("CSHARP_SOLUTION_HAS_NO_RESTORED_PROJECTS",
                    "no project in the selected solution has restore output; run dotnet restore in the repository first");
        }

        // Design-time builds in dependency order: every project after its references.
        var ordered = new List<LoadedProject>();
        var buildErrors = new SortedSet<string>(StringComparer.Ordinal);
        using (var loader = new ProjectLoader(properties))
        {
            var visited = new HashSet<string>(StringComparer.Ordinal);
            void Visit(string path, bool root)
            {
                if (!visited.Add(path))
                {
                    return;
                }
                if (!IsRestored(path))
                {
                    unrestored.Add(Inside(request.Repository, path) ? Relative(request.Repository, path) : Path.GetFileName(path));
                    return;
                }
                var frameworks = loader.TargetFrameworks(path);
                string? framework = null;
                if (root && request.Selector.TargetFramework is { } selectedFramework)
                {
                    framework = selectedFramework;
                }
                else if (frameworks.Count > 0)
                {
                    if (root && request.Selector.Kind == "csproj")
                    {
                        throw new UnsupportedException("CSHARP_MULTI_TARGET_SELECTION_REQUIRED",
                            "selected project targets several frameworks; select one with csproj:<path>@<tfm>");
                    }
                    framework = frameworks[0];
                    Count("CSHARP_MULTI_TARGET_FIRST_FRAMEWORK_SELECTED");
                }
                var loaded = loader.Load(path, framework);
                foreach (var error in loaded.Errors)
                {
                    buildErrors.Add(error);
                }
                foreach (var reference in loaded.ProjectReferences)
                {
                    Visit(reference, root: false);
                }
                if (loaded.CompilerArguments.Count == 0)
                {
                    Count("CSHARP_COMPILER_ARGUMENTS_UNAVAILABLE");
                    return;
                }
                ordered.Add(loaded);
                if (ordered.Count > MaxProjects)
                {
                    throw new UnsupportedException("CSHARP_PROJECT_LIMIT", $"C# analysis supports at most {MaxProjects} projects");
                }
            }
            foreach (var root in roots)
            {
                Visit(root, root: true);
            }
        }

        var byTargetPath = new Dictionary<string, BuiltCompilation>(StringComparer.Ordinal);
        var compilations = new List<BuiltCompilation>();
        foreach (var project in ordered)
        {
            var built = CompilationBuilder.Build(project, byTargetPath);
            // Referencing projects compile against the reference assembly when one is produced.
            foreach (var output in new[] { project.TargetPath, project.TargetRefPath })
            {
                if (!string.IsNullOrEmpty(output))
                {
                    byTargetPath[Path.GetFullPath(output)] = built;
                }
            }
            compilations.Add(built);
            foreach (var boundary in built.Boundaries)
            {
                Count(boundary);
            }
        }

        var header = new JsonObject
        {
            ["protocol"] = Program.Protocol,
            ["language"] = "csharp",
            ["authorityMode"] = "MSBUILD_DESIGN_TIME_COMMAND_LINE",
            ["compilation"] = request.Selector.Canonical,
            ["analyzerRoslynVersion"] = typeof(CSharpCompilation).Assembly
                .GetCustomAttributes(typeof(System.Reflection.AssemblyInformationalVersionAttribute), false)
                .OfType<System.Reflection.AssemblyInformationalVersionAttribute>()
                .FirstOrDefault()?.InformationalVersion.Split('+')[0] ?? "unknown",
            ["msbuildVersion"] = Program.MSBuildVersion,
            ["sdkVersion"] = Program.SdkVersion,
            ["runtimeVersion"] = Environment.Version.ToString(),
            ["projects"] = new JsonArray(compilations
                .OrderBy(built => Relative(request.Repository, built.Project.ProjectPath), StringComparer.Ordinal)
                .Select(built => (JsonNode?)ProjectRow(request.Repository, built)).ToArray()),
            ["unrestoredProjects"] = new JsonArray(unrestored.Select(path => (JsonNode?)JsonValue.Create(path)).ToArray()),
        };

        using var writer = new FactWriter(request.Output, request.MaxFacts);
        var sourceFiles = new SortedSet<string>(StringComparer.Ordinal);
        var plan = new List<(BuiltCompilation Built, SyntaxTree Tree, string Relative)>();
        foreach (var built in compilations.OrderBy(built => built.Project.ProjectPath, StringComparer.Ordinal))
        {
            if (built.GeneratedDocuments > 0)
            {
                writer.CountBoundary("CSHARP_SOURCE_GENERATED_DOCUMENTS_NOT_INDEXED", built.GeneratedDocuments);
            }
            foreach (var tree in built.Compilation.SyntaxTrees.OrderBy(tree => tree.FilePath, StringComparer.Ordinal))
            {
                var path = tree.FilePath;
                if (string.IsNullOrEmpty(path) || !Path.IsPathRooted(path))
                {
                    continue;
                }
                if (!Inside(request.Repository, path))
                {
                    // MSBuild-generated files (global usings, assembly info) live in the redirected scratch.
                    if (!path.StartsWith(request.Scratch, StringComparison.Ordinal))
                    {
                        writer.CountBoundary("CSHARP_SOURCE_OUTSIDE_REPOSITORY");
                    }
                    continue;
                }
                var relative = Relative(request.Repository, path);
                if (sourceFiles.Add(relative))
                {
                    plan.Add((built, tree, relative));
                }
            }
        }
        if (sourceFiles.Count > MaxSourceFiles)
        {
            throw new UnsupportedException("CSHARP_SOURCE_FILE_LIMIT", $"C# analysis supports at most {MaxSourceFiles} source files");
        }
        header["sourceFiles"] = new JsonArray(sourceFiles.Select(path => (JsonNode?)JsonValue.Create(path)).ToArray());
        header["boundaries"] = new JsonArray(buildErrors.Select(code => (JsonNode?)JsonValue.Create(code)).ToArray());
        writer.Header(header);

        foreach (var code in buildErrors)
        {
            writer.Boundary("CSHARP_DESIGN_TIME_BUILD_ERROR", diagnosticCode: code);
        }
        foreach (var _ in unrestored)
        {
            writer.CountBoundary("CSHARP_PROJECT_UNRESTORED");
        }
        foreach (var (code, count) in boundaries)
        {
            writer.CountBoundary(code, count);
        }

        var emittedTypes = new HashSet<string>(StringComparer.Ordinal);
        var generatedFiles = 0;
        foreach (var (built, tree, relative) in plan)
        {
            if (new FileInfo(tree.FilePath).Length > MaxSourceFileBytes)
            {
                writer.Boundary("CSHARP_SOURCE_FILE_BYTE_LIMIT");
                continue;
            }
            var anchors = new SourceAnchors(relative, tree.GetText(), SourceAnchors.BomBytes(tree.FilePath));
            var generated = IsGenerated(relative, tree);
            if (generated)
            {
                generatedFiles++;
            }
            var model = built.Compilation.GetSemanticModel(tree);
            var projectPath = Inside(request.Repository, built.Project.ProjectPath)
                ? Relative(request.Repository, built.Project.ProjectPath)
                : Path.GetFileName(built.Project.ProjectPath);
            new FactWalker(model, anchors, writer, projectPath, generated, emittedTypes).Visit(tree.GetRoot());
        }
        if (generatedFiles > 0)
        {
            writer.CountBoundary("GENERATED_SOURCE_BODIES_NOT_INDEXED", generatedFiles);
        }
        writer.Finish();
        return Task.FromResult(new AnalysisSummary(writer.FactCount));
    }

    private static JsonObject ProjectRow(string repository, BuiltCompilation built)
    {
        var references = new JsonArray();
        foreach (var reference in built.ReferencePaths.Distinct(StringComparer.Ordinal).Order(StringComparer.Ordinal))
        {
            references.Add(new JsonObject
            {
                ["logicalName"] = LogicalReference(repository, reference),
                ["digest"] = FileDigest(reference),
                ["size"] = new FileInfo(reference).Length,
            });
        }
        var project = built.Project;
        return new JsonObject
        {
            ["path"] = Inside(repository, project.ProjectPath) ? Relative(repository, project.ProjectPath) : Path.GetFileName(project.ProjectPath),
            ["assemblyName"] = project.AssemblyName,
            ["targetFramework"] = project.TargetFramework,
            ["langVersion"] = built.ParseOptions.LanguageVersion.ToDisplayString(),
            ["nullable"] = built.Compilation.Options.NullableContextOptions.ToString().ToUpperInvariant(),
            ["defineConstants"] = new JsonArray(built.ParseOptions.PreprocessorSymbolNames.Order(StringComparer.Ordinal)
                .Select(name => (JsonNode?)JsonValue.Create(name)).ToArray()),
            ["sourceFiles"] = built.SourceFiles.Count,
            ["generatedDocuments"] = built.GeneratedDocuments,
            ["metadataReferences"] = references,
        };
    }

    private static bool Inside(string repository, string path) =>
        Path.GetFullPath(path).StartsWith(repository + Path.DirectorySeparatorChar, StringComparison.Ordinal);

    private static readonly Dictionary<string, string> Digests = new(StringComparer.Ordinal);

    private static string FileDigest(string path)
    {
        if (!Digests.TryGetValue(path, out var digest))
        {
            using var stream = File.OpenRead(path);
            digest = "sha256:" + Convert.ToHexStringLower(SHA256.HashData(stream));
            Digests[path] = digest;
        }
        return digest;
    }

    /// <summary>Reference names without machine-local prefixes.</summary>
    private static string LogicalReference(string repository, string path)
    {
        var normalized = path.Replace('\\', '/');
        foreach (var (marker, prefix) in new[] { ("/.nuget/packages/", "nuget:"), ("/packs/", "pack:"), ("/shared/", "shared:") })
        {
            var index = normalized.IndexOf(marker, StringComparison.OrdinalIgnoreCase);
            if (index >= 0)
            {
                return prefix + normalized[(index + marker.Length)..];
            }
        }
        if (path.StartsWith(repository + Path.DirectorySeparatorChar, StringComparison.Ordinal))
        {
            return "repository:" + Relative(repository, path);
        }
        return "external:" + Path.GetFileName(path);
    }

    private static bool IsRestored(string project)
    {
        var directory = Path.GetDirectoryName(project)!;
        return File.Exists(Path.Combine(directory, "obj", "project.assets.json"));
    }

    private static IEnumerable<string> SolutionProjects(string solution)
    {
        var directory = Path.GetDirectoryName(solution)!;
        IEnumerable<string> paths;
        if (solution.EndsWith(".slnx", StringComparison.Ordinal))
        {
            paths = XDocument.Load(solution).Descendants("Project")
                .Select(element => (string?)element.Attribute("Path"))
                .Where(path => path is not null)!;
        }
        else
        {
            paths = File.ReadLines(solution)
                .Select(line => SolutionProjectPattern().Match(line))
                .Where(match => match.Success)
                .Select(match => match.Groups[1].Value);
        }
        return paths
            .Where(path => path.EndsWith(".csproj", StringComparison.OrdinalIgnoreCase))
            .Select(path => Path.GetFullPath(Path.Combine(directory, path.Replace('\\', Path.DirectorySeparatorChar))))
            .Distinct(StringComparer.Ordinal)
            .Order(StringComparer.Ordinal)
            .ToList();
    }

    private static bool IsGenerated(string relative, SyntaxTree tree)
    {
        var name = Path.GetFileName(relative);
        if (relative.Contains("/Connected Services/", StringComparison.Ordinal)
            || relative.StartsWith("Connected Services/", StringComparison.Ordinal)
            || name.EndsWith(".g.cs", StringComparison.OrdinalIgnoreCase)
            || name.EndsWith(".g.i.cs", StringComparison.OrdinalIgnoreCase)
            || name.EndsWith(".designer.cs", StringComparison.OrdinalIgnoreCase))
        {
            return true;
        }
        var leading = tree.GetRoot().GetLeadingTrivia().ToFullString();
        return leading.Contains("<auto-generated", StringComparison.OrdinalIgnoreCase)
            || leading.Contains("<autogenerated", StringComparison.OrdinalIgnoreCase);
    }

    public static string Relative(string repository, string path) =>
        Path.GetRelativePath(repository, path).Replace(Path.DirectorySeparatorChar, '/');

    // GeneratedRegex emits file-local types whose identities depend on the staging path.
    private static readonly Regex SolutionProjectRegex = new(@"^Project\(""\{[^}]+\}""\)\s*=\s*""[^""]*""\s*,\s*""([^""]+)""");
    private static Regex SolutionProjectPattern() => SolutionProjectRegex;

    /// <summary>
    /// Imports the repository's own Directory.Build.props, keeps NuGet restore output
    /// read from the project's obj directory, and redirects every MSBuild write
    /// (intermediate and output paths) into the private scratch directory.
    /// </summary>
    private const string WrapperProps = """
        <Project>
          <PropertyGroup>
            <_CodeclewRepositoryDirectoryBuildProps>$([MSBuild]::GetPathOfFileAbove('Directory.Build.props', '$(MSBuildProjectDirectory)'))</_CodeclewRepositoryDirectoryBuildProps>
          </PropertyGroup>
          <Import Project="$(_CodeclewRepositoryDirectoryBuildProps)" Condition="'$(_CodeclewRepositoryDirectoryBuildProps)' != '' and Exists('$(_CodeclewRepositoryDirectoryBuildProps)')" />
          <PropertyGroup>
            <_CodeclewOriginalIntermediate>$(BaseIntermediateOutputPath)</_CodeclewOriginalIntermediate>
            <_CodeclewOriginalIntermediate Condition="'$(_CodeclewOriginalIntermediate)' == ''">obj\</_CodeclewOriginalIntermediate>
            <MSBuildProjectExtensionsPath Condition="'$(MSBuildProjectExtensionsPath)' == ''">$([MSBuild]::EnsureTrailingSlash($([System.IO.Path]::Combine('$(MSBuildProjectDirectory)', '$(_CodeclewOriginalIntermediate)'))))</MSBuildProjectExtensionsPath>
            <_CodeclewProjectKey>$(MSBuildProjectName)-$([MSBuild]::StableStringHash('$(MSBuildProjectFullPath)'))</_CodeclewProjectKey>
            <BaseIntermediateOutputPath>$(CodeclewScratch)obj/$(_CodeclewProjectKey)/</BaseIntermediateOutputPath>
            <BaseOutputPath>$(CodeclewScratch)bin/$(_CodeclewProjectKey)/</BaseOutputPath>
            <UseArtifactsOutput>false</UseArtifactsOutput>
          </PropertyGroup>
        </Project>
        """;
}
