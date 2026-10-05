using System.Collections.Immutable;
using System.Reflection;
using System.Runtime.Loader;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.Diagnostics;
using Microsoft.CodeAnalysis.Text;

namespace Codeclew.CSharp.Analyzer;

public sealed record BuiltCompilation(
    LoadedProject Project,
    CSharpCompilation Compilation,
    CSharpParseOptions ParseOptions,
    IReadOnlyList<string> SourceFiles,
    IReadOnlyList<string> ReferencePaths,
    int GeneratedDocuments,
    IReadOnlyList<string> Boundaries);

/// <summary>
/// Builds a Roslyn compilation from the exact csc command line of a design-time
/// build. Project references become compilation references to already built
/// projects; source generators named on the command line run in-process.
/// </summary>
public static class CompilationBuilder
{
    public static BuiltCompilation Build(LoadedProject project, IReadOnlyDictionary<string, BuiltCompilation> byTargetPath)
    {
        var directory = Path.GetDirectoryName(project.ProjectPath)!;
        var arguments = project.CompilerArguments.Where(argument => !IsCompilerExecutable(argument)).ToList();
        var parsed = CSharpCommandLineParser.Default.Parse(arguments, directory, sdkDirectory: null);
        var boundaries = new List<string>();
        foreach (var error in parsed.Errors.Where(error => error.Severity == DiagnosticSeverity.Error))
        {
            boundaries.Add("CSHARP_COMMAND_LINE_ERROR:" + error.Id);
        }
        var trees = new List<SyntaxTree>();
        var sources = new List<string>();
        foreach (var source in parsed.SourceFiles)
        {
            var path = Path.GetFullPath(source.Path);
            if (!File.Exists(path))
            {
                boundaries.Add("CSHARP_SOURCE_FILE_MISSING");
                continue;
            }
            using var stream = File.OpenRead(path);
            var text = SourceText.From(stream, encoding: parsed.Encoding, checksumAlgorithm: SourceHashAlgorithm.Sha256);
            trees.Add(CSharpSyntaxTree.ParseText(text, parsed.ParseOptions, path));
            sources.Add(path);
        }
        var references = new List<MetadataReference>();
        var referencePaths = new List<string>();
        foreach (var reference in parsed.MetadataReferences)
        {
            var path = Path.GetFullPath(reference.Reference, directory);
            if (byTargetPath.TryGetValue(path, out var built))
            {
                references.Add(built.Compilation.ToMetadataReference(reference.Properties.Aliases, reference.Properties.EmbedInteropTypes));
                continue;
            }
            if (!File.Exists(path))
            {
                boundaries.Add("CSHARP_METADATA_REFERENCE_UNAVAILABLE");
                continue;
            }
            references.Add(MetadataReference.CreateFromFile(path, reference.Properties));
            referencePaths.Add(path);
        }
        var options = parsed.CompilationOptions
            .WithMetadataReferenceResolver(null)
            .WithSourceReferenceResolver(null)
            .WithXmlReferenceResolver(null)
            .WithStrongNameProvider(null)
            .WithAssemblyIdentityComparer(DesktopAssemblyIdentityComparer.Default);
        var compilation = CSharpCompilation.Create(parsed.CompilationName ?? project.AssemblyName, trees, references, options);
        var generated = 0;
        if (parsed.AnalyzerReferences.Length > 0)
        {
            (compilation, generated) = RunGenerators(compilation, parsed, directory, boundaries);
        }
        return new BuiltCompilation(project, compilation, parsed.ParseOptions, sources, referencePaths, generated, boundaries);
    }

    private static bool IsCompilerExecutable(string argument) =>
        !argument.StartsWith('/') && !argument.StartsWith('-')
        && (argument.EndsWith("csc.dll", StringComparison.OrdinalIgnoreCase)
            || argument.EndsWith("csc.exe", StringComparison.OrdinalIgnoreCase));

