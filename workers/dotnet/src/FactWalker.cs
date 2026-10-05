using System.Text.Json.Nodes;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;
using Microsoft.CodeAnalysis.Operations;

namespace Codeclew.CSharp.Analyzer;

/// <summary>
/// Emits declarations, compiler relations and documentation flow for one source
/// document. Generated documents contribute declarations only.
/// </summary>
public sealed class FactWalker : CSharpSyntaxWalker
{
    private readonly SemanticModel _model;
    private readonly SourceAnchors _anchors;
    private readonly FactWriter _writer;
    private readonly string _project;
    private readonly bool _generated;
    private readonly HashSet<string> _emittedTypes;

    public FactWalker(SemanticModel model, SourceAnchors anchors, FactWriter writer, string project, bool generated, HashSet<string> emittedTypes)
    {
        _model = model;
        _anchors = anchors;
        _writer = writer;
        _project = project;
        _generated = generated;
        _emittedTypes = emittedTypes;
    }

    public override void VisitClassDeclaration(ClassDeclarationSyntax node) => TypeDeclaration(node, () => base.VisitClassDeclaration(node));
    public override void VisitStructDeclaration(StructDeclarationSyntax node) => TypeDeclaration(node, () => base.VisitStructDeclaration(node));
    public override void VisitInterfaceDeclaration(InterfaceDeclarationSyntax node) => TypeDeclaration(node, () => base.VisitInterfaceDeclaration(node));
    public override void VisitRecordDeclaration(RecordDeclarationSyntax node) => TypeDeclaration(node, () => base.VisitRecordDeclaration(node));
    public override void VisitEnumDeclaration(EnumDeclarationSyntax node) => TypeDeclaration(node, () => base.VisitEnumDeclaration(node));

    public override void VisitDelegateDeclaration(DelegateDeclarationSyntax node)
    {
        if (_model.GetDeclaredSymbol(node) is INamedTypeSymbol type)
        {
            var row = Declaration("DELEGATE", Identities.Class(type), type, node.Identifier.Span, node);
            row["qualifiedName"] = Identities.BinaryName(type).Replace('$', '.');
            _writer.Write(row);
        }
    }

    private void TypeDeclaration(BaseTypeDeclarationSyntax node, Action visitMembers)
    {
        if (_model.GetDeclaredSymbol(node) is not INamedTypeSymbol type)
        {
            _writer.Boundary("CSHARP_TYPE_SYMBOL_UNRESOLVED", _anchors, node.Identifier.Span);
            visitMembers();
            return;
        }
        var identity = Identities.Class(type);
        // Partial types are declared once, at their first part in path/position order.
        if (_emittedTypes.Add(identity))
        {
            var kind = type.TypeKind switch
            {
                TypeKind.Interface => "INTERFACE",
                TypeKind.Enum => "ENUM",
                TypeKind.Struct => type.IsRecord ? "RECORD" : "STRUCT",
                _ => type.IsRecord ? "RECORD" : "CLASS",
            };
            var row = Declaration(kind, identity, type, node.Span, node);
            row["qualifiedName"] = Identities.BinaryName(type).Replace('$', '.');
            row["interfaces"] = Strings(type.Interfaces.Select(Identities.Class));
            if (type.BaseType is { } baseType && type.TypeKind == TypeKind.Class
                && baseType.SpecialType != SpecialType.System_Object)
            {
                row["superclass"] = Identities.Class(baseType);
            }
            row["attributes"] = AttributeFacts.Uses(type);
            if (type.DeclaringSyntaxReferences.Length > 1)
            {
                row["partialParts"] = type.DeclaringSyntaxReferences.Length;
            }
            _writer.Write(row);
        }
        visitMembers();
    }

    public override void VisitMethodDeclaration(MethodDeclarationSyntax node)
    {
        if (_model.GetDeclaredSymbol(node) is not IMethodSymbol method)
        {
            _writer.Boundary("CSHARP_METHOD_SYMBOL_UNRESOLVED", _anchors, node.Identifier.Span);
            return;
        }
        // A partial method's definition part without a body is represented by its implementation.
        if (method.PartialImplementationPart is not null)
        {
            return;
        }
        Callable("METHOD", method, node, (SyntaxNode?)node.Body ?? node.ExpressionBody);
    }

