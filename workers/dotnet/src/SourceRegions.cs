using System.Text.Json.Nodes;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;

namespace Codeclew.CSharp.Analyzer;

/// <summary>Source containment, not a semantic CFG or an execution trace.</summary>
public sealed class SourceRegions
{
    public const string Schema = "codeclew-csharp-source-regions/1.0";
    private const int MaxSlots = 96;
    private const int MaxDepth = 24;
    private readonly SemanticModel _model;
    private readonly SourceAnchors _anchors;
    private readonly List<JsonObject> _nodes = [];
    private readonly List<JsonObject> _sites = [];
    private readonly List<JsonObject> _expressions = [];
    private readonly SortedSet<string> _boundaries = new(StringComparer.Ordinal);
    private bool _budget;

    private SourceRegions(SemanticModel model, SourceAnchors anchors)
    {
        _model = model;
        _anchors = anchors;
    }

    public static JsonObject Read(SemanticModel model, SourceAnchors anchors, IMethodSymbol method, SyntaxNode body)
    {
        var reader = new SourceRegions(model, anchors);
        reader.Node(body, -1, "BODY", 0);
        return new JsonObject
        {
            ["schema"] = Schema,
            ["authority"] = "ROSLYN_SOURCE_REGIONS",
            ["interpretation"] = "LEXICAL_AND_CONDITIONAL_NORMAL_ONLY",
            ["execution"] = "NOT_ASSERTED",
            ["ownerSymbolIdentity"] = Identities.Method(method),
            ["file"] = anchors.File,
            ["nodes"] = new JsonArray(reader._nodes.Select(n => (JsonNode?)n).ToArray()),
            ["sites"] = new JsonArray(reader._sites.Select(n => (JsonNode?)n).ToArray()),
            ["expressions"] = new JsonArray(reader._expressions.Select(n => (JsonNode?)n).ToArray()),
            ["boundaries"] = new JsonArray(reader._boundaries.Select(s => (JsonNode?)JsonValue.Create(s)).ToArray()),
            ["exceptionCompleteness"] = "UNKNOWN",
            ["interproceduralCompletion"] = "NOT_MODELED_USE_ORIGINAL_SOURCE",
        };
    }

    private int Slots => _nodes.Count + _sites.Count + _expressions.Count;
    private JsonObject Span(SyntaxNode syntax)
    {
        var span = new JsonObject();
        _anchors.Apply(span, syntax.Span);
        return span;
    }

    private void Opaque(JsonObject node, string reason)
    {
        if (node["opaque"]?.GetValue<string>() != "SOURCE_REGIONS_BUDGET") node["opaque"] = reason;
        _boundaries.Add(reason);
    }

