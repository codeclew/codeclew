using System.Text.Json.Nodes;
using Microsoft.CodeAnalysis;

namespace Codeclew.CSharp.Analyzer;

/// <summary>
/// Compiler-resolved attribute observations. No framework names live here: the
/// framework packages interpret attribute types, their base types and interfaces.
/// </summary>
public static class AttributeFacts
{
    public const string Schema = "clr-attribute-facts/1.0";
    public const string Authority = "ROSLYN_RESOLVED_ATTRIBUTES";
    private const int MaxValueDepth = 8;
    private const int MaxArrayValues = 256;

    public static JsonArray Names(ISymbol symbol) =>
        new(symbol.GetAttributes()
            .Select(attribute => attribute.AttributeClass is { } type ? Identities.Class(type) : null)
            .Where(name => name is not null)
            .Distinct()
            .Order(StringComparer.Ordinal)
            .Select(name => (JsonNode?)JsonValue.Create(name))
            .ToArray());

    public static JsonArray Uses(ISymbol symbol, string? inheritedFrom = null)
    {
        var result = new JsonArray();
        foreach (var attribute in symbol.GetAttributes())
        {
            if (Use(attribute, inheritedFrom) is { } use)
            {
                result.Add(use);
            }
        }
        return result;
    }

    public static JsonObject? Use(AttributeData attribute, string? inheritedFrom)
    {
        if (attribute.AttributeClass is not { } type || type.TypeKind == TypeKind.Error)
        {
            return null;
        }
        var row = new JsonObject
        {
            ["attributeType"] = Identities.Class(type),
            ["attributeBases"] = new JsonArray(BaseTypes(type).Select(name => (JsonNode?)JsonValue.Create(name)).ToArray()),
            ["attributeInterfaces"] = new JsonArray(type.AllInterfaces
                .Select(Identities.Class).Distinct().Order(StringComparer.Ordinal)
                .Select(name => (JsonNode?)JsonValue.Create(name)).ToArray()),
            ["constructorArguments"] = new JsonArray(attribute.ConstructorArguments.Select(value => (JsonNode?)Value(value, 0)).ToArray()),
        };
        var named = new JsonObject();
        foreach (var (name, value) in attribute.NamedArguments.OrderBy(pair => pair.Key, StringComparer.Ordinal))
        {
            named[name] = Value(value, 0);
        }
        row["namedArguments"] = named;
        if (attribute.AttributeConstructor is { } constructor)
        {
            row["constructorParameters"] = new JsonArray(constructor.Parameters
                .Select(parameter => (JsonNode?)JsonValue.Create(parameter.Name)).ToArray());
        }
        if (inheritedFrom is not null)
        {
            row["inheritedFrom"] = inheritedFrom;
        }
        return row;
    }

    public static IEnumerable<string> BaseTypes(INamedTypeSymbol type)
    {
        for (var current = type.BaseType; current is not null; current = current.BaseType)
        {
            yield return Identities.Class(current);
        }
    }

    /// <summary>
    /// The <c>clr-attribute-facts/1.0</c> record for an ordinary method: direct method
    /// attributes, containing-type attributes including those inherited from base
    /// types (when the attribute type allows inheritance), and attributes of
    /// overridden base methods.
    /// </summary>
    public static JsonObject? ForMethod(IMethodSymbol method, string identity)
    {
        if (method.MethodKind != MethodKind.Ordinary || method.ContainingType is not { TypeKind: TypeKind.Class } owner)
        {
            return null;
        }
        var typeAttributes = Uses(owner);
        var boundaries = new SortedSet<string>(StringComparer.Ordinal);
        for (var current = owner.BaseType; current is not null; current = current.BaseType)
        {
            foreach (var attribute in current.GetAttributes())
            {
                if (IsInherited(attribute) && Use(attribute, Identities.Class(current)) is { } use)
                {
                    typeAttributes.Add(use);
                }
            }
        }
        var overridden = new JsonArray();
        for (var current = method.OverriddenMethod; current is not null; current = current.OverriddenMethod)
        {
            foreach (var attribute in current.GetAttributes())
            {
                if (IsInherited(attribute) && Use(attribute, Identities.Method(current)) is { } use)
                {
                    overridden.Add(use);
                }
            }
        }
        var methodAttributes = Uses(method);
        if (method.GetAttributes().Any(attribute => attribute.AttributeClass is null or { TypeKind: TypeKind.Error })
            || owner.GetAttributes().Any(attribute => attribute.AttributeClass is null or { TypeKind: TypeKind.Error }))
        {
            boundaries.Add("ATTRIBUTE_TYPE_UNRESOLVED");
        }
        return new JsonObject
        {
            ["schema"] = Schema,
            ["authority"] = Authority,
            ["declaration"] = identity,
            ["method"] = new JsonObject
            {
                ["name"] = method.Name,
                ["accessibility"] = Accessibility(method.DeclaredAccessibility),
                ["isStatic"] = method.IsStatic,
                ["isAbstract"] = method.IsAbstract,
                ["isGeneric"] = method.IsGenericMethod,
                ["isOverride"] = method.IsOverride,
                ["attributes"] = methodAttributes,
            },
            ["containingType"] = new JsonObject
            {
                ["identity"] = Identities.Class(owner),
                ["name"] = owner.Name,
                ["accessibility"] = Accessibility(EffectiveAccessibility(owner)),
                ["isAbstract"] = owner.IsAbstract,
                ["isGeneric"] = owner.IsGenericType,
                ["isNested"] = owner.ContainingType is not null,
                ["attributes"] = typeAttributes,
                ["baseTypes"] = new JsonArray(BaseTypes(owner).Select(name => (JsonNode?)JsonValue.Create(name)).ToArray()),
            },
            ["overriddenAttributes"] = overridden,
            ["boundaries"] = new JsonArray(boundaries.Select(code => (JsonNode?)JsonValue.Create(code)).ToArray()),
            ["coverage"] = new JsonObject
            {
                ["status"] = boundaries.Count == 0 ? "COMPLETE" : "PARTIAL",
                ["scope"] = "METHOD_TYPE_AND_INHERITED_ATTRIBUTES",
            },
        };
    }