    public override void VisitConstructorDeclaration(ConstructorDeclarationSyntax node)
    {
        if (_model.GetDeclaredSymbol(node) is IMethodSymbol method)
        {
            Callable("CONSTRUCTOR", method, node, (SyntaxNode?)node.Body ?? node.ExpressionBody, node.Initializer);
        }
    }

    public override void VisitOperatorDeclaration(OperatorDeclarationSyntax node)
    {
        if (_model.GetDeclaredSymbol(node) is IMethodSymbol method)
        {
            Callable("METHOD", method, node, (SyntaxNode?)node.Body ?? node.ExpressionBody);
        }
    }

    public override void VisitConversionOperatorDeclaration(ConversionOperatorDeclarationSyntax node)
    {
        if (_model.GetDeclaredSymbol(node) is IMethodSymbol method)
        {
            Callable("METHOD", method, node, (SyntaxNode?)node.Body ?? node.ExpressionBody);
        }
    }

    public override void VisitDestructorDeclaration(DestructorDeclarationSyntax node)
    {
        if (_model.GetDeclaredSymbol(node) is IMethodSymbol method)
        {
            Callable("METHOD", method, node, (SyntaxNode?)node.Body ?? node.ExpressionBody);
        }
    }

    public override void VisitPropertyDeclaration(PropertyDeclarationSyntax node)
    {
        if (_model.GetDeclaredSymbol(node) is IPropertySymbol property)
        {
            Property(property, node, node.ExpressionBody, node.AccessorList, node.Initializer?.Value);
        }
    }

    public override void VisitIndexerDeclaration(IndexerDeclarationSyntax node)
    {
        if (_model.GetDeclaredSymbol(node) is IPropertySymbol property)
        {
            Property(property, node, node.ExpressionBody, node.AccessorList, null);
        }
    }

    public override void VisitEventDeclaration(EventDeclarationSyntax node)
    {
        if (_model.GetDeclaredSymbol(node) is IEventSymbol @event)
        {
            var identity = Identities.Event(@event);
            _writer.Write(Declaration("EVENT", identity, @event, node.Span, node));
            if (node.AccessorList is { } accessors)
            {
                Relations(identity, accessors);
            }
        }
    }

    public override void VisitEventFieldDeclaration(EventFieldDeclarationSyntax node)
    {
        foreach (var variable in node.Declaration.Variables)
        {
            if (_model.GetDeclaredSymbol(variable) is IEventSymbol @event)
            {
                _writer.Write(Declaration("EVENT", Identities.Event(@event), @event, variable.Span, node));
            }
        }
    }

    public override void VisitFieldDeclaration(FieldDeclarationSyntax node)
    {
        foreach (var variable in node.Declaration.Variables)
        {
            if (_model.GetDeclaredSymbol(variable) is not IFieldSymbol field)
            {
                continue;
            }
            var identity = Identities.Field(field);
            var row = Declaration("FIELD", identity, field, variable.Span, node);
            row["jvmDescriptor"] = Identities.Descriptor(field.Type);
            row["type"] = Identities.TypeIdentity(field.Type) ?? field.Type.ToDisplayString();
            _writer.Write(row);
            if (variable.Initializer is { } initializer)
            {
                Relations(identity, initializer.Value);
            }
        }
    }

    public override void VisitEnumMemberDeclaration(EnumMemberDeclarationSyntax node)
    {
        if (_model.GetDeclaredSymbol(node) is IFieldSymbol field)
        {
            var row = Declaration("FIELD", Identities.Field(field), field, node.Span, node);
            row["jvmDescriptor"] = Identities.Descriptor(field.Type);
            _writer.Write(row);
        }
    }

    // Bodies are handled by their declarations; local functions and lambdas belong to them.
    public override void VisitLocalFunctionStatement(LocalFunctionStatementSyntax node) { }
    public override void VisitBlock(BlockSyntax node) { }
    public override void VisitArrowExpressionClause(ArrowExpressionClauseSyntax node) { }

