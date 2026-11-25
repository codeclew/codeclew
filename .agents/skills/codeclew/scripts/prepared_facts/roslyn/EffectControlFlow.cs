// Project Roslyn CFG blocks and bound operations from sealed source selections.
using System.Text;
using System.Text.Json;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;
using Microsoft.CodeAnalysis.FlowAnalysis;
using Microsoft.CodeAnalysis.Operations;
using Microsoft.CodeAnalysis.Text;
using RetainedEffects;
using Codeclew.CSharp.Analyzer;

internal sealed class EffectControlFlow(SemanticModel model, BaseMethodDeclarationSyntax method, string text,
    bool sdkImports = false)
{
    internal sealed record Selection(string Reference, string File, int Start, int End);
    internal sealed record Request(Dictionary<string, string> Files, Selection[]? Selections, bool AllowSdkImports = false,
        bool DiscoverHostedWorkers = false);

    private readonly Dictionary<CaptureId, int> captures = [];
    private readonly Dictionary<IOperation, int> results = new(ReferenceEqualityComparer.Instance);
    private readonly List<Dictionary<string, object?>> steps = [];
    private readonly Dictionary<Microsoft.CodeAnalysis.Text.TextSpan, int> lexicalResults = [];
    private readonly HashSet<TextSpan> explicitCalls = [];

    internal static void Run(string input)
    {
        var request = JsonSerializer.Deserialize<Request>(input,
            new JsonSerializerOptions { PropertyNameCaseInsensitive = true })
            ?? throw new InvalidOperationException("Source selections are required.");
        var trees = request.Files.ToDictionary(pair => pair.Key,
            pair => CSharpSyntaxTree.ParseText(pair.Value, path: pair.Key));
        var references = ((string?)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES") ?? "")
            .Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries)
            .Where(path => Path.GetFileName(path) is var file &&
                (file.StartsWith("System.", StringComparison.Ordinal) ||
                 file.StartsWith("Microsoft.AspNetCore.", StringComparison.Ordinal) ||
                 file.StartsWith("Microsoft.Extensions.", StringComparison.Ordinal)))
            .Select(path => MetadataReference.CreateFromFile(path));
        var compilation = CSharpCompilation.Create("RetainedEffectProjection", trees.Values, references,
            new CSharpCompilationOptions(request.DiscoverHostedWorkers ? OutputKind.ConsoleApplication : OutputKind.DynamicallyLinkedLibrary));
        if (request.DiscoverHostedWorkers)
        {
            Console.WriteLine(JsonSerializer.Serialize(HostedWorkers.Discover(compilation)));
            return;
        }
        // SDK-generated imports may be outside the sealed files. Keep a
        // fallback compilation conditional and restricted to framework names.
        var withSdkImports = request.AllowSdkImports ? compilation.AddSyntaxTrees(CSharpSyntaxTree.ParseText("""
            global using System;
            global using System.Collections.Generic;
            global using System.Linq;
            global using System.Threading;
            global using System.Threading.Tasks;
            global using Microsoft.AspNetCore.Http;
            """)) : null;
        var selected = new Dictionary<string, object?>();
        foreach (var group in (request.Selections ?? []).GroupBy(selection => selection.File))
        {
            if (!trees.TryGetValue(group.Key, out var tree)) continue;
            var root = tree.GetRoot();
            if (root.DescendantTrivia(descendIntoTrivia: true).Any(trivia => trivia.Kind() is
                SyntaxKind.IfDirectiveTrivia or SyntaxKind.ElifDirectiveTrivia or SyntaxKind.DefineDirectiveTrivia
                or SyntaxKind.UndefDirectiveTrivia))
                continue;
            var source = request.Files[group.Key];
            var methods = root.DescendantNodes().OfType<BaseMethodDeclarationSyntax>()
                .Where(node => node is MethodDeclarationSyntax or ConstructorDeclarationSyntax)
                .Select(node => (node, range: Range(node, source))).ToArray();
            var model = compilation.GetSemanticModel(tree);
            var sdkModel = withSdkImports?.GetSemanticModel(tree);
            foreach (var selection in group)
            {
                var matches = methods.Where(m => ContainsOnly(selection, m.range, source) ||
                    m.node is MethodDeclarationSyntax && (m.node.Body ?? (SyntaxNode?)m.node.ExpressionBody) is { } body &&
                    ContainsOnly(selection, Range(body, source), source)).ToArray();
                if (matches.Length != 1) continue;
                var node = matches[0].node;
                var sdk = sdkModel is not null && HasSdkNames(node, model, sdkModel);
                var current = sdk ? sdkModel! : model;
                if (current.GetDeclaredSymbol(node) is not IMethodSymbol symbol) continue;
                try
                {
                    selected[selection.Reference] = new EffectControlFlow(current, node, source, sdk).Read(symbol);
                }
                catch (ArgumentException)
                {
                    // Some retained declarations have no executable CFG root.
                }
            }
        }
        Console.WriteLine(JsonSerializer.Serialize(selected));
    }

    private static bool HasSdkNames(BaseMethodDeclarationSyntax node, SemanticModel primary, SemanticModel fallback)
    {
        foreach (var name in node.DescendantNodes().OfType<SimpleNameSyntax>())
        {
            if (name.Identifier.ValueText is not ("Guid" or "CancellationToken" or "Task" or "ValueTask" or "List" or
                "IList" or "IReadOnlyList" or "IEnumerable" or "Dictionary" or "Select" or "FirstOrDefault" or
                "Results" or "TypedResults" or "IResult"))
                continue;
            if (primary.GetSymbolInfo(name).Symbol is { } current && current is not IErrorTypeSymbol)
                continue;
            if (fallback.GetSymbolInfo(name).Symbol?.ContainingAssembly?.Name is
                "System.Private.CoreLib" or "System.Linq" or "Microsoft.AspNetCore.Http.Results" or "Microsoft.AspNetCore.Http.Abstractions")
                return true;
        }
        return false;
    }

    private static bool ContainsOnly(Selection selection, (int start, int end) span, string source)
    {
        if (span == (selection.Start, selection.End)) return true;
        if (selection.Start > span.start || selection.End < span.end ||
            selection.End - selection.Start - span.end + span.start > 256)
            return false;
        var bytes = Encoding.UTF8.GetBytes(source);
        return string.IsNullOrWhiteSpace(Encoding.UTF8.GetString(bytes, selection.Start, span.start - selection.Start)) &&
            string.IsNullOrWhiteSpace(Encoding.UTF8.GetString(bytes, span.end, selection.End - span.end));
    }

    private static (int start, int end) Range(SyntaxNode syntax, string text) =>
        (Encoding.UTF8.GetByteCount(text.AsSpan(0, syntax.SpanStart)),
         Encoding.UTF8.GetByteCount(text.AsSpan(0, syntax.Span.End)));

    private object Site(SyntaxNode syntax)
    {
        var (start, end) = Range(syntax, text);
        var lines = syntax.GetLocation().GetLineSpan();
        return new { start, end, first = lines.StartLinePosition.Line + 1, last = lines.EndLinePosition.Line + 1 };
    }

    private static string MethodId(IMethodSymbol symbol) =>
        (symbol.ReducedFrom ?? symbol).OriginalDefinition.GetDocumentationCommentId() ?? "";

    private object? Read(IMethodSymbol symbol)
    {
        var cfg = model.GetOperation(method) switch
        {
            IMethodBodyOperation body => ControlFlowGraph.Create(body),
            IConstructorBodyOperation body => ControlFlowGraph.Create(body),
            _ => null,
        };
        if (cfg is null || cfg.Blocks.Length > 80)
            return null;
        var regions = new Dictionary<ControlFlowRegion, int>();
        void AddRegion(ControlFlowRegion region)
        {
            regions[region] = regions.Count;
            foreach (var nested in region.NestedRegions) AddRegion(nested);
        }
        AddRegion(cfg.Root);
        var blocks = new List<object>();
        foreach (var block in cfg.Blocks)
        {
            var start = steps.Count;
            foreach (var operation in block.Operations) Visit(operation, 0);
            var value = block.BranchValue;
            if (value is not null) Visit(value, 0);
            if (steps.Count > 160) return null;
            var throwing = block.FallThroughSuccessor?.Semantics is ControlFlowBranchSemantics.Throw
                or ControlFlowBranchSemantics.Rethrow;
            var terminator = value?.Syntax.AncestorsAndSelf().FirstOrDefault(node =>
                node is ThrowStatementSyntax or ThrowExpressionSyntax or ReturnStatementSyntax);
            // A valueless throw/return may be absent from the Roslyn operation list.
            var condition = value?.Syntax;
            while (condition?.Parent is ParenthesizedExpressionSyntax parentheses)
                condition = parentheses;
            var sourceTest = condition;
            var inverted = false;
            while (sourceTest?.Parent is ParenthesizedExpressionSyntax or PrefixUnaryExpressionSyntax)
            {
                if (sourceTest.Parent is PrefixUnaryExpressionSyntax unary)
                {
                    if (!unary.IsKind(SyntaxKind.LogicalNotExpression) ||
                        model.GetOperation(unary) is not IUnaryOperation { OperatorMethod: null, OperatorKind: UnaryOperatorKind.Not })
                        break;
                    inverted = !inverted;
                }
                sourceTest = sourceTest.Parent;
            }
            var decision = value is null ? null : new
            {
                site = Site(terminator ?? value.Syntax),
                value = OperationValue(value),
                explicitThrow = terminator is ThrowStatementSyntax or ThrowExpressionSyntax,
                // A lowered null test can use the syntax of its non-Boolean operand.
                sourcePredicate = block.ConditionKind != ControlFlowConditionKind.None &&
                    value is not IIsNullOperation && value.Syntax is ExpressionSyntax expression &&
                    (SourceCondition(condition) || model.GetTypeInfo(expression).Type?.SpecialType == SpecialType.System_Boolean),
                loopTest = sourceTest?.Parent switch
                {
                    WhileStatementSyntax node when node.Condition == sourceTest => "while",
                    DoStatementSyntax node when node.Condition == sourceTest => "do",
                    ForStatementSyntax node when node.Condition == sourceTest => "for",
                    _ => null,
                },
            };
            object? guard = null;
            if (value is not IIsNullOperation && sourceTest?.Parent is IfStatementSyntax branch && branch.Condition == sourceTest)
            {
                var trueArm = inverted ? branch.Else?.Statement : branch.Statement;
                var falseArm = inverted ? branch.Statement : branch.Else?.Statement;
                guard = new { whenTrue = GuardReturn(trueArm), whenFalse = GuardReturn(falseArm),
                    trueControl = GuardControl(trueArm), falseControl = GuardControl(falseArm) };
            }
            object? Edge(ControlFlowBranch? edge) => edge is null ? null : new
            {
                target = edge.Destination?.Ordinal,
                kind = edge.Semantics.ToString(),
                cleanups = edge.FinallyRegions.Select(region => regions[region]).ToArray(),
            };
            blocks.Add(new
            {
                id = block.Ordinal,
                region = regions[block.EnclosingRegion],
                kind = block.Kind.ToString(),
                reachable = block.IsReachable,
                steps = steps.Skip(start).ToArray(),
                decision,
                guard,
                branch = block.ConditionKind.ToString(),
                next = Edge(block.FallThroughSuccessor),
                conditional = Edge(block.ConditionalSuccessor),
                throwType = throwing && value is not null ? ExceptionTypes(value) : [],
            });
        }
        return new
        {
            schema = "clew-effect-cfg/1",
            sdkImports,
            site = Site(method),
            declarationStart = method.SpanStart,
            symbol = MethodId(symbol),
            nativeOwner = $"method:{Identities.Owner(symbol)}#{Identities.MethodName(symbol)}",
            returnDescriptor = symbol.ReturnType.TypeKind == TypeKind.Error ? null : Identities.Descriptor(symbol.ReturnType),
            returnFamily = ManagedValues.ResourceFamily(symbol.ReturnType),
            parameters = symbol.Parameters.Select(parameter => new
            {
                id = parameter.Ordinal, name = parameter.Name,
                descriptor = parameter.Type.TypeKind == TypeKind.Error ? null : Identities.Descriptor(parameter.Type),
                unboundToken = IsUnboundCancellationParameter(parameter),
                input = ManagedValues.Input(parameter),
                resourceFamily = ManagedValues.ResourceFamily(parameter.Type),
                site = parameter.DeclaringSyntaxReferences.SingleOrDefault()?.GetSyntax() is ParameterSyntax syntax
                    ? Site(syntax) : null,
            }).ToArray(),
            blocks,
            explicitCallSitesComplete = method.DescendantNodes().Where(IsExplicitCall)
                .All(node => explicitCalls.Contains(node.Span) || model.GetOperation(node) is INameOfOperation),
            regions = regions.Select(row => new
            {
                id = row.Value,
                kind = row.Key.Kind.ToString(),
                entry = row.Key.FirstBlockOrdinal,
                parent = row.Key.EnclosingRegion is { } parent ? (int?)regions[parent] : null,
                type = row.Key.ExceptionType?.ToDisplayString(),
                site = RegionSite(cfg, row.Key),
            }).ToArray(),
            loopStructure = LoopStructure(),
            catchContinuations = CatchContinuations(),
            awaitedGuardWrites = AwaitedGuardWrites(cfg),
            catchReturns = CatchReturns(),
        };
    }

    private static bool IsExplicitCall(SyntaxNode node) => node is InvocationExpressionSyntax or
        BaseObjectCreationExpressionSyntax or ConstructorInitializerSyntax;

    private static bool CanDescend(SyntaxNode node) => node is not (AnonymousFunctionExpressionSyntax or LocalFunctionStatementSyntax);

    private static bool IsLoop(SyntaxNode node) => node is WhileStatementSyntax or DoStatementSyntax or ForStatementSyntax or
        ForEachStatementSyntax or ForEachVariableStatementSyntax;

    private object[] LoopStructure()
    {
        var facts = new List<object>();
        foreach (var loop in method.DescendantNodes(CanDescend).Where(IsLoop).Take(6))
        {
            (string Kind, ExpressionSyntax? Test) shape = loop switch
            {
                WhileStatementSyntax node => ("while", node.Condition),
                DoStatementSyntax node => ("do", node.Condition),
                ForStatementSyntax node => ("for", node.Condition),
                _ => ("foreach", null),
            };
            if (shape.Test is not { } test) continue;
            foreach (var clause in loop.DescendantNodes(CanDescend).OfType<CatchClauseSyntax>()
                .Where(clause => clause.Ancestors().FirstOrDefault(IsLoop) == loop).Take(6))
            {
                if (clause.Block.Statements.LastOrDefault() is { } last && GuardControl(clause.Block) is { } &&
                    last is BreakStatementSyntax or ContinueStatementSyntax && last.Ancestors().FirstOrDefault(node =>
                        IsLoop(node) || last is BreakStatementSyntax && node is SwitchStatementSyntax) == loop)
                    facts.Add(new { kind = last is BreakStatementSyntax ? "catch-break" : "catch-continue", loop = shape.Kind,
                        test = Site(test), site = Site(last), exception = clause.Declaration is { } declared ? Site(declared.Type) : null,
                        filter = clause.Filter is { } filter ? Site(filter.FilterExpression) : null });
                else if (model.AnalyzeControlFlow(clause.Block) is { Succeeded: true, EndPointIsReachable: true })
                    facts.Add(new { kind = "catch-fallthrough", loop = shape.Kind, test = Site(test), site = Site(clause.Block),
                        exception = clause.Declaration is { } declared ? Site(declared.Type) : null,
                        filter = clause.Filter is { } filter ? Site(filter.FilterExpression) : null,
                        actions = clause.Block.Statements.Select(InvocationName)
                            .Where(expression => expression is SimpleNameSyntax or MemberAccessExpressionSyntax).Take(4)
                            .Select(expression => Site(expression!)).ToArray() });
            }
            foreach (var statement in loop.DescendantNodes(CanDescend).OfType<ExpressionStatementSyntax>()
                .Where(statement => statement.Ancestors().FirstOrDefault(IsLoop) == loop).Take(20))
            {
                if (statement.Expression is not AwaitExpressionSyntax { Expression: InvocationExpressionSyntax invocation } ||
                    !AtLoopTail(statement, loop) || LoopDelay(invocation) is not { } delay)
                    continue;
                facts.Add(new { kind = "delay", loop = shape.Kind, test = Site(test), site = Site(statement),
                    token = delay.Token is { } token ? Site(token) : null, milliseconds = delay.Milliseconds, conditional = delay.Conditional });
            }
        }
        return facts.Take(16).ToArray();
    }

    // A condition's own await must resume before a field assignment in its selected arm.
    private object[] AwaitedGuardWrites(ControlFlowGraph cfg)
    {
        var reachable = cfg.Blocks.Where(block => block.IsReachable).ToArray();
        var decisions = reachable.Where(block => block.ConditionKind != ControlFlowConditionKind.None && block.BranchValue is not null)
            .SelectMany(block => block.BranchValue!.DescendantsAndSelf()).OfType<IAwaitOperation>()
            .DistinctBy(operation => operation.Syntax.Span).ToDictionary(operation => operation.Syntax.Span);
        var assignments = reachable.SelectMany(block => block.Operations).SelectMany(operation => operation.DescendantsAndSelf())
            .Where(operation => operation is IAssignmentOperation or IIncrementOrDecrementOperation)
            .DistinctBy(operation => operation.Syntax.Span).ToDictionary(operation => operation.Syntax.Span);
        var facts = new List<object>();
        foreach (var branch in method.DescendantNodes(CanDescend).OfType<IfStatementSyntax>().Take(24))
        {
            ExpressionSyntax condition = branch.Condition;
            while (condition is ParenthesizedExpressionSyntax or PrefixUnaryExpressionSyntax)
            {
                if (condition is ParenthesizedExpressionSyntax parentheses)
                    condition = parentheses.Expression;
                else if (condition is PrefixUnaryExpressionSyntax unary &&
                    model.GetOperation(unary) is IUnaryOperation { OperatorMethod: null, OperatorKind: UnaryOperatorKind.Not })
                    condition = unary.Operand;
                else
                    break;
            }
            if (condition is not AwaitExpressionSyntax awaited || !decisions.TryGetValue(awaited.Span, out var operation))
                continue;
            foreach (var (arm, truth) in new[] { (branch.Statement, true), (branch.Else?.Statement, false) })
            {
                IEnumerable<StatementSyntax> statements = arm switch
                {
                    BlockSyntax block => block.Statements,
                    null => Array.Empty<StatementSyntax>(),
                    _ => new[] { arm },
                };
                foreach (var statement in statements.Take(8).OfType<ExpressionStatementSyntax>())
                {
                    if (!assignments.TryGetValue(statement.Expression.Span, out var write))
                        continue;
                    var target = write switch
                    {
                        IAssignmentOperation assignment => assignment.Target,
                        IIncrementOrDecrementOperation increment => increment.Target,
                        _ => null,
                    };
                    if (!(target is IFieldReferenceOperation ||
                        target is IPropertyReferenceOperation { Arguments.IsEmpty: true } property && IsAutoProperty(property.Property)))
                        continue;
                    facts.Add(new { condition = Site(branch.Condition), awaitSite = Site(awaited),
                        when = truth, write = Site(statement.Expression),
                        callSite = operation.Operation is IInvocationOperation invocation ? Site(invocation.Syntax) : null });
                }
            }
        }
        return facts.Take(12).ToArray();
    }

    // Roslyn can lower a bare return to a regular edge to Exit, with no branch value.
    private object[] CatchReturns()
    {
        var facts = new List<object>();
        foreach (var clause in method.DescendantNodes(CanDescend).OfType<CatchClauseSyntax>().Take(8))
        {
            if (model.AnalyzeControlFlow(clause.Block) is not
                { Succeeded: true, StartPointIsReachable: true, EndPointIsReachable: false } flow ||
                clause.Block.Statements.LastOrDefault() is not ReturnStatementSyntax { Expression: null } returned ||
                !flow.ReturnStatements.Contains(returned) ||
                model.AnalyzeControlFlow(returned) is not { Succeeded: true, StartPointIsReachable: true })
                continue;
            facts.Add(new { handler = Site(clause), returned = Site(returned),
                exception = clause.Declaration is { } declaration ? Site(declaration.Type) : null,
                filter = clause.Filter is { } filter ? Site(filter.FilterExpression) : null });
        }
        return facts.ToArray();
    }

    private object[] CatchContinuations()
    {
        var facts = new List<object>();
        foreach (var clause in method.DescendantNodes(CanDescend).OfType<CatchClauseSyntax>().Take(8))
        {
            if (clause.Parent is not TryStatementSyntax { Finally: null, Parent: BlockSyntax surrounding } attempt ||
                model.AnalyzeControlFlow(clause.Block) is not
                    { Succeeded: true, StartPointIsReachable: true, EndPointIsReachable: true })
                continue;
            var following = surrounding.Statements.Skip(surrounding.Statements.IndexOf(attempt) + 1);
            var prefix = FollowingAwaits(following).ToArray();
            var next = following.Skip(prefix.Length).FirstOrDefault();
            var scope = attempt.Ancestors().OfType<IfStatementSyntax>().Reverse().Take(5)
                .Select(branch => (Test: branch.Condition, Truth: branch.Statement.Span.Contains(attempt.Span)))
                .Concat(CatchExitGuards(clause.Block)).ToArray();
            var handled = clause.Block.Statements.TakeWhile(statement => InvocationName(statement) is not null).Take(3)
                .Select(statement => Site(InvocationName(statement)!)).ToArray();
            var loop = attempt.Ancestors().FirstOrDefault(IsLoop);

            void Add(IEnumerable<AwaitExpressionSyntax> awaits, IEnumerable<(ExpressionSyntax Test, bool Truth)> checks)
            {
                var sites = awaits.Take(4).Select(Site).ToArray();
                if (sites.Length == 0 || facts.Count == 12) return;
                facts.Add(new
                {
                    site = Site(clause.Block),
                    clause = clause.Declaration is { } declaration ? Site(declaration.Type) : null,
                    filter = clause.Filter is { } filter ? Site(filter.FilterExpression) : null,
                    loop = loop switch
                    {
                        WhileStatementSyntax => "while",
                        DoStatementSyntax => "do",
                        ForStatementSyntax => "for",
                        ForEachStatementSyntax or ForEachVariableStatementSyntax => "foreach",
                        _ => null,
                    },
                    loopEntryTest = loop switch
                    {
                        WhileStatementSyntax node => Site(node.Condition),
                        ForStatementSyntax { Condition: { } condition } => Site(condition),
                        _ => null,
                    },
                    tests = checks.Select(check => new { site = Site(check.Test), when = check.Truth }).ToArray(),
                    actions = handled,
                    awaits = sites,
                });
            }

            Add(prefix, scope);
            if (next is not IfStatementSyntax branch || prefix.Length == 4) continue;
            foreach (var (arm, truth) in new[] { (branch.Statement, true), (branch.Else?.Statement, false) })
            {
                var known = model.GetConstantValue(branch.Condition);
                if (arm is null || known.HasValue && known.Value is bool fixedValue && fixedValue != truth) continue;
                IEnumerable<StatementSyntax> statements = arm is BlockSyntax block ? block.Statements : [arm];
                var awaits = FollowingAwaits(statements).ToArray();
                if (awaits.Length > 0)
                    Add(prefix.Concat(awaits), scope.Append((branch.Condition, truth)));
            }
        }
        return facts.ToArray();
    }

    private IEnumerable<(ExpressionSyntax Test, bool Truth)> CatchExitGuards(BlockSyntax block)
    {
        if (block.DescendantNodes(CanDescend).Any(node => node is GotoStatementSyntax or LabeledStatementSyntax))
            yield break;
        foreach (var branch in block.Statements.OfType<IfStatementSyntax>().Take(4))
        {
            var thenExits = model.AnalyzeControlFlow(branch.Statement) is { Succeeded: true, EndPointIsReachable: false };
            var elseExits = branch.Else is { } otherwise &&
                model.AnalyzeControlFlow(otherwise.Statement) is { Succeeded: true, EndPointIsReachable: false };
            if (thenExits != elseExits)
                yield return (branch.Condition, elseExits);
        }
    }

    private static IEnumerable<AwaitExpressionSyntax> FollowingAwaits(IEnumerable<StatementSyntax> statements)
    {
        foreach (var statement in statements.Take(4))
        {
            var awaited = statement switch
            {
                ExpressionStatementSyntax { Expression: AwaitExpressionSyntax expression } => expression,
                LocalDeclarationStatementSyntax { UsingKeyword.RawKind: 0, Declaration.Variables.Count: 1 } declared =>
                    declared.Declaration.Variables[0].Initializer?.Value as AwaitExpressionSyntax,
                _ => null,
            };
            if (awaited is not { Expression: InvocationExpressionSyntax }) yield break;
            yield return awaited;
        }
    }

    private static ExpressionSyntax? InvocationName(StatementSyntax statement) => statement switch
    {
        ExpressionStatementSyntax { Expression: InvocationExpressionSyntax invocation } => invocation.Expression,
        ExpressionStatementSyntax { Expression: AwaitExpressionSyntax { Expression: InvocationExpressionSyntax invocation } }
            => invocation.Expression,
        _ => null,
    };

    private (ExpressionSyntax? Token, int? Milliseconds, bool Conditional)? LoopDelay(InvocationExpressionSyntax invocation)
    {
        if (model.GetOperation(invocation) is IInvocationOperation call)
        {
            if (Role(call.TargetMethod) == "AWAITER_CONFIGURATION" && call.Instance is IInvocationOperation decorated)
                call = decorated;
            if (Role(call.TargetMethod) != "POLL_DELAY") return null;
            var token = call.Arguments.FirstOrDefault(argument => !argument.IsImplicit && AdoOperations.IsCancellationToken(argument.Parameter?.Type));
            var milliseconds = call.Arguments.FirstOrDefault(argument => argument.Parameter?.Name == "millisecondsDelay");
            return ((token?.Syntax as ArgumentSyntax)?.Expression,
                milliseconds?.Value.ConstantValue is { HasValue: true, Value: int count } && count >= 0 ? count : null, false);
        }
        if (invocation.Expression is MemberAccessExpressionSyntax { Name.Identifier.ValueText: "ConfigureAwait",
                Expression: InvocationExpressionSyntax inner } && invocation.ArgumentList.Arguments is [{ Expression: LiteralExpressionSyntax option }] &&
            option.Token.Value is bool)
            invocation = inner;
        var binding = model.GetSymbolInfo(invocation);
        if (binding.Symbol is not null || !binding.CandidateSymbols.IsEmpty ||
            invocation.Expression is not MemberAccessExpressionSyntax { Name.Identifier.ValueText: "Delay" } access ||
            access.Expression.WithoutTrivia().ToString() is not ("Task" or "System.Threading.Tasks.Task" or "global::System.Threading.Tasks.Task"))
            return null;
        var arguments = invocation.ArgumentList.Arguments;
        if (arguments.Count is not (1 or 2) || arguments[0].NameColon is not null || arguments[0].RefKindKeyword != default ||
            arguments[0].Expression is not LiteralExpressionSyntax { Token.Value: int literal } || literal < 0)
            return null;
        return (arguments.Count == 2 && arguments[1].RefKindKeyword == default ? arguments[1].Expression : null, literal, true);
    }

    private static bool AtLoopTail(StatementSyntax statement, SyntaxNode loop)
    {
        while (statement.Parent is BlockSyntax block)
        {
            var last = block.Statements.LastOrDefault();
            if (last is ContinueStatementSyntax jump && block.Statements.Count >= 2 && block.Statements[^2] == statement &&
                jump.Ancestors().FirstOrDefault(IsLoop) == loop &&
                !jump.Ancestors().TakeWhile(node => node != loop).OfType<TryStatementSyntax>().Any(attempt => attempt.Finally is not null))
                return true;
            if (last != statement) return false;
            if (block.Parent == loop) return true;
            if (block.Parent is CatchClauseSyntax { Parent: TryStatementSyntax { Finally: null } guarded })
                statement = guarded;
            else if (block.Parent is IfStatementSyntax conditional)
                statement = conditional;
            else return false;
        }
        return statement.Parent == loop;
    }

    private object? RegionSite(ControlFlowGraph cfg, ControlFlowRegion region)
    {
        if (region.Kind is not (ControlFlowRegionKind.Catch or ControlFlowRegionKind.Finally))
            return null;
        foreach (var block in cfg.Blocks.Skip(region.FirstBlockOrdinal).Take(region.LastBlockOrdinal - region.FirstBlockOrdinal + 1))
            foreach (var syntax in block.Operations.Select(operation => operation.Syntax).Append(block.BranchValue?.Syntax))
                if (syntax?.AncestorsAndSelf().FirstOrDefault(node => region.Kind == ControlFlowRegionKind.Catch
                    ? node is CatchClauseSyntax : node is FinallyClauseSyntax) is { } clause)
                    return Site(clause);
        return null;
    }

    private static bool SourceCondition(SyntaxNode? condition) => condition?.Parent switch
    {
        IfStatementSyntax statement => statement.Condition == condition,
        WhileStatementSyntax statement => statement.Condition == condition,
        DoStatementSyntax statement => statement.Condition == condition,
        ForStatementSyntax statement => statement.Condition == condition,
        ConditionalExpressionSyntax expression => expression.Condition == condition,
        CatchFilterClauseSyntax clause => clause.FilterExpression == condition,
        _ => false,
    };

    private object? GuardReturn(StatementSyntax? statement) => statement switch
    {
        ReturnStatementSyntax returned => Site(returned),
        BlockSyntax block when block.Statements.LastOrDefault() is ReturnStatementSyntax { Expression: not null } returned => Site(returned),
        BlockSyntax { Statements.Count: 1 } block when block.Statements[0] is ReturnStatementSyntax returned => Site(returned),
        _ => null,
    };

    private object? GuardControl(StatementSyntax? statement)
    {
        var exit = statement is BlockSyntax block ? block.Statements.LastOrDefault() : statement;
        if (exit is not (ContinueStatementSyntax or BreakStatementSyntax) ||
            model.GetOperation(exit) is not IBranchOperation branch ||
            model.AnalyzeControlFlow(exit) is not { Succeeded: true, StartPointIsReachable: true })
            return null;
        var target = exit.Ancestors().FirstOrDefault(node => node is WhileStatementSyntax or DoStatementSyntax or
            ForStatementSyntax or ForEachStatementSyntax or ForEachVariableStatementSyntax ||
            branch.BranchKind == BranchKind.Break && node is SwitchStatementSyntax);
        return target is null ? null : new { kind = branch.BranchKind.ToString(), site = Site(exit),
            target = target is SwitchStatementSyntax ? "switch" : "loop" };
    }

    private static string[] ExceptionTypes(IOperation operation)
    {
        while (operation is IConversionOperation conversion) operation = conversion.Operand;
        if (operation is not IObjectCreationOperation || operation.Type is not INamedTypeSymbol { TypeKind: not TypeKind.Error } type)
            return [];
        var types = new List<string>();
        for (var current = type; current is not null; current = current.BaseType)
            types.Add(current.ToDisplayString());
        return types.ToArray();
    }

    private int Capture(CaptureId id)
    {
        if (!captures.TryGetValue(id, out var number)) captures[id] = number = captures.Count;
        return number;
    }

    private object? Value(IOperation operation, int depth = 0, IParameterSymbol? selector = null)
    {
        if (depth > 8) return null;
        var constant = operation.ConstantValue;
        if (constant.HasValue && constant.Value is null or bool or string or int or long)
            return new { kind = "literal", value = constant.Value };
        if (results.TryGetValue(operation, out var step)) return new { kind = "result", id = step };
        if (operation is IAwaitOperation && lexicalResults.TryGetValue(operation.Syntax.Span, out step))
            return new { kind = "result", id = step };
        object? Child(IOperation? child) => child is null ? null : Value(child, depth + 1, selector);
        return operation switch
        {
            ILocalReferenceOperation local => LocalValue(local.Local),
            IParameterReferenceOperation parameter when selector is not null &&
                SymbolEqualityComparer.Default.Equals(parameter.Parameter, selector) => new { kind = "selector-item" },
            IParameterReferenceOperation parameter => ParameterValue(parameter.Parameter),
            IInstanceReferenceOperation => new { kind = "instance" },
            ITupleOperation tuple => new { kind = "tuple", items = tuple.Elements.Select(Child).ToArray() },
            IAnonymousObjectCreationOperation anonymous => new { kind = "record", fields = anonymous.Initializers
                .OfType<ISimpleAssignmentOperation>()
                .Where(item => item.Target is IPropertyReferenceOperation)
                .Select(item => new { name = ((IPropertyReferenceOperation)item.Target).Property.Name, value = Child(item.Value) })
                .ToArray() },
            IFieldReferenceOperation field when TuplePosition(field.Field) is { } index =>
                new { kind = "tuple-slot", index, owner = Child(field.Instance) },
            IFieldReferenceOperation field => new { kind = "member", id = field.Field.OriginalDefinition.GetDocumentationCommentId(),
                name = field.Field.Name, owner = Child(field.Instance), direct = true, shared = field.Field.IsStatic },
            IPropertyReferenceOperation property when ManagedValues.JsonProperty(property.Property) == "json-root" =>
                new { kind = "json-root", owner = Child(property.Instance) },
            IPropertyReferenceOperation property when ManagedValues.JsonProperty(property.Property) == "json-key" &&
                property.Arguments.Length == 1 =>
                new { kind = "json-key", owner = Child(property.Instance), key = Child(property.Arguments[0].Value) },
            IPropertyReferenceOperation property when property.Property.ContainingType.IsAnonymousType =>
                new { kind = "record-member", name = property.Property.Name, owner = Child(property.Instance) },
            IPropertyReferenceOperation property when NullableProperty(property.Property) == "Value" => Child(property.Instance),
            IPropertyReferenceOperation property when NullableProperty(property.Property) == "HasValue" =>
                new { kind = "different", left = Child(property.Instance), right = new { kind = "literal", value = (object?)null } },
            IPropertyReferenceOperation property when AdoOperations.Property(property.Property) is { } member && property.Arguments.IsEmpty =>
                new { kind = "ado-member", name = member, owner = Child(property.Instance), typed = true },
            IPropertyReferenceOperation property when AdoOperations.Property(property.Property) == "Item" && property.Arguments.Length == 1 =>
                new { kind = "ado-index", owner = Child(property.Instance), selector = Child(property.Arguments[0].Value), typed = true },
            IPropertyReferenceOperation property when property.Arguments.IsEmpty =>
                new { kind = "member", id = property.Property.OriginalDefinition.GetDocumentationCommentId(),
                    name = property.Property.Name, owner = Child(property.Instance), direct = IsAutoProperty(property.Property),
                    shared = property.Property.IsStatic },
            IFlowCaptureReferenceOperation capture => new { kind = "capture", id = Capture(capture.Id) },
            IConversionOperation conversion when conversion.OperatorMethod is null => Child(conversion.Operand),
            IUnaryOperation { OperatorKind: UnaryOperatorKind.Not, OperatorMethod: null } unary =>
                new { kind = "not", value = Child(unary.Operand) },
            IIsPatternOperation { Pattern: IConstantPatternOperation pattern } tested =>
                new { kind = "equal", left = Child(tested.Value), right = Child(pattern.Value) },
            IIsPatternOperation { Pattern: INegatedPatternOperation { Pattern: IConstantPatternOperation pattern } } tested =>
                new { kind = "different", left = Child(tested.Value), right = Child(pattern.Value) },
            IIsNullOperation isNull => new { kind = "equal", left = Child(isNull.Operand), right = new { kind = "literal", value = (object?)null } },
            IBinaryOperation binary when binary.OperatorKind is BinaryOperatorKind.Equals or BinaryOperatorKind.NotEquals &&
                (binary.OperatorMethod is null || binary.OperatorMethod.ContainingType.SpecialType == SpecialType.System_String) =>
                new { kind = binary.OperatorKind == BinaryOperatorKind.Equals ? "equal" : "different",
                    left = Child(binary.LeftOperand), right = Child(binary.RightOperand) },
            IInvalidOperation when operation.Syntax is MemberAccessExpressionSyntax member => InferredDataMember(member),
            _ => null,
        };
    }

    private object? Selector(LambdaExpressionSyntax? lambda)
    {
        if (lambda?.Body is not ExpressionSyntax expression ||
            model.GetOperation(lambda) is not IAnonymousFunctionOperation { Symbol.Parameters.Length: 1 } function ||
            model.GetOperation(expression) is not { } body)
            return null;
        return Value(body, selector: function.Symbol.Parameters[0]);
    }

    private static int? TuplePosition(IFieldSymbol field)
    {
        if (field.ContainingType is not { IsTupleType: true } tuple) return null;
        var reference = field.CorrespondingTupleField ?? field;
        for (var index = 0; index < tuple.TupleElements.Length; index++)
        {
            var item = tuple.TupleElements[index];
            if (SymbolEqualityComparer.Default.Equals(item.CorrespondingTupleField ?? item, reference)) return index;
        }
        return null;
    }

    private static string? NullableProperty(IPropertySymbol property) =>
        property.ContainingType.OriginalDefinition.SpecialType == SpecialType.System_Nullable_T ? property.Name : null;

    private object ParameterValue(IParameterSymbol parameter)
    {
        var bound = SymbolEqualityComparer.Default.Equals(parameter.ContainingSymbol, model.GetDeclaredSymbol(method));
        return new { kind = bound ? "parameter" : "captured-parameter", id = parameter.Ordinal,
            name = parameter.Name, scope = parameter.ContainingSymbol.GetDocumentationCommentId() };
    }

    private static object LocalValue(ILocalSymbol local) =>
        new { kind = "local", id = local.DeclaringSyntaxReferences.FirstOrDefault()?.Span.Start, name = local.Name };

    private static bool IsAutoProperty(IPropertySymbol property) => property.ContainingType.IsAnonymousType ||
        !property.IsAbstract && !property.IsVirtual && !property.IsOverride && property.ContainingType.TypeKind != TypeKind.Interface &&
        property.DeclaringSyntaxReferences.Select(reference => reference.GetSyntax()).Any(node => node switch
        {
            PropertyDeclarationSyntax { AccessorList.Accessors.Count: > 0 } declaration =>
                declaration.AccessorList!.Accessors.All(accessor => accessor.Body is null && accessor.ExpressionBody is null),
            ParameterSyntax { Parent.Parent: RecordDeclarationSyntax } => property.ContainingType.IsRecord,
            _ => false,
        });

    private static object? BoundTarget(IMethodSymbol target, bool isVirtual)
    {
        var declaration = (target.ReducedFrom ?? target).OriginalDefinition.DeclaringSyntaxReferences;
        if (isVirtual || target.MethodKind is MethodKind.DelegateInvoke or MethodKind.LocalFunction ||
            declaration.Length != 1 || declaration[0].GetSyntax() is not BaseMethodDeclarationSyntax node ||
            node is not (MethodDeclarationSyntax or ConstructorDeclarationSyntax) ||
            node.Body is null && node.ExpressionBody is null)
            return null;
        return new { file = node.SyntaxTree.FilePath, start = node.SpanStart, symbol = MethodId(target) };
    }

    private static bool ReturnsTask(IMethodSymbol target) => !target.ReturnsVoid &&
        (target.IsAsync || target.ReturnType is INamedTypeSymbol type &&
         type.ContainingNamespace.ToDisplayString() == "System.Threading.Tasks" && type.Name is "Task" or "ValueTask");

    private static string? Role(IMethodSymbol target)
    {
        // Runtime framework identities available in this standalone compilation.
        var type = target.ContainingType.ToDisplayString();
        var assembly = target.ContainingAssembly.Name;
        if (assembly == "System.Private.CoreLib" && target.Name == "ConfigureAwait" &&
            target.ContainingNamespace.ToDisplayString() == "System.Threading.Tasks" &&
            target.ContainingType.Name is "Task" or "ValueTask")
            return "AWAITER_CONFIGURATION";
        if (AdoOperations.Role(target) is { } dataRole) return dataRole;
        if (ManagedValues.IsList(target.ContainingType) && target.Name is "Add" or "Clear")
            return target.Name == "Add" ? "LIST_ADD" : "LIST_CLEAR";
        if (assembly == "System.Private.CoreLib" && type == "System.Threading.Tasks.Task" && target.Name == "Delay")
            return "POLL_DELAY";
        if (assembly == "System.Net.Http" && type == "System.Net.Http.HttpClient" &&
            target.Name is "Send" or "SendAsync" or "PostAsync" or "PutAsync" or "DeleteAsync" or "PatchAsync")
            return "EXTERNAL_REQUEST";
        if (assembly == "System.Private.CoreLib" && type == "System.IO.File" &&
            target.Name is "WriteAllText" or "WriteAllTextAsync" or "WriteAllBytes" or "WriteAllBytesAsync"
                or "AppendAllText" or "AppendAllTextAsync")
            return "PERSISTENCE_REQUEST";
        return null;
    }

    private static string? DirectEffectFamily(IMethodSymbol target, string? role)
    {
        var owner = target.ContainingType;
        var type = owner.ToDisplayString();
        var assembly = target.ContainingAssembly.Name;
        if (assembly == "System.Net.Http" && type is "System.Net.Http.HttpClient" or "System.Net.Http.HttpMessageInvoker" &&
            target.Name is "Send" or "SendAsync" or "GetAsync" or "GetStringAsync" or "GetByteArrayAsync" or "GetStreamAsync" or
                "PostAsync" or "PutAsync" or "DeleteAsync" or "PatchAsync")
            return "HTTP";
        if (role is not null && (role.StartsWith("ADO_", StringComparison.Ordinal) ||
                role.StartsWith("TRANSACTION_", StringComparison.Ordinal)) ||
            AdoOperations.IsType(owner, "DbConnection") && target.Name is "Open" or "OpenAsync" or "Close" or "CloseAsync")
            return "database";
        if (assembly == "System.Data.Common" && (type, target.Name) is
            ("System.Data.IDbConnection", "Open" or "Close" or "CreateCommand" or "BeginTransaction") or
            ("System.Data.IDbCommand", "ExecuteReader" or "ExecuteNonQuery" or "ExecuteScalar" or "CreateParameter") or
            ("System.Data.IDbTransaction", "Commit" or "Rollback"))
            return "database";
        if ((assembly == "System.Private.CoreLib" && type is "System.IO.File" or "System.IO.Directory" or "System.IO.FileStream" ||
             assembly == "System.IO.FileSystem.Watcher" && type == "System.IO.FileSystemWatcher") && !target.IsImplicitlyDeclared)
            return "file system";
        if (assembly == "System.Diagnostics.Process" && type == "System.Diagnostics.Process" && target.Name is "Start" or "Kill")
            return "process";
        if (role is "AWAITER_CONFIGURATION" or "POLL_DELAY" or "LIST_ADD" or "LIST_CLEAR" ||
            assembly == "System.Private.CoreLib" && type == "System.Threading.Tasks.Task" && target.Name == "FromResult")
            return "local";
        return null;
    }

    private void Add(IOperation operation, string kind, params (string key, object? value)[] details)
    {
        var id = steps.Count;
        results[operation] = id;
        if (kind is "CALL" or "AWAIT" or "CONSTRUCT" or "ACCESS") lexicalResults.TryAdd(operation.Syntax.Span, id);
        if (kind is "CALL" or "CONSTRUCT" && IsExplicitCall(operation.Syntax)) explicitCalls.Add(operation.Syntax.Span);
        var row = new Dictionary<string, object?> { ["id"] = id, ["kind"] = kind, ["site"] = Site(operation.Syntax) };
        foreach (var (key, value) in details) row[key] = value;
        steps.Add(row);
    }

    private object? LexicalValue(ExpressionSyntax expression)
    {
        if (lexicalResults.TryGetValue(expression.Span, out var result))
            return new { kind = "result", id = result };
        if (model.GetOperation(expression) is { } operation && Value(operation) is { } value)
            return value;
        return expression switch
        {
            ThisExpressionSyntax => new { kind = "instance" },
            IdentifierNameSyntax name => model.GetSymbolInfo(name).Symbol switch
            {
                ILocalSymbol local => LocalValue(local),
                IParameterSymbol parameter => ParameterValue(parameter),
                _ => new { kind = "lexical", name = name.Identifier.ValueText },
            },
            MemberAccessExpressionSyntax access when AdoOperations.IsMember(access.Name.Identifier.ValueText) =>
                new { kind = "ado-member", name = access.Name.Identifier.ValueText,
                      owner = LexicalValue(access.Expression), typed = false },
            ElementAccessExpressionSyntax { ArgumentList.Arguments.Count: 1 } index =>
                new { kind = "ado-index", owner = LexicalValue(index.Expression),
                      selector = LexicalValue(index.ArgumentList.Arguments[0].Expression), typed = false },
            LiteralExpressionSyntax literal when literal.Token.Value is string or bool or int or long ||
                literal.IsKind(SyntaxKind.NullLiteralExpression) => new { kind = "literal", value = literal.Token.Value },
            ParenthesizedExpressionSyntax wrapped => LexicalValue(wrapped.Expression),
            CastExpressionSyntax cast => LexicalValue(cast.Expression),
            MemberAccessExpressionSyntax access when InferredDataMember(access) is { } field => field,
            MemberAccessExpressionSyntax access => new { kind = "lexical-member", owner = LexicalValue(access.Expression),
                name = access.Name.Identifier.ValueText },
            _ => null,
        };
    }

    internal IMethodSymbol? SourceCandidate(InvocationExpressionSyntax invocation, int depth = 0)
    {
        var name = CalleeName(invocation);
        if (name is null || model.GetOperation(invocation) is IDynamicInvocationOperation || model.GetDeclaredSymbol(method) is not IMethodSymbol owner) return null;
        var access = invocation.Expression as MemberAccessExpressionSyntax;
        var implicitReceiver = access is null && invocation.Expression is SimpleNameSyntax;
        if (!implicitReceiver && access is null) return null;
        var staticReceiver = access is not null && model.GetSymbolInfo(access.Expression).Symbol is INamedTypeSymbol;
        var type = access is null ? owner.ContainingType : NominalType(access.Expression, depth + 1) as INamedTypeSymbol;
        if (type is not { TypeKind: TypeKind.Class or TypeKind.Struct }) return null;

        // A missing argument dependency may prevent overload resolution even when
        // the receiver type is known. Bind only an accessible, unique nominal
        // member, with the call's receiver and explicit argument slots intact.
        var info = model.GetSymbolInfo(invocation);
        if (info.CandidateSymbols.Length > 1) return null;
        var lookup = info.CandidateSymbols.IsEmpty
            ? model.LookupSymbols(invocation.SpanStart, implicitReceiver ? null : type, name)
            : info.CandidateSymbols;
        var candidate = lookup.OfType<IMethodSymbol>()
            .Where(target => target.MethodKind == MethodKind.Ordinary &&
                (implicitReceiver ? !owner.IsStatic || target.IsStatic : target.IsStatic == staticReceiver) &&
                IsNominalMember(type, target.ContainingType))
            .Take(2).ToArray();
        if (candidate.Length != 1) return null;
        var target = candidate[0];
        if (target.TypeParameters.Length != 0 || target.IsVirtual || target.IsOverride || target.IsAbstract ||
            CandidateSlots(invocation.ArgumentList.Arguments, target) is null ||
            target.Parameters.Any(parameter => parameter.IsParams || parameter.RefKind != RefKind.None))
            return null;
        return target;
    }

    private static bool IsNominalMember(INamedTypeSymbol type, INamedTypeSymbol declaring)
    {
        for (INamedTypeSymbol? current = type; current is not null; current = current.BaseType)
            if (SymbolEqualityComparer.Default.Equals(current, declaring)) return true;
        return false;
    }

    private static int[]? CandidateSlots(SeparatedSyntaxList<ArgumentSyntax> arguments, IMethodSymbol method)
    {
        if (arguments.Count != method.Parameters.Length) return null;
        var selected = new int[arguments.Count];
        var seen = new HashSet<int>();
        var reordered = false;
        for (var index = 0; index < arguments.Count; index++)
        {
            var arg = arguments[index];
            if (arg.RefKindKeyword != default || reordered && arg.NameColon is null) return null;
            var slot = arg.NameColon is { } named ? method.Parameters.FirstOrDefault(parameter =>
                parameter.Name == named.Name.Identifier.ValueText)?.Ordinal ?? -1 : index;
            if (slot < 0 || !seen.Add(slot)) return null;
            selected[index] = slot;
            reordered |= slot != index;
        }
        return selected;
    }

    private ITypeSymbol? NominalType(ExpressionSyntax expression, int depth = 0)
    {
        var proven = model.GetTypeInfo(expression).Type;
        if (depth > 4 || proven is { TypeKind: not TypeKind.Error }) return proven;
        if (expression is IdentifierNameSyntax name && model.GetSymbolInfo(name).Symbol is ILocalSymbol local &&
            local.DeclaringSyntaxReferences.SingleOrDefault()?.GetSyntax() is VariableDeclaratorSyntax { Initializer.Value: { } init })
            return NominalType(init, depth + 1) ?? proven;
        if (expression is AwaitExpressionSyntax awaiting && NominalType(awaiting.Expression, depth + 1) is INamedTypeSymbol awaitable &&
            awaitable.ContainingNamespace.ToDisplayString() is "System.Threading.Tasks" or "System.Runtime.CompilerServices" &&
            awaitable.Name is "Task" or "ValueTask" or "ConfiguredTaskAwaitable" or "ConfiguredValueTaskAwaitable" &&
            awaitable.TypeArguments.Length == 1)
            return awaitable.TypeArguments[0];
        if (expression is AwaitExpressionSyntax { Expression: InvocationExpressionSyntax {
            Expression: MemberAccessExpressionSyntax { Name: GenericNameSyntax generic } query } } &&
            generic.TypeArgumentList.Arguments is [var rowType] && generic.Identifier.ValueText is
                ("QueryFirstAsync" or "QueryFirstOrDefaultAsync" or "QuerySingleAsync" or "QuerySingleOrDefaultAsync") &&
            RetainedSdkEffects.Dapper(model, query.Expression, generic.Identifier.ValueText,
                ManagedValues.ResourceFamily(NominalType(query.Expression, depth + 1))) &&
            model.GetTypeInfo(rowType).Type is { TypeKind: not TypeKind.Error } row)
            return row;
        if (expression is InvocationExpressionSyntax invocation)
        {
            var method = model.GetSymbolInfo(invocation).Symbol as IMethodSymbol ?? SourceCandidate(invocation, depth + 1);
            if (method is not null) return method.ReturnType;
        }
        return proven;
    }

    private object? InferredDataMember(MemberAccessExpressionSyntax member)
    {
        if (NominalType(member.Expression) is not INamedTypeSymbol { TypeKind: not TypeKind.Error } type) return null;
        var declarations = type.GetMembers(member.Name.Identifier.ValueText);
        if (declarations.Length != 1 || LexicalValue(member.Expression) is not { } owner) return null;
        var symbol = declarations[0];
        return symbol is IFieldSymbol { IsStatic: false } || symbol is IPropertySymbol { IsStatic: false } property && IsAutoProperty(property)
            ? new { kind = "member", id = symbol.OriginalDefinition.GetDocumentationCommentId(), name = symbol.Name,
                owner, direct = true, shared = false } : null;
    }

    private static string? CalleeName(InvocationExpressionSyntax invocation) => invocation.Expression switch
    {
        SimpleNameSyntax simple => simple.Identifier.ValueText,
        MemberAccessExpressionSyntax access => access.Name.Identifier.ValueText,
        _ => null,
    };

    private void UnresolvedCall(IOperation operation, InvocationExpressionSyntax invocation)
    {
        var access = invocation.Expression as MemberAccessExpressionSyntax;
        var name = CalleeName(invocation);
        // Dynamic invocation uses runtime dispatch, even if a declaration is visible.
        var candidate = operation is IDynamicInvocationOperation ? null : SourceCandidate(invocation);
        var receiver = candidate is { IsStatic: true } ? null : access is not null ? LexicalValue(access.Expression) :
            candidate is { IsStatic: false } ? new { kind = "instance" } : null;
        var selector = name == "FirstOrDefault" ? invocation.ArgumentList.Arguments.Select(argument => argument.Expression)
            .OfType<LambdaExpressionSyntax>().Take(2).ToArray() : [];
        var family = access is null ? null : ManagedValues.ResourceFamily(NominalType(access.Expression));
        var nominalRole = candidate is not null && ManagedValues.IsList(candidate.ContainingType) ?
            Role(candidate) : null;
        var effectFamily = operation is not IDynamicInvocationOperation && access is not null
            ? RetainedSdkEffects.Family(model, access.Expression, name, family) : null;
        var dapper = effectFamily == "database" && access is not null && RetainedSdkEffects.Dapper(model, access.Expression, name, family);
        var kafkaTopic = operation is not IDynamicInvocationOperation && candidate is null && access is not null &&
            RetainedSdkEffects.KafkaProducer(model, access.Expression, name)
            ? NominalKafkaTopic(invocation) : null;
        var asyncResourceApi = name is "BeginTransactionAsync" or "CommitAsync" or "RollbackAsync" or
            "ExecuteNonQueryAsync" or "ExecuteReaderAsync" or "ExecuteScalarAsync" or "ReadAsync" or "DisposeAsync";
        Add(operation, "CALL", ("symbol", null), ("spelling", name), ("receiver", receiver),
            ("arguments", SyntaxArguments(invocation.ArgumentList.Arguments, candidate)), ("binding", null),
            ("candidate", candidate is null ? null : BoundTarget(candidate, false)),
            ("receiverFamily", family),
            ("candidateEffectFamily", effectFamily),
            ("nominalPublication", candidate is null && RetainedSdkEffects.Publication(name, effectFamily)),
            ("kafkaTopicPosition", kafkaTopic),
            ("dapperCandidate", candidate is null && dapper),
            ("resultFamily", ManagedValues.ResourceFamily(candidate?.ReturnType)),
            ("predicateSite", selector.Length == 1 ? Site(selector[0]) : null),
            ("role", nominalRole), ("nominalFramework", nominalRole is not null),
            ("dynamic", operation is IDynamicInvocationOperation),
            ("dispatch", "OPEN"), ("inSource", false), ("instanceCall", candidate is { IsStatic: false }), ("isAsync", candidate?.IsAsync ?? false),
            ("taskLike", candidate is not null ? ReturnsTask(candidate) : asyncResourceApi ||
                (dapper || RetainedSdkEffects.Publication(name, effectFamily)) && name!.EndsWith("Async", StringComparison.Ordinal)));
    }

    private static bool IsUnboundCancellationParameter(IParameterSymbol parameter) =>
        parameter.RefKind == RefKind.None && parameter.Type.TypeKind == TypeKind.Error &&
        parameter.DeclaringSyntaxReferences.FirstOrDefault()?.GetSyntax() is
            ParameterSyntax { Type: IdentifierNameSyntax { Identifier.ValueText: "CancellationToken" } };

    private int? UnboundTokenInput(ExpressionSyntax? expression)
    {
        while (expression is ParenthesizedExpressionSyntax parentheses) expression = parentheses.Expression;
        if (expression is IdentifierNameSyntax &&
            model.GetSymbolInfo(expression).Symbol is IParameterSymbol parameter &&
            SymbolEqualityComparer.Default.Equals(parameter.ContainingSymbol, model.GetDeclaredSymbol(method)) &&
            IsUnboundCancellationParameter(parameter))
            return parameter.Ordinal;
        return null;
    }

    private static int? NominalKafkaTopic(InvocationExpressionSyntax invocation)
    {
        var arguments = invocation.ArgumentList.Arguments;
        if (arguments.Count < 2 || arguments.Any(argument => argument.RefKindKeyword != default))
            return null;
        var named = arguments.Select((argument, index) => (argument, index))
            .Where(row => row.argument.NameColon?.Name.Identifier.ValueText == "topic").ToArray();
        if (named.Length == 1)
            return named[0].index == 0 || arguments[0].NameColon is not null ? named[0].index : null;
        return named.Length == 0 && arguments[0].NameColon is null ? 0 : null;
    }

    private object SyntaxArguments(SeparatedSyntaxList<ArgumentSyntax> arguments, IMethodSymbol? candidate = null)
    {
        var slots = candidate is null ? null : CandidateSlots(arguments, candidate);
        return arguments.Select((argument, ordinal) => new
        {
            id = slots?[ordinal] ?? ordinal, name = slots is null ? argument.NameColon?.Name.Identifier.ValueText : candidate!.Parameters[slots[ordinal]].Name,
            value = LexicalValue(argument.Expression), byRef = argument.RefKindKeyword != default,
            cancellationToken = AdoOperations.IsCancellationToken(model.GetTypeInfo(argument.Expression).Type),
            unboundTokenInput = UnboundTokenInput(argument.Expression),
        }).ToArray();
    }

    private object OperationArguments(IEnumerable<IArgumentOperation> arguments) => arguments.Select(argument => new
    {
        id = argument.Parameter?.Ordinal, name = argument.Parameter?.Name,
        value = OperationValue(argument.Value), byRef = argument.Parameter?.RefKind is not (null or RefKind.None),
        cancellationToken = AdoOperations.IsCancellationToken(argument.Parameter?.Type),
        unboundTokenInput = argument.IsImplicit ? null : UnboundTokenInput((argument.Syntax as ArgumentSyntax)?.Expression),
    }).ToArray();

    private object? OperationValue(IOperation operation) =>
        Value(operation) ?? (operation.Syntax is ExpressionSyntax expression ? LexicalValue(expression) : null);

    private static (string Name, BaseObjectCreationExpressionSyntax Creation)? InitializerMember(IOperation operation) =>
        operation.Syntax is AssignmentExpressionSyntax
        {
            Left: IdentifierNameSyntax name,
            Parent: InitializerExpressionSyntax { Parent: BaseObjectCreationExpressionSyntax creation },
        } ? (name.Identifier.ValueText, creation) : null;

    private void Assign(IOperation operation, ISimpleAssignmentOperation assignment)
    {
        var target = OperationValue(assignment.Target);
        var initializer = InitializerMember(operation);
        if (assignment.Target is IInvalidOperation && UnresolvedInitializerTarget(initializer) is { } unresolved)
            target = unresolved;
        Add(operation, "WRITE", ("target", target), ("value", OperationValue(assignment.Value)),
            ("initializerMember", initializer?.Name));
    }

    private object? UnresolvedInitializerTarget((string Name, BaseObjectCreationExpressionSyntax Creation)? initializer)
    {
        if (initializer is not { } member || !lexicalResults.TryGetValue(member.Creation.Span, out var allocation))
            return null;
        var family = ManagedValues.ResourceFamily(model.GetTypeInfo(member.Creation).Type);
        var kind = AdoOperations.IsMember(member.Name) && (member.Name is not ("Value" or "ParameterName") || family == "parameter")
            ? "ado-member" : "object-initializer-member";
        return new { kind, name = member.Name, owner = new { kind = "result", id = allocation }, typed = false };
    }

    private void Visit(IOperation operation, int depth)
    {
        if (steps.Count > 160) return;
        if (depth > 32)
        {
            Add(operation, "OPEN", ("reason", "OPERATION_DEPTH"));
            return;
        }
        if (operation is IAnonymousFunctionOperation or IFlowAnonymousFunctionOperation or ILocalFunctionOperation)
            return;
        foreach (var child in operation.ChildOperations) Visit(child, depth + 1);
        switch (operation)
        {
            case IInvocationOperation invocation:
                var target = invocation.TargetMethod;
                var role = Role(target);
                var factory = ManagedValues.ValueFactory(target);
                var selector = invocation.Arguments.FirstOrDefault(argument => argument.Parameter?.Name is "predicate" or "selector");
                var lambda = selector?.Syntax is ArgumentSyntax { Expression: LambdaExpressionSyntax source } ? source : null;
                Add(operation, "CALL", ("symbol", MethodId(target)), ("spelling", target.Name), ("receiver", invocation.Instance is { } receiver ? OperationValue(receiver) : null),
                    ("arguments", OperationArguments(invocation.Arguments)),
                    ("instanceCall", invocation.Instance is not null && target.ReducedFrom is null && !target.IsStatic &&
                        target.MethodKind != MethodKind.DelegateInvoke),
                    ("binding", BoundTarget(target, invocation.IsVirtual)), ("role", role),
                    ("inventoryTarget", invocation.IsVirtual ? BoundTarget(target, false) : null),
                    ("candidateEffectFamily", operation.Syntax is InvocationExpressionSyntax { Expression: MemberAccessExpressionSyntax access } ?
                        RetainedSdkEffects.Family(model, access.Expression, target.Name, ManagedValues.ResourceFamily(invocation.Instance?.Type)) : null),
                    ("directEffectFamily", DirectEffectFamily(target, role)),
                    ("kafkaTopicPosition", RetainedSdkEffects.SelectedKafkaTopic(target)),
                    ("httpResult", ManagedValues.HttpResult(target, invocation.Arguments)),
                    ("responseConstruction", ManagedValues.IsResponseConstruction(target)),
                    ("receiverFamily", ManagedValues.ResourceFamily(invocation.Instance?.Type)),
                    ("resultFamily", ManagedValues.ResourceFamily(target.ReturnType)), ("implicit", invocation.IsImplicit),
                    ("valueFactory", factory), ("nullDefault", factory == "LINQ_FIRST_OR_DEFAULT" && ManagedValues.NullDefault(target.ReturnType)),
                    ("predicateSite", lambda is not null ? Site(lambda) : null),
                    ("selector", Selector(lambda)),
                    ("hasSelector", selector is not null),
                    ("dispatch", invocation.IsVirtual || target.MethodKind == MethodKind.DelegateInvoke ? "OPEN" : "STATIC_SELECTION"),
                    ("inSource", target.Locations.Any(location => location.IsInSource)),
                    ("isAsync", target.IsAsync), ("taskLike", ReturnsTask(target)));
                break;
            case IAwaitOperation awaitOperation:
                Add(operation, "AWAIT", ("value", OperationValue(awaitOperation.Operation)));
                break;
            case ISimpleAssignmentOperation when operation.Parent is IAnonymousObjectCreationOperation:
                break;
            case ISimpleAssignmentOperation assignment:
                Assign(operation, assignment);
                break;
            case ICompoundAssignmentOperation assignment:
                Add(operation, "WRITE", ("target", Value(assignment.Target)), ("value", null));
                break;
            case IIncrementOrDecrementOperation increment:
                Add(operation, "WRITE", ("target", Value(increment.Target)), ("value", null));
                break;
            case IFlowCaptureOperation capture:
                Add(operation, "BIND", ("capture", Capture(capture.Id)), ("value", OperationValue(capture.Value)));
                break;
            case IObjectCreationOperation creation:
                Add(operation, "CONSTRUCT", ("symbol", creation.Constructor is { } ctor ? MethodId(ctor) : null),
                    ("binding", creation.Constructor is { } bound ? BoundTarget(bound, false) : null),
                    ("directEffectFamily", creation.Constructor is { } selected ? DirectEffectFamily(selected, null) : null),
                    ("candidateEffectFamily", creation.Syntax is BaseObjectCreationExpressionSyntax sourceCreation ?
                        RetainedSdkEffects.Construction(model, sourceCreation) : null),
                    ("implicit", creation.IsImplicit),
                    ("arguments", OperationArguments(creation.Arguments)), ("command", AdoOperations.IsType(creation.Type, "DbCommand")),
                    ("httpResult", creation.Constructor is { } constructor ? ManagedValues.HttpResult(constructor, creation.Arguments) : null),
                    ("responseConstruction", creation.Constructor is { } result && ManagedValues.IsResponseConstruction(result)),
                    ("objectType", creation.Type is { TypeKind: not TypeKind.Error } type ? type.ToDisplayString() : null),
                    ("resourceFamily", ManagedValues.ResourceFamily(creation.Type)),
                    ("emptyList", ManagedValues.IsList(creation.Type) && creation.Constructor is { } listConstructor &&
                        listConstructor.Parameters.All(parameter => parameter.Name == "capacity")),
                    ("recordFields", ManagedValues.RecordFields(creation.Constructor)),
                    ("unresolved", false), ("parameter", AdoOperations.IsType(creation.Type, "DbParameter")));
                break;
            case IInvalidOperation when operation.Syntax is BaseObjectCreationExpressionSyntax creation:
                Add(operation, "CONSTRUCT", ("symbol", null), ("arguments", SyntaxArguments(creation.ArgumentList?.Arguments ?? default)),
                    ("candidateEffectFamily", RetainedSdkEffects.Construction(model, creation)),
                    ("resourceFamily", ManagedValues.ResourceFamily(model.GetTypeInfo(creation).Type)),
                    ("objectType", creation is ObjectCreationExpressionSyntax named && named.Type.Span.Length <= 100 ? named.Type.ToString() : null),
                    ("command", false), ("unresolved", true), ("parameter", false));
                break;
            case IInvalidOperation when operation.Syntax is InvocationExpressionSyntax invocation:
                UnresolvedCall(operation, invocation);
                break;
            case IDynamicInvocationOperation when operation.Syntax is InvocationExpressionSyntax invocation:
                UnresolvedCall(operation, invocation);
                break;
            case IPropertyReferenceOperation property when ManagedValues.JsonProperty(property.Property) is not null ||
                AdoOperations.Property(property.Property) == "Item" &&
                AdoOperations.IsType(property.Property.ContainingType, "DbDataReader"):
                Add(operation, "ACCESS", ("value", Value(property)));
                break;
            case IPropertyReferenceOperation property when property.Parent is ISimpleAssignmentOperation assignment &&
                assignment.Parent is IAnonymousObjectCreationOperation && assignment.Target == property:
                break;
            case IPropertyReferenceOperation property when AdoOperations.Property(property.Property) is null &&
                NullableProperty(property.Property) is null && !IsAutoProperty(property.Property):
                Add(operation, "OPEN", ("reason", "PROPERTY_ACCESSOR_COMPLETION_OPEN"));
                break;
            case IInvalidOperation when operation.Syntax is MemberAccessExpressionSyntax member &&
                (AdoOperations.IsMember(member.Name.Identifier.ValueText) || InferredDataMember(member) is not null):
                break;
            case IInvalidOperation when operation.Syntax is ElementAccessExpressionSyntax { ArgumentList.Arguments.Count: 1 } index:
                Add(operation, "ACCESS", ("value", LexicalValue(index)));
                break;
            case IInvalidOperation when operation.Syntax is AssignmentExpressionSyntax assign &&
                assign.Left is MemberAccessExpressionSyntax targetMember && AdoOperations.IsMember(targetMember.Name.Identifier.ValueText):
                Add(operation, "WRITE", ("target", LexicalValue(assign.Left)), ("value", LexicalValue(assign.Right)));
                break;
            case IInvalidOperation when operation.Syntax is AssignmentExpressionSyntax assign &&
                UnresolvedInitializerTarget(InitializerMember(operation)) is { } unresolved:
                Add(operation, "WRITE", ("target", unresolved), ("value", LexicalValue(assign.Right)));
                break;
            case IInvalidOperation when operation.ChildOperations.Any(child => child.Syntax == operation.Syntax):
                break;
            case IInvalidOperation:
                Add(operation, "OPEN", ("reason", "UNRESOLVED_OPERATION"));
                break;
        }
    }
}