    public static string Accessibility(Accessibility accessibility) => accessibility switch
    {
        Microsoft.CodeAnalysis.Accessibility.Public => "PUBLIC",
        Microsoft.CodeAnalysis.Accessibility.Internal => "INTERNAL",
        Microsoft.CodeAnalysis.Accessibility.Protected => "PROTECTED",
        Microsoft.CodeAnalysis.Accessibility.ProtectedOrInternal => "PROTECTED_INTERNAL",
        Microsoft.CodeAnalysis.Accessibility.ProtectedAndInternal => "PRIVATE_PROTECTED",
        Microsoft.CodeAnalysis.Accessibility.Private => "PRIVATE",
        _ => "NOT_APPLICABLE",
    };

    private static Accessibility EffectiveAccessibility(INamedTypeSymbol type)
    {
        var result = type.DeclaredAccessibility;
        for (var outer = type.ContainingType; outer is not null; outer = outer.ContainingType)
        {
            if (outer.DeclaredAccessibility < result)
            {
                result = outer.DeclaredAccessibility;
            }
        }
        return result;
    }

    private static bool IsInherited(AttributeData attribute)
    {
        if (attribute.AttributeClass is not { } type)
        {
            return false;
        }
        for (var current = type; current is not null; current = current.BaseType)
        {
            foreach (var usage in current.GetAttributes())
            {
                if (usage.AttributeClass?.ToDisplayString() != "System.AttributeUsageAttribute")
                {
                    continue;
                }
                foreach (var (name, value) in usage.NamedArguments)
                {
                    if (name == "Inherited" && value.Value is bool inherited)
                    {
                        return inherited;
                    }
                }
                return true;
            }
        }
        return true;
    }

    private static JsonObject Value(TypedConstant constant, int depth)
    {
        if (depth > MaxValueDepth)
        {
            return new JsonObject { ["kind"] = "UNRESOLVED", ["reason"] = "VALUE_DEPTH_LIMIT" };
        }
        if (constant.IsNull)
        {
            return new JsonObject { ["kind"] = "NULL" };
        }
        switch (constant.Kind)
        {
            case TypedConstantKind.Primitive:
                return new JsonObject
                {
                    ["kind"] = "PRIMITIVE",
                    ["type"] = constant.Type?.SpecialType == SpecialType.System_String ? "string" : constant.Type?.ToDisplayString(),
                    ["value"] = constant.Value switch
                    {
                        string text => text,
                        bool flag => flag ? "true" : "false",
                        IFormattable number => number.ToString(null, System.Globalization.CultureInfo.InvariantCulture),
                        var other => other?.ToString(),
                    },
                };
            case TypedConstantKind.Enum:
                var enumType = constant.Type as INamedTypeSymbol;
                var member = enumType?.GetMembers().OfType<IFieldSymbol>()
                    .FirstOrDefault(field => field.HasConstantValue && Equals(field.ConstantValue, constant.Value));
                var row = new JsonObject
                {
                    ["kind"] = "ENUM",
                    ["type"] = enumType is null ? null : Identities.Class(enumType),
                    ["value"] = Convert.ToString(constant.Value, System.Globalization.CultureInfo.InvariantCulture),
                };
                if (member is not null)
                {
                    row["member"] = member.Name;
                }
                return row;
            case TypedConstantKind.Type:
                return new JsonObject
                {
                    ["kind"] = "TYPE",
                    ["identity"] = Identities.TypeIdentity(constant.Value as ITypeSymbol),
                };
            case TypedConstantKind.Array:
                var values = constant.Values;
                var array = new JsonArray(values.Take(MaxArrayValues).Select(value => (JsonNode?)Value(value, depth + 1)).ToArray());
                var result = new JsonObject { ["kind"] = "ARRAY", ["values"] = array };
                if (values.Length > MaxArrayValues)
                {
                    result["truncated"] = true;
                }
                return result;
            default:
                return new JsonObject { ["kind"] = "UNRESOLVED", ["reason"] = "VALUE_UNRESOLVED" };
        }
    }
}
