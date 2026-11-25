// Bind retained hosting registration and the corresponding framework callback.
using System.Text;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;

internal static class HostedWorkers
{
    private const string RegistrationMethod =
        "M:Microsoft.Extensions.DependencyInjection.ServiceCollectionHostedServiceExtensions.AddHostedService``1(Microsoft.Extensions.DependencyInjection.IServiceCollection)";
    private const string CallbackMethod =
        "M:Microsoft.Extensions.Hosting.BackgroundService.ExecuteAsync(System.Threading.CancellationToken)";

    private static bool ConfiguredSource(SyntaxTree tree) =>
        tree.GetRoot().DescendantTrivia(descendIntoTrivia: true).Any(trivia => trivia.Kind() is
            SyntaxKind.IfDirectiveTrivia or SyntaxKind.ElifDirectiveTrivia or SyntaxKind.DefineDirectiveTrivia or
            SyntaxKind.UndefDirectiveTrivia);

    internal static List<object> Discover(CSharpCompilation compilation)
    {
        var found = new List<object>();
        if (compilation.SyntaxTrees.Any(ConfiguredSource))
            return found;
        var withSdk = compilation.AddSyntaxTrees(CSharpSyntaxTree.ParseText("""
            global using System;
            global using System.Threading;
            global using System.Threading.Tasks;
            global using Microsoft.AspNetCore.Builder;
            global using Microsoft.Extensions.DependencyInjection;
            global using Microsoft.Extensions.Hosting;
            """));

        foreach (var tree in compilation.SyntaxTrees)
        {
            var model = compilation.GetSemanticModel(tree);
            var sdkModel = withSdk.GetSemanticModel(tree);
            foreach (var invocation in tree.GetRoot().DescendantNodes().OfType<InvocationExpressionSyntax>())
            {
                if (found.Count == 16) return found;
                if (invocation.ArgumentList.Arguments.Count != 0 ||
                    invocation.Expression is not MemberAccessExpressionSyntax
                    {
                        Name: GenericNameSyntax { Identifier.ValueText: "AddHostedService" }
                    })
                    continue;
                var current = model;
                var symbol = model.GetSymbolInfo(invocation).Symbol as IMethodSymbol;
                if (symbol is null)
                {
                    current = sdkModel;
                    symbol = sdkModel.GetSymbolInfo(invocation).Symbol as IMethodSymbol;
                }
                if (symbol is null || symbol.TypeArguments.Length != 1 ||
                    symbol.ContainingAssembly?.Name != "Microsoft.Extensions.Hosting.Abstractions" ||
                    (symbol.ReducedFrom ?? symbol).OriginalDefinition.GetDocumentationCommentId() != RegistrationMethod ||
                    symbol.TypeArguments[0] is not INamedTypeSymbol worker || worker.IsGenericType ||
                    !worker.Locations.Any(location => location.IsInSource) ||
                    current.GetDiagnostics(invocation.Span).Any(diagnostic => diagnostic.Severity == DiagnosticSeverity.Error))
                    continue;
                var callbacks = worker.GetMembers("ExecuteAsync").OfType<IMethodSymbol>().Where(OverridesCallback)
                    .SelectMany(method => method.DeclaringSyntaxReferences.Select(reference => reference.GetSyntax()))
                    .OfType<MethodDeclarationSyntax>().Where(method => method.Body is not null || method.ExpressionBody is not null)
                    .ToArray();
                if (callbacks.Length != 1)
                    continue;
                var callback = callbacks[0];
                found.Add(new
                {
                    kind = "HOSTED_SERVICE",
                    name = worker.ToDisplayString() + ".ExecuteAsync",
                    registration = Range(invocation),
                    callback = Range(callback),
                    helpers = Helpers(current.Compilation, callback),
                    sdkImports = current == sdkModel
                });
            }
        }
        return found;
    }

    private static bool OverridesCallback(IMethodSymbol method)
    {
        for (var parent = method.OverriddenMethod; parent is not null; parent = parent.OverriddenMethod)
        {
            if (parent.ContainingAssembly?.Name == "Microsoft.Extensions.Hosting.Abstractions" &&
                parent.GetDocumentationCommentId() == CallbackMethod)
                return true;
        }
        return false;
    }

    private static IEnumerable<object> Helpers(Compilation compilation, MethodDeclarationSyntax callback)
    {
        var pending = new Queue<MethodDeclarationSyntax>();
        var visited = new HashSet<SyntaxNode> { callback };
        pending.Enqueue(callback);
        while (pending.TryDequeue(out var method))
        {
            var model = compilation.GetSemanticModel(method.SyntaxTree);
            foreach (var call in method.DescendantNodes(node => node is not
                (AnonymousFunctionExpressionSyntax or LocalFunctionStatementSyntax)).OfType<InvocationExpressionSyntax>())
            {
                if (visited.Count >= 8) yield break;
                if (model.GetSymbolInfo(call).Symbol is not IMethodSymbol target ||
                    target.IsAbstract || (target.IsVirtual && !target.IsSealed && !target.ContainingType.IsSealed) ||
                    target.DeclaringSyntaxReferences.Length != 1 ||
                    target.DeclaringSyntaxReferences[0].GetSyntax() is not MethodDeclarationSyntax candidate ||
                    (candidate.Body is null && candidate.ExpressionBody is null) || !visited.Add(candidate))
                    continue;
                yield return Range(candidate);
                pending.Enqueue(candidate);
            }
        }
    }

    private static object Range(SyntaxNode node)
    {
        var tree = node.SyntaxTree;
        var text = tree.GetText().ToString();
        return new
        {
            file = tree.FilePath,
            start = Encoding.UTF8.GetByteCount(text.AsSpan(0, node.SpanStart)),
            end = Encoding.UTF8.GetByteCount(text.AsSpan(0, node.Span.End))
        };
    }
}