    private void Callable(string kind, IMethodSymbol method, SyntaxNode node, SyntaxNode? body, SyntaxNode? initializer = null)
    {
        var identity = Identities.Method(method);
        var row = Declaration(kind, identity, method, node.Span, node);
        row["jvmDescriptor"] = Identities.MethodDescriptor(Identities.Normalize(method));
        row["signature"] = method.ToDisplayString(SymbolDisplayFormat.CSharpShortErrorMessageFormat);
        row["returnType"] = Identities.TypeIdentity(method.ReturnType) ?? method.ReturnType.ToDisplayString();
        row["parameters"] = new JsonArray(method.Parameters.Select(parameter => (JsonNode?)new JsonObject
        {
            ["name"] = parameter.Name,
            ["type"] = parameter.Type.ToDisplayString(),
            ["typeIdentity"] = Identities.TypeIdentity(parameter.Type),
            ["refKind"] = parameter.RefKind == RefKind.None ? null : parameter.RefKind.ToString().ToUpperInvariant(),
            ["attributes"] = AttributeFacts.Names(parameter),
        }).ToArray());
        row["signatureTypes"] = Strings(SignatureTypes(method));
        row["attributes"] = AttributeFacts.Uses(method);
        if (method.OverriddenMethod is { } overridden)
        {
            row["overrides"] = Identities.Method(overridden);
        }
        var implemented = Implemented(method).ToList();
        if (implemented.Count > 0)
        {
            row["implements"] = Strings(implemented);
        }
        if (AttributeFacts.ForMethod(method, identity) is { } clr && HasAnyAttribute(method))
        {
            row["clrAttributes"] = clr;
        }
        if (!_generated && (body is not null || initializer is not null))
        {
            var flowBody = body ?? initializer!;
            row["documentation"] = DocumentationFlow.Read(_model, _anchors, method, flowBody);
        }
        _writer.Write(row);
        if (_generated)
        {
            return;
        }
        if (initializer is not null)
        {
            Relations(identity, initializer);
        }
        if (body is not null)
        {
            Relations(identity, body);
        }
    }

    private void Property(IPropertySymbol property, BasePropertyDeclarationSyntax node, ArrowExpressionClauseSyntax? expressionBody, AccessorListSyntax? accessors, ExpressionSyntax? initializer)
    {
        var identity = Identities.Property(property);
        var row = Declaration("PROPERTY", identity, property, node.Span, node);
        row["jvmDescriptor"] = Identities.Descriptor(property.Type);
        row["type"] = Identities.TypeIdentity(property.Type) ?? property.Type.ToDisplayString();
        row["attributes"] = AttributeFacts.Uses(property);
        if (property.OverriddenProperty is { } overridden)
        {
            row["overrides"] = Identities.Property(overridden);
        }
        _writer.Write(row);
        if (_generated)
        {
            return;
        }
        if (expressionBody is not null)
        {
            Relations(identity, expressionBody);
        }
        if (accessors is not null)
        {
            Relations(identity, accessors);
        }
        if (initializer is not null)
        {
            Relations(identity, initializer);
        }
    }

    private JsonObject Declaration(string kind, string identity, ISymbol symbol, Microsoft.CodeAnalysis.Text.TextSpan span, SyntaxNode node)
    {
        var row = _writer.Row("DECLARATION");
        row["declarationKind"] = kind;
        row["name"] = symbol is IMethodSymbol { MethodKind: MethodKind.Constructor or MethodKind.StaticConstructor } ? "<init>" : symbol.Name;
        row["symbolIdentity"] = identity;
        row["ownerIdentity"] = Identities.Owner(symbol);
        row["csharpIdentity"] = Identities.DocumentationId(symbol);
        row["accessibility"] = AttributeFacts.Accessibility(symbol.DeclaredAccessibility);
        row["modifiers"] = Strings(Modifiers(symbol, node));
        row["annotations"] = AttributeFacts.Names(symbol);
        row["project"] = _project;
        if (_generated)
        {
            row["generated"] = true;
        }
        _anchors.Apply(row, span);
        row["resolution"] = "COMPILER_EXACT";
        return row;
    }

    private void Relations(string source, SyntaxNode body)
    {
        if (_generated)
        {
            return;
        }
        foreach (var root in OperationRoots(body))
        {
            foreach (var operation in root.DescendantsAndSelf())
            {
                switch (operation)
                {
                    case IInvocationOperation invocation:
                        Relation("CALLS", source, invocation.TargetMethod, invocation.Syntax);
                        break;
                    case IObjectCreationOperation { Constructor: { } constructor } creation:
                        Relation("CONSTRUCTS", source, constructor, creation.Syntax);
                        break;
                    case IMethodReferenceOperation reference:
                        Relation("REFERENCES", source, reference.Method, reference.Syntax);
                        break;
                    case IInvalidOperation invalid when invalid.Syntax is InvocationExpressionSyntax or ObjectCreationExpressionSyntax:
                        _writer.Boundary("CSHARP_CALL_TARGET_UNRESOLVED", _anchors, invalid.Syntax.Span);
                        break;
                }
            }
        }
    }