    private int Node(SyntaxNode syntax, int parent, string role, int depth)
    {
        if (Slots >= MaxSlots && parent >= 0)
        {
            _budget = true;
            Opaque(_nodes[parent], "SOURCE_REGIONS_BUDGET");
            return -1;
        }
        var id = _nodes.Count;
        var node = new JsonObject
        {
            ["id"] = id, ["parent"] = parent < 0 ? null : parent,
            ["role"] = role, ["kind"] = "OPAQUE", ["source"] = Span(syntax),
        };
        _nodes.Add(node);
        if (_budget || Slots >= MaxSlots || depth > MaxDepth)
        {
            _budget = true;
            Opaque(node, "SOURCE_REGIONS_BUDGET");
            return id;
        }
        switch (syntax)
        {
            case BlockSyntax block:
                node["kind"] = "BLOCK";
                foreach (var statement in block.Statements)
                {
                    if (_budget || Slots >= MaxSlots - 1)
                    {
                        // One exact opaque suffix, not a hidden entire owner.
                        var gap = Node(statement, id, "STATEMENT", depth + 1);
                        if (gap < 0) break;
                        _anchors.Apply(_nodes[gap]["source"]!.AsObject(),
                            Microsoft.CodeAnalysis.Text.TextSpan.FromBounds(statement.SpanStart, block.Statements.Last().Span.End));
                        Opaque(_nodes[gap], "SOURCE_REGIONS_BUDGET");
                        _budget = true;
                        break;
                    }
                    Node(statement, id, "STATEMENT", depth + 1);
                }
                break;
            case IfStatementSyntax branch:
                node["kind"] = "IF";
                Expression(branch.Condition, id, "CONDITION");
                Node(branch.Statement, id, "THEN", depth + 1);
                if (branch.Else is { } otherwise) Node(otherwise.Statement, id, "ELSE", depth + 1);
                break;
            case WhileStatementSyntax loop:
                Loop(node, "WHILE", "CONDITION");
                Expression(loop.Condition, id, "CONDITION");
                Node(loop.Statement, id, "LOOP_BODY", depth + 1);
                break;
            case DoStatementSyntax loop:
                Loop(node, "DO", "CONDITION");
                Expression(loop.Condition, id, "CONDITION");
                Node(loop.Statement, id, "LOOP_BODY", depth + 1);
                break;
            case ForStatementSyntax loop:
                Loop(node, "FOR", "INCREMENTOR_THEN_CONDITION");
                if (loop.Declaration is { } declaration) Expression(declaration, id, "INITIALIZER");
                foreach (var expression in loop.Initializers) Expression(expression, id, "INITIALIZER");
                if (loop.Condition is { } condition) Expression(condition, id, "CONDITION");
                foreach (var expression in loop.Incrementors) Expression(expression, id, "INCREMENTOR");
                Node(loop.Statement, id, "LOOP_BODY", depth + 1);
                break;
            case ForEachStatementSyntax loop:
                Loop(node, "FOREACH", "ENUMERATOR_ADVANCE");
                Expression(loop.Expression, id, "ENUMERABLE");
                Node(loop.Statement, id, "LOOP_BODY", depth + 1);
                Opaque(node, "FOREACH_DISPOSAL_COMPLETION_NOT_MODELED");
                break;
            case TryStatementSyntax guarded:
                node["kind"] = "TRY";
                node["hasFinally"] = guarded.Finally is not null;
                Node(guarded.Block, id, "TRY_BODY", depth + 1);
                foreach (var handler in guarded.Catches)
                {
                    if (_budget) { Opaque(node, "SOURCE_REGIONS_BUDGET"); break; }
                    Node(handler, id, "CATCH", depth + 1);
                }
                if (guarded.Finally is { } cleanup)
                {
                    var region = Node(cleanup, id, "FINALLY", depth + 1);
                    if (region >= 0)
                    {
                        Opaque(_nodes[region], "FINALLY_COMPLETION_NOT_MODELED");
                        node["exitBoundary"] = region;
                    }
                }
                break;
            case CatchClauseSyntax handler:
                node["kind"] = "CATCH";
                if (handler.Declaration is { } caught) Expression(caught.Type, id, "CATCH_TYPE");
                if (handler.Filter is { } filter) Expression(filter.FilterExpression, id, "FILTER");
                Node(handler.Block, id, "CATCH_BODY", depth + 1);
                break;
            case UsingStatementSyntax resource:
                node["kind"] = "USING";
                if (resource.Expression is { } resourceValue) Expression(resourceValue, id, "RESOURCE");
                if (resource.Declaration is { } declared) Expression(declared, id, "RESOURCE");
                Node(resource.Statement, id, "USING_BODY", depth + 1);
                Opaque(node, "USING_EXIT_CLEANUP_NOT_MODELED");
                break;
            case LocalDeclarationStatementSyntax local:
                node["kind"] = "LOCAL";
                Expression(local.Declaration, id, "VALUE");
                if (local.UsingKeyword != default)
                {
                    node["scopeCleanup"] = local.AwaitKeyword != default ? "AWAIT_USING_CLEANUP_NOT_MODELED" : "USING_CLEANUP_NOT_MODELED";
                    _boundaries.Add(node["scopeCleanup"]!.GetValue<string>());
                }
                break;
            case ExpressionStatementSyntax expression:
                node["kind"] = "EXPRESSION";
                Expression(expression.Expression, id, "VALUE");
                break;
            case ReturnStatementSyntax returned:
                node["kind"] = "RETURN";
                if (returned.Expression is { } returnValue) Expression(returnValue, id, "RETURN_VALUE");
                break;
            case ThrowStatementSyntax thrown:
                node["kind"] = "THROW";
                if (thrown.Expression is { } throwValue) Expression(throwValue, id, "THROW_VALUE");
                break;
            case BreakStatementSyntax:
                node["kind"] = "BREAK";
                break;
            case ContinueStatementSyntax:
                node["kind"] = "CONTINUE";
                break;
            case EmptyStatementSyntax:
                node["kind"] = "EMPTY";
                break;
            case FinallyClauseSyntax:
                Opaque(node, "FINALLY_COMPLETION_NOT_MODELED");
                break;
            default:
                Opaque(node, "UNSUPPORTED_SOURCE_REGION:" + syntax.Kind());
                break;
        }
        return id;
    }

