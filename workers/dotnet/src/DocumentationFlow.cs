using System.Text;
using System.Text.Json.Nodes;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;

namespace Codeclew.CSharp.Analyzer;

/// <summary>
/// Bounded source structure in the event vocabulary of the JVM documentation flows.
/// It is not a runtime trace or a control-flow proof: branches that need review
/// become explicit boundaries instead of guessed paths.
/// </summary>
public sealed class DocumentationFlow : CSharpSyntaxWalker
{
    public const string Schema = "codeclew-csharp-documentation-flow/1.0";
    private const int MaxEvents = 2048;
    private const int MaxBytes = 150_000;
    private const int MaxConditionLength = 512;

    private readonly SemanticModel _model;
    private readonly SourceAnchors _anchors;
    private readonly List<JsonObject> _events = [];
    private readonly SortedSet<string> _boundaries = new(StringComparer.Ordinal);
    private int _groups;

    private DocumentationFlow(SemanticModel model, SourceAnchors anchors)
    {
        _model = model;
        _anchors = anchors;
    }

    public static JsonObject Read(SemanticModel model, SourceAnchors anchors, IMethodSymbol method, SyntaxNode body)
    {
        var flow = new DocumentationFlow(model, anchors);
        if (body is ArrowExpressionClauseSyntax arrow)
        {
            flow.Visit(arrow.Expression);
            flow.Event(method.ReturnsVoid ? "STATEMENT" : "RETURN", arrow.Expression);
        }
        else
        {
            flow.Visit(body);
        }
        return flow.Result(method);
    }

    private JsonObject Result(IMethodSymbol method)
    {
        var events = _events;
        var bytes = 0;
        var kept = new List<JsonObject>();
        foreach (var row in events)
        {
            var cost = Encoding.UTF8.GetByteCount(row.ToJsonString()) + 2;
            if (kept.Count > 0 && bytes + cost > MaxBytes)
            {
                _boundaries.Add("DOCUMENTATION_FLOW_BYTE_BUDGET");
                break;
            }
            kept.Add(row);
            bytes += cost;
        }
        return new JsonObject
        {
            ["schema"] = Schema,
            ["authority"] = "ROSLYN_SOURCE_STRUCTURE",
            ["parameterTypes"] = new JsonArray(Identities.ParameterTypes(method).Select(name => (JsonNode?)JsonValue.Create(name)).ToArray()),
            ["events"] = new JsonArray(kept.Select(row => (JsonNode?)row.DeepClone()).ToArray()),
            ["boundaries"] = new JsonArray(_boundaries.Select(code => (JsonNode?)JsonValue.Create(code)).ToArray()),
        };
    }

    private JsonObject Event(string kind, SyntaxNode node)
    {
        var row = new JsonObject { ["kind"] = kind };
        _anchors.Apply(row, node.Span);
        if (_events.Count < MaxEvents)
        {
            _events.Add(row);
        }
        else
        {
            _boundaries.Add("DOCUMENTATION_FLOW_EVENT_BUDGET");
        }
        return row;
    }

    private void Boundary(string code, SyntaxNode node)
    {
        _boundaries.Add(code);
        Event("BOUNDARY", node);
    }

    private static string Condition(SyntaxNode node)
    {
        var text = node.ToString();
        return text.Length <= MaxConditionLength ? text : text[..MaxConditionLength];
    }

    public override void VisitIfStatement(IfStatementSyntax node)
    {
        Visit(node.Condition);
        var branch = Event("IF", node.Condition);
        branch["condition"] = Condition(node.Condition);
        branch["group"] = ++_groups;
        Visit(node.Statement);
        if (node.Else is { } otherwise)
        {
            Event("ELSE", otherwise.Statement);
            Visit(otherwise.Statement);
        }
        Event("END", node);
    }

    public override void VisitWhileStatement(WhileStatementSyntax node)
    {
        Event("LOOP", node.Condition)["condition"] = Condition(node.Condition);
        Visit(node.Condition);
        Visit(node.Statement);
        Event("END", node);
    }

    public override void VisitDoStatement(DoStatementSyntax node)
    {
        Event("LOOP", node)["condition"] = "do-while";
        Visit(node.Statement);
        Visit(node.Condition);
        Event("END", node);
    }

    public override void VisitForStatement(ForStatementSyntax node)
    {
        if (node.Declaration is { } declaration)
        {
            Visit(declaration);
        }
        foreach (var initializer in node.Initializers)
        {
            Visit(initializer);
        }
        Event("LOOP", node)["condition"] = "for-loop";
        if (node.Condition is { } condition)
        {
            Visit(condition);
        }
        Visit(node.Statement);
        foreach (var incrementor in node.Incrementors)
        {
            Visit(incrementor);
        }
        Event("END", node);
    }

    public override void VisitForEachStatement(ForEachStatementSyntax node)
    {
        Visit(node.Expression);
        Event("LOOP", node)["condition"] = "for-each";
        Visit(node.Statement);
        Event("END", node);
    }

    public override void VisitForEachVariableStatement(ForEachVariableStatementSyntax node)
    {
        Visit(node.Expression);
        Event("LOOP", node)["condition"] = "for-each";
        Visit(node.Statement);
        Event("END", node);
    }