    private static (CSharpCompilation, int) RunGenerators(CSharpCompilation compilation, CSharpCommandLineArguments parsed, string directory, List<string> boundaries)
    {
        var loader = new IsolatedAnalyzerLoader();
        var generators = new List<ISourceGenerator>();
        foreach (var reference in parsed.AnalyzerReferences)
        {
            var path = Path.GetFullPath(reference.FilePath, directory);
            if (!File.Exists(path))
            {
                boundaries.Add("CSHARP_ANALYZER_REFERENCE_UNAVAILABLE");
                continue;
            }
            loader.AddDependencyLocation(path);
            try
            {
                generators.AddRange(new AnalyzerFileReference(path, loader).GetGenerators(LanguageNames.CSharp));
            }
            catch (Exception)
            {
                boundaries.Add("CSHARP_SOURCE_GENERATOR_LOAD_FAILED");
            }
        }
        if (generators.Count == 0)
        {
            return (compilation, 0);
        }
        try
        {
            var configs = parsed.AnalyzerConfigPaths
                .Where(File.Exists)
                .Select(path => AnalyzerConfig.Parse(SourceText.From(File.ReadAllText(path)), path))
                .ToImmutableArray();
            var set = AnalyzerConfigSet.Create(configs);
            var additional = parsed.AdditionalFiles
                .Where(file => File.Exists(file.Path))
                .Select(file => (AdditionalText)new FileAdditionalText(file.Path))
                .ToImmutableArray();
            var driver = CSharpGeneratorDriver.Create(
                generators.ToImmutableArray(),
                additional,
                (CSharpParseOptions)compilation.SyntaxTrees.FirstOrDefault()?.Options!,
                new ConfigOptionsProvider(set));
            driver.RunGeneratorsAndUpdateCompilation(compilation, out var updated, out var diagnostics);
            if (diagnostics.Any(diagnostic => diagnostic.Severity == DiagnosticSeverity.Error))
            {
                boundaries.Add("CSHARP_SOURCE_GENERATOR_ERRORS");
            }
            return ((CSharpCompilation)updated, updated.SyntaxTrees.Count() - compilation.SyntaxTrees.Count());
        }
        catch (Exception)
        {
            boundaries.Add("CSHARP_SOURCE_GENERATOR_FAILED");
            return (compilation, 0);
        }
    }

    private sealed class FileAdditionalText(string path) : AdditionalText
    {
        public override string Path { get; } = path;
        public override SourceText GetText(CancellationToken cancellationToken = default) => SourceText.From(File.ReadAllText(Path));
    }

    private sealed class ConfigOptionsProvider(AnalyzerConfigSet set) : AnalyzerConfigOptionsProvider
    {
        private readonly ConfigOptions _global = new(set.GlobalConfigOptions.AnalyzerOptions);
        public override AnalyzerConfigOptions GlobalOptions => _global;
        public override AnalyzerConfigOptions GetOptions(SyntaxTree tree) => new ConfigOptions(set.GetOptionsForSourcePath(tree.FilePath).AnalyzerOptions, _global);
        public override AnalyzerConfigOptions GetOptions(AdditionalText textFile) => new ConfigOptions(set.GetOptionsForSourcePath(textFile.Path).AnalyzerOptions, _global);
    }

    private sealed class ConfigOptions(ImmutableDictionary<string, string> values, ConfigOptions? fallback = null) : AnalyzerConfigOptions
    {
        public override bool TryGetValue(string key, out string value)
        {
            if (values.TryGetValue(key, out var found))
            {
                value = found;
                return true;
            }
            if (fallback is not null)
            {
                return fallback.TryGetValue(key, out value);
            }
            value = "";
            return false;
        }

        public override IEnumerable<string> Keys => fallback is null ? values.Keys : values.Keys.Union(fallback.Keys);
    }

    /// <summary>Loads analyzer assemblies in a private context that shares Roslyn with the host.</summary>
    private sealed class IsolatedAnalyzerLoader : IAnalyzerAssemblyLoader
    {
        private readonly Context _context = new();

        public void AddDependencyLocation(string fullPath) => _context.Directories.Add(Path.GetDirectoryName(fullPath)!);

        public Assembly LoadFromPath(string fullPath) => _context.LoadFromAssemblyPath(fullPath);

        private sealed class Context() : AssemblyLoadContext("codeclew-analyzers", isCollectible: false)
        {
            public HashSet<string> Directories { get; } = new(StringComparer.Ordinal);

            protected override Assembly? Load(AssemblyName name)
            {
                if (name.Name is { } simple && (simple.StartsWith("Microsoft.CodeAnalysis", StringComparison.Ordinal)
                        || simple.StartsWith("System.", StringComparison.Ordinal) || simple == "netstandard"))
                {
                    try
                    {
                        return Default.LoadFromAssemblyName(name);
                    }
                    catch (FileNotFoundException)
                    {
                    }
                }
                foreach (var directory in Directories)
                {
                    var candidate = Path.Combine(directory, name.Name + ".dll");
                    if (File.Exists(candidate))
                    {
                        return LoadFromAssemblyPath(candidate);
                    }
                }
                return null;
            }
        }
    }
}