    private static void Loop(JsonObject node, string kind, string continuation)
    {
        node["kind"] = "LOOP";
        node["loopKind"] = kind;
        node["continuePhase"] = continuation;
    }

    private void Expression(SyntaxNode syntax, int region, string role)
    {
        if (_budget || Slots >= MaxSlots - 1)
        {
            _budget = true;
            Opaque(_nodes[region], "SOURCE_REGIONS_BUDGET");
            return;
        }
        _expressions.Add(new JsonObject { ["region"] = region, ["role"] = role, ["source"] = Span(syntax) });
        Sites(syntax, region, 0);
    }

    private void Sites(SyntaxNode syntax, int region, int depth)
    {
        if (_budget) return;
        if (Slots >= MaxSlots - 1 || depth > MaxDepth)
        {
            _budget = true;
            Opaque(_nodes[region], "SOURCE_REGIONS_BUDGET");
            return;
        }
        string? opaque = syntax switch
        {
            AnonymousFunctionExpressionSyntax => "DEFERRED_BODY_NOT_EXPANDED",
            ConditionalExpressionSyntax or ConditionalAccessExpressionSyntax => "CONDITIONAL_EXPRESSION_NOT_MODELED",
            BinaryExpressionSyntax binary when binary.Kind() is SyntaxKind.LogicalAndExpression or SyntaxKind.LogicalOrExpression or SyntaxKind.CoalesceExpression => "SHORT_CIRCUIT_EXPRESSION_NOT_MODELED",
            _ => null,
        };
        if (opaque is not null)
        {
            _sites.Add(new JsonObject { ["region"] = region, ["kind"] = "OPAQUE", ["source"] = Span(syntax), ["boundary"] = opaque });
            _boundaries.Add(opaque);
            return;
        }
        if (syntax is AwaitExpressionSyntax or InvocationExpressionSyntax or ObjectCreationExpressionSyntax or ImplicitObjectCreationExpressionSyntax)
        {
            var site = new JsonObject { ["region"] = region, ["source"] = Span(syntax),
                ["kind"] = syntax is AwaitExpressionSyntax ? "AWAIT" : syntax is InvocationExpressionSyntax ? "CALL" : "CONSTRUCT" };
            if (syntax is not AwaitExpressionSyntax)
            {
                var target = _model.GetSymbolInfo(syntax).Symbol as IMethodSymbol;
                var identity = target is null ? null : Identities.Method(target);
                if (identity is not null && identity.Length <= 512)
                {
                    site["target"] = identity;
                    site["resolution"] = "COMPILER_EXACT_TARGET_NOT_RUNTIME_DISPATCH";
                    site["targetInSource"] = target!.Locations.Any(location => location.IsInSource);
                }
                else site["resolution"] = "UNRESOLVED_OR_TARGET_BUDGET";
            }
            _sites.Add(site);
        }
        foreach (var child in syntax.ChildNodes()) Sites(child, region, depth + 1);
    }
}