    public override void VisitSwitchStatement(SwitchStatementSyntax node) => Boundary("SWITCH_FLOW_REQUIRES_SOURCE_REVIEW", node);
    public override void VisitSwitchExpression(SwitchExpressionSyntax node) => Boundary("SWITCH_FLOW_REQUIRES_SOURCE_REVIEW", node);
    public override void VisitTryStatement(TryStatementSyntax node) => Boundary("EXCEPTION_FLOW_REQUIRES_SOURCE_REVIEW", node);
    public override void VisitBreakStatement(BreakStatementSyntax node) => Boundary("BREAK_FLOW_REQUIRES_SOURCE_REVIEW", node);
    public override void VisitContinueStatement(ContinueStatementSyntax node) => Boundary("CONTINUE_FLOW_REQUIRES_SOURCE_REVIEW", node);
    public override void VisitGotoStatement(GotoStatementSyntax node) => Boundary("GOTO_FLOW_REQUIRES_SOURCE_REVIEW", node);
    public override void VisitLabeledStatement(LabeledStatementSyntax node) => Boundary("LABELED_STATEMENT_FLOW_REQUIRES_SOURCE_REVIEW", node);
    public override void VisitYieldStatement(YieldStatementSyntax node) => Boundary("ITERATOR_FLOW_REQUIRES_SOURCE_REVIEW", node);
    public override void VisitLockStatement(LockStatementSyntax node) => Boundary("SYNCHRONIZED_FLOW_REQUIRES_SOURCE_REVIEW", node);
    public override void VisitConditionalExpression(ConditionalExpressionSyntax node) => Boundary("TERNARY_FLOW_REQUIRES_SOURCE_REVIEW", node);
    public override void VisitLocalFunctionStatement(LocalFunctionStatementSyntax node) => _boundaries.Add("LOCAL_FUNCTION_BODY_NOT_EXPANDED");

    public override void VisitSimpleLambdaExpression(SimpleLambdaExpressionSyntax node) => Boundary("LAMBDA_EXECUTION_NOT_EXPANDED", node);
    public override void VisitParenthesizedLambdaExpression(ParenthesizedLambdaExpressionSyntax node) => Boundary("LAMBDA_EXECUTION_NOT_EXPANDED", node);
    public override void VisitAnonymousMethodExpression(AnonymousMethodExpressionSyntax node) => Boundary("LAMBDA_EXECUTION_NOT_EXPANDED", node);

    public override void VisitBinaryExpression(BinaryExpressionSyntax node)
    {
        if (node.Kind() is SyntaxKind.LogicalAndExpression or SyntaxKind.LogicalOrExpression or SyntaxKind.CoalesceExpression)
        {
            Boundary("SHORT_CIRCUIT_FLOW_REQUIRES_SOURCE_REVIEW", node);
            return;
        }
        base.VisitBinaryExpression(node);
    }

    public override void VisitConditionalAccessExpression(ConditionalAccessExpressionSyntax node)
    {
        // Calls behind '?.' are recorded in order; their execution depends on a null check.
        _boundaries.Add("NULL_CONDITIONAL_FLOW_REQUIRES_SOURCE_REVIEW");
        base.VisitConditionalAccessExpression(node);
    }

    public override void VisitReturnStatement(ReturnStatementSyntax node)
    {
        if (node.Expression is { } expression)
        {
            Visit(expression);
        }
        Event("RETURN", node);
    }

    public override void VisitThrowStatement(ThrowStatementSyntax node)
    {
        if (node.Expression is { } expression)
        {
            Visit(expression);
        }
        Event("THROW", node);
    }

    public override void VisitThrowExpression(ThrowExpressionSyntax node)
    {
        Visit(node.Expression);
        Event("THROW", node);
    }

    public override void VisitLocalDeclarationStatement(LocalDeclarationStatementSyntax node)
    {
        foreach (var variable in node.Declaration.Variables)
        {
            if (variable.Initializer is { } initializer)
            {
                Visit(initializer.Value);
            }
        }
        Event("LOCAL", node);
    }

    public override void VisitExpressionStatement(ExpressionStatementSyntax node)
    {
        Visit(node.Expression);
        var expression = node.Expression is AwaitExpressionSyntax awaited ? awaited.Expression : node.Expression;
        if (expression is not InvocationExpressionSyntax and not ConditionalAccessExpressionSyntax)
        {
            Event("STATEMENT", node);
        }
    }

    public override void VisitInvocationExpression(InvocationExpressionSyntax node)
    {
        // Receiver and argument calls are evaluated before this invocation.
        base.VisitInvocationExpression(node);
        var row = Event("CALL", node);
        if (_model.GetSymbolInfo(node).Symbol is IMethodSymbol method)
        {
            row["target"] = Identities.Method(method);
            row["resolution"] = "COMPILER_EXACT";
            row["http"] = new JsonObject();
        }
        else if (_model.GetOperation(node) is not null and not Microsoft.CodeAnalysis.Operations.IInvocationOperation)
        {
            // Delegate invocations and nameof() are not method calls with a static target.
            row["resolution"] = "DYNAMIC_DISPATCH";
            _boundaries.Add("DOCUMENTATION_CALL_TARGET_DYNAMIC");
        }
        else
        {
            _boundaries.Add("DOCUMENTATION_CALL_TARGET_UNRESOLVED");
        }
    }

    public override void VisitObjectCreationExpression(ObjectCreationExpressionSyntax node)
    {
        base.VisitObjectCreationExpression(node);
        Construct(node);
    }

    public override void VisitImplicitObjectCreationExpression(ImplicitObjectCreationExpressionSyntax node)
    {
        base.VisitImplicitObjectCreationExpression(node);
        Construct(node);
    }

    private void Construct(SyntaxNode node)
    {
        var row = Event("CONSTRUCT", node);
        if (_model.GetSymbolInfo(node).Symbol is IMethodSymbol constructor)
        {
            row["target"] = Identities.Method(constructor);
            row["resolution"] = "COMPILER_EXACT";
        }
        else
        {
            _boundaries.Add("DOCUMENTATION_CONSTRUCTOR_UNRESOLVED");
        }
    }
}