    private IEnumerable<IOperation> OperationRoots(SyntaxNode body)
    {
        if (_model.GetOperation(body) is { } operation)
        {
            yield return operation;
            yield break;
        }
        // Accessor lists and other containers have no operation of their own.
        foreach (var child in body.ChildNodes())
        {
            foreach (var nested in OperationRoots(child))
            {
                yield return nested;
            }
        }
    }

    private void Relation(string kind, string source, IMethodSymbol target, SyntaxNode syntax)
    {
        var row = _writer.Row("RELATION");
        row["relationKind"] = kind;
        row["sourceIdentity"] = source;
        row["targetIdentity"] = Identities.Method(target);
        row["targetCsharpIdentity"] = Identities.DocumentationId(Identities.Normalize(target));
        _anchors.Apply(row, syntax.Span);
        row["resolution"] = "COMPILER_EXACT";
        _writer.Write(row);
    }

    private static bool HasAnyAttribute(IMethodSymbol method)
    {
        if (method.GetAttributes().Length > 0)
        {
            return true;
        }
        for (var type = method.ContainingType; type is not null; type = type.BaseType)
        {
            if (type.GetAttributes().Length > 0)
            {
                return true;
            }
        }
        return false;
    }

    private static IEnumerable<string> Implemented(IMethodSymbol method)
    {
        if (method.ContainingType is not { } owner || method.MethodKind != MethodKind.Ordinary && method.MethodKind != MethodKind.ExplicitInterfaceImplementation)
        {
            yield break;
        }
        foreach (var @interface in owner.AllInterfaces)
        {
            foreach (var member in @interface.GetMembers().OfType<IMethodSymbol>())
            {
                if (SymbolEqualityComparer.Default.Equals(owner.FindImplementationForInterfaceMember(member), method))
                {
                    yield return Identities.Method(member);
                }
            }
        }
    }

    private static IEnumerable<string> SignatureTypes(IMethodSymbol method)
    {
        var result = new SortedSet<string>(StringComparer.Ordinal);
        void Add(ITypeSymbol type, int depth)
        {
            if (depth > 6)
            {
                return;
            }
            switch (type)
            {
                case IArrayTypeSymbol array:
                    Add(array.ElementType, depth + 1);
                    break;
                case INamedTypeSymbol named:
                    if (named.SpecialType == SpecialType.None)
                    {
                        result.Add(Identities.Class(named));
                    }
                    foreach (var argument in named.TypeArguments)
                    {
                        Add(argument, depth + 1);
                    }
                    break;
            }
        }
        Add(method.ReturnType, 0);
        foreach (var parameter in method.Parameters)
        {
            Add(parameter.Type, 0);
        }
        return result;
    }

    private static IEnumerable<string> Modifiers(ISymbol symbol, SyntaxNode node)
    {
        var result = new SortedSet<string>(StringComparer.Ordinal);
        switch (symbol.DeclaredAccessibility)
        {
            case Accessibility.Public: result.Add("PUBLIC"); break;
            case Accessibility.Private: result.Add("PRIVATE"); break;
            case Accessibility.Protected: result.Add("PROTECTED"); break;
            case Accessibility.Internal: result.Add("INTERNAL"); break;
            case Accessibility.ProtectedOrInternal: result.Add("PROTECTED"); result.Add("INTERNAL"); break;
            case Accessibility.ProtectedAndInternal: result.Add("PRIVATE"); result.Add("PROTECTED"); break;
        }
        if (symbol.IsStatic) result.Add("STATIC");
        if (symbol.IsAbstract) result.Add("ABSTRACT");
        if (symbol.IsSealed) result.Add("SEALED");
        if (symbol.IsVirtual) result.Add("VIRTUAL");
        if (symbol.IsOverride) result.Add("OVERRIDE");
        if (symbol is IMethodSymbol { IsAsync: true }) result.Add("ASYNC");
        if (symbol is IFieldSymbol { IsReadOnly: true }) result.Add("READONLY");
        if (symbol is IFieldSymbol { IsConst: true }) result.Add("CONST");
        if (node is MemberDeclarationSyntax member && member.Modifiers.Any(SyntaxKind.PartialKeyword)) result.Add("PARTIAL");
        return result;
    }

    private static JsonArray Strings(IEnumerable<string> values) =>
        new(values.Select(value => (JsonNode?)JsonValue.Create(value)).ToArray());
}
