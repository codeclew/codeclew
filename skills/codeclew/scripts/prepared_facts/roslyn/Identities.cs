using System.Security.Cryptography;
using System.Text;
using Microsoft.CodeAnalysis;

namespace Codeclew.CSharp.Analyzer;

/// <summary>
/// Portable identities in the grammar shared with the JVM analyzers:
/// <c>class:Ns.Outer$Inner`1</c>, <c>method:class:Ns.Type#Name(desc)ret</c>,
/// <c>field:class:Ns.Type#name:desc</c>. CLR signatures are erased into JVM
/// descriptor grammar; members whose erased signatures collide are disambiguated
/// with a digest of their Roslyn documentation-comment ID.
/// </summary>
public static class Identities
{
    public static string Class(INamedTypeSymbol type) => "class:" + BinaryName(type);

    public static string Owner(ISymbol symbol) =>
        symbol.ContainingType is { } owner ? Class(owner) : "module:unnamed";

    public static string? TypeIdentity(ITypeSymbol? type) => type switch
    {
        null => null,
        INamedTypeSymbol named => Class(named.OriginalDefinition),
        IArrayTypeSymbol array => TypeIdentity(array.ElementType),
        _ => null,
    };

    public static string BinaryName(INamedTypeSymbol type)
    {
        type = type.OriginalDefinition;
        if (type.ContainingType is { } outer)
        {
            return BinaryName(outer) + "$" + type.MetadataName;
        }
        var ns = type.ContainingNamespace;
        return ns is null || ns.IsGlobalNamespace
            ? type.MetadataName
            : ns.ToDisplayString() + "." + type.MetadataName;
    }

    public static string Descriptor(ITypeSymbol? type)
    {
        switch (type)
        {
            case null:
                return "Ljava/lang/Object;";
            case IArrayTypeSymbol array:
                return new string('[', array.Rank) + Descriptor(array.ElementType);
            case IPointerTypeSymbol pointer:
                return "[" + Descriptor(pointer.PointedAtType);
            case ITypeParameterSymbol:
            case IDynamicTypeSymbol:
                return "LSystem/Object;";
        }
        switch (type.SpecialType)
        {
            case SpecialType.System_Void: return "V";
            case SpecialType.System_Boolean: return "Z";
            case SpecialType.System_SByte: return "B";
            case SpecialType.System_Int16: return "S";
            case SpecialType.System_Int32: return "I";
            case SpecialType.System_Int64: return "J";
            case SpecialType.System_Char: return "C";
            case SpecialType.System_Single: return "F";
            case SpecialType.System_Double: return "D";
        }
        if (type is INamedTypeSymbol named)
        {
            return "L" + BinaryName(named).Replace('.', '/') + ";";
        }
        return "LSystem/Object;";
    }

    public static string MethodName(IMethodSymbol method) => method.MethodKind switch
    {
        MethodKind.Constructor => "<init>",
        MethodKind.StaticConstructor => "<clinit>",
        _ => method.MetadataName,
    };

    public static string MethodDescriptor(IMethodSymbol method)
    {
        var value = new StringBuilder("(");
        foreach (var parameter in method.Parameters)
        {
            value.Append(Descriptor(parameter.Type));
        }
        value.Append(')');
        value.Append(method.MethodKind is MethodKind.Constructor or MethodKind.StaticConstructor
            ? "V"
            : Descriptor(method.ReturnType));
        return value.ToString();
    }

    public static string Method(IMethodSymbol method)
    {
        method = Normalize(method);
        var descriptor = MethodDescriptor(method);
        var identity = "method:" + Owner(method) + "#" + MethodName(method) + descriptor;
        return Collides(method, descriptor) ? identity + "@" + ShortDigest(method) : identity;
    }

    public static string Field(IFieldSymbol field) =>
        "field:" + Owner(field) + "#" + field.Name + ":" + Descriptor(field.Type);

    public static string Property(IPropertySymbol property)
    {
        property = property.OriginalDefinition;
        var descriptor = new StringBuilder();
        if (property.Parameters.Length > 0)
        {
            descriptor.Append('(');
            foreach (var parameter in property.Parameters)
            {
                descriptor.Append(Descriptor(parameter.Type));
            }
            descriptor.Append(')');
        }
        descriptor.Append(Descriptor(property.Type));
        return "property:" + Owner(property) + "#" + property.MetadataName + ":" + descriptor;
    }

    public static string Event(IEventSymbol @event) =>
        "event:" + Owner(@event) + "#" + @event.Name + ":" + Descriptor(@event.Type);

    public static string? Symbol(ISymbol? symbol) => symbol switch
    {
        IMethodSymbol method => Method(method),
        INamedTypeSymbol type => Class(type),
        IFieldSymbol field => Field(field.OriginalDefinition),
        IPropertySymbol property => Property(property),
        IEventSymbol @event => Event(@event.OriginalDefinition),
        _ => null,
    };

    public static string DocumentationId(ISymbol symbol) =>
        "csharp:" + (symbol.OriginalDefinition.GetDocumentationCommentId() ?? symbol.ToDisplayString());

    public static IMethodSymbol Normalize(IMethodSymbol method)
    {
        method = method.ReducedFrom ?? method;
        method = method.OriginalDefinition;
        return method.PartialImplementationPart ?? method;
    }

    /// <summary>Parameter type names derived from the erased descriptor, as the JVM flows report them.</summary>
    public static IReadOnlyList<string> ParameterTypes(IMethodSymbol method) =>
        method.Parameters.Select(parameter => ErasedName(Descriptor(parameter.Type))).ToList();

    private static string ErasedName(string descriptor)
    {
        var dimensions = descriptor.TakeWhile(c => c == '[').Count();
        var element = descriptor[dimensions..];
        var name = element switch
        {
            "Z" => "boolean",
            "B" => "byte",
            "S" => "short",
            "I" => "int",
            "J" => "long",
            "C" => "char",
            "F" => "float",
            "D" => "double",
            "V" => "void",
            _ => element.TrimStart('L').TrimEnd(';').Replace('/', '.').Replace('$', '.'),
        };
        return name + string.Concat(Enumerable.Repeat("[]", dimensions));
    }

    private static bool Collides(IMethodSymbol method, string descriptor)
    {
        if (method.ContainingType is null)
        {
            return false;
        }
        var name = MethodName(method);
        foreach (var member in method.ContainingType.GetMembers(method.Name))
        {
            if (member is IMethodSymbol other
                && !SymbolEqualityComparer.Default.Equals(other.OriginalDefinition, method)
                && other.PartialDefinitionPart is null
                && !SymbolEqualityComparer.Default.Equals(other.PartialImplementationPart, method)
                && !SymbolEqualityComparer.Default.Equals(other, method.PartialDefinitionPart)
                && MethodName(other) == name
                && MethodDescriptor(other.OriginalDefinition) == descriptor)
            {
                return true;
            }
        }
        return false;
    }

    private static string ShortDigest(ISymbol symbol)
    {
        var id = symbol.GetDocumentationCommentId() ?? symbol.ToDisplayString();
        var bytes = SHA256.HashData(Encoding.UTF8.GetBytes(id));
        return Convert.ToHexStringLower(bytes)[..12];
    }
}
