using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp.Syntax;
using Microsoft.CodeAnalysis.Operations;

namespace RetainedEffects;

internal static class ManagedValues
{
    private static readonly Dictionary<string, int> Status = new()
    {
        ["Ok"] = 200, ["Created"] = 201, ["CreatedAtAction"] = 201, ["CreatedAtRoute"] = 201,
        ["Accepted"] = 202, ["AcceptedAtAction"] = 202, ["AcceptedAtRoute"] = 202,
        ["NoContent"] = 204, ["BadRequest"] = 400, ["Unauthorized"] = 401,
        ["NotFound"] = 404, ["Conflict"] = 409, ["Gone"] = 410, ["UnprocessableEntity"] = 422,
        ["InternalServerError"] = 500,
    };

    internal static object? Input(IParameterSymbol parameter)
    {
        var type = parameter.Type.TypeKind == TypeKind.Error ? null : parameter.Type.ToDisplayString();
        var bindings = parameter.GetAttributes().Select(attribute =>
        {
            var attributeType = attribute.AttributeClass;
            var assembly = attributeType?.ContainingAssembly.Name;
            var category = (assembly, attributeType?.ToDisplayString()) switch
            {
                ("Microsoft.AspNetCore.Mvc.Core", "Microsoft.AspNetCore.Mvc.FromBodyAttribute") => "body",
                ("Microsoft.AspNetCore.Mvc.Core", "Microsoft.AspNetCore.Mvc.FromRouteAttribute") => "route",
                ("Microsoft.AspNetCore.Mvc.Core", "Microsoft.AspNetCore.Mvc.FromQueryAttribute") => "query",
                ("Microsoft.AspNetCore.Mvc.Core", "Microsoft.AspNetCore.Mvc.FromHeaderAttribute") => "header",
                ("Microsoft.AspNetCore.Mvc.Core", "Microsoft.AspNetCore.Mvc.FromFormAttribute") => "form",
                ("Microsoft.AspNetCore.Mvc.Core", "Microsoft.AspNetCore.Mvc.FromServicesAttribute") => "services",
                ("Microsoft.AspNetCore.Http.Abstractions", "Microsoft.AspNetCore.Http.AsParametersAttribute") => "parameter group",
                _ => null,
            };
            return new { category, name = attribute.NamedArguments.FirstOrDefault(value => value.Key == "Name").Value.Value as string };
        }).Where(attribute => attribute.category is not null).Take(2).ToArray();
        var ambient = type is "Microsoft.AspNetCore.Http.HttpContext" or "Microsoft.AspNetCore.Http.HttpRequest" or
            "Microsoft.AspNetCore.Http.HttpResponse" or "System.Security.Claims.ClaimsPrincipal" or
            "System.Threading.CancellationToken";
        return type is null && bindings.Length == 0 ? null : new
        {
            type,
            binding = bindings.Length == 1 ? bindings[0].category : ambient && bindings.Length == 0 ? "context" : null,
            bindingName = bindings.Length == 1 ? bindings[0].name : null,
        };
    }

    internal static object? HttpResult(IMethodSymbol method, IEnumerable<IArgumentOperation> arguments)
    {
        var owner = method.ContainingType.ToDisplayString();
        var assembly = method.ContainingAssembly.Name;
        var mvc = assembly == "Microsoft.AspNetCore.Mvc.Core";
        var factory = mvc && owner == "Microsoft.AspNetCore.Mvc.ControllerBase" || assembly == "Microsoft.AspNetCore.Http.Results" &&
            owner is "Microsoft.AspNetCore.Http.Results" or "Microsoft.AspNetCore.Http.TypedResults";
        var constructor = mvc && method.MethodKind == MethodKind.Constructor &&
            method.ContainingType.ContainingNamespace.ToDisplayString() == "Microsoft.AspNetCore.Mvc";
        if (!factory && !constructor) return null;

        var typeName = method.ContainingType.Name;
        var name = factory ? method.Name : typeName.EndsWith("ObjectResult", StringComparison.Ordinal) && typeName != "ObjectResult"
            ? typeName[..^12] : typeName.EndsWith("Result", StringComparison.Ordinal) ? typeName[..^6] : typeName;
        var codeArgument = arguments.FirstOrDefault(argument => argument.Parameter?.Name == "statusCode");
        int? status = Status.TryGetValue(name, out var code) ? code : null;
        var statusValue = codeArgument?.Value;
        while (statusValue is IConversionOperation { OperatorMethod: null } conversion)
            statusValue = conversion.Operand;
        if (name is "StatusCode" or "Json" or "Problem" or "ValidationProblem" &&
            statusValue?.ConstantValue is { HasValue: true, Value: int explicitCode }
            && explicitCode is >= 100 and <= 599)
            status = explicitCode;
        if (name == "Json" && (codeArgument?.ArgumentKind == ArgumentKind.DefaultValue ||
            statusValue is IDefaultValueOperation || statusValue?.ConstantValue is { HasValue: true, Value: null }))
            status = 200;
        if (status is null) return null;
        var payload = arguments.FirstOrDefault(argument => argument.Parameter?.Name is "value" or "data" or "error" or "valueToSerialize");
        return new { status, payload = payload?.Parameter?.Ordinal };
    }

    internal static bool IsResponseConstruction(IMethodSymbol method)
    {
        if (method.ContainingAssembly.Name is not ("Microsoft.AspNetCore.Mvc.Core" or "Microsoft.AspNetCore.Http.Results"))
            return false;
        if ((method.MethodKind == MethodKind.Constructor ? method.ContainingType : method.ReturnType) is not INamedTypeSymbol result)
            return false;
        static bool ResponseInterface(INamedTypeSymbol type) => type.ToDisplayString() is
            "Microsoft.AspNetCore.Mvc.IActionResult" or "Microsoft.AspNetCore.Http.IResult";
        return ResponseInterface(result) || result.AllInterfaces.Any(ResponseInterface);
    }

    internal static object[] RecordFields(IMethodSymbol? constructor)
    {
        if (constructor?.ContainingType.IsRecord != true || !constructor.Parameters.All(parameter =>
            parameter.DeclaringSyntaxReferences.SingleOrDefault()?.GetSyntax() is ParameterSyntax { Parent.Parent: RecordDeclarationSyntax }))
            return [];
        return constructor.Parameters.Select(parameter => constructor.ContainingType.GetMembers(parameter.Name)
            .OfType<IPropertySymbol>().SingleOrDefault(property => property.IsImplicitlyDeclared ||
                property.DeclaringSyntaxReferences.Any(reference => reference.GetSyntax() is ParameterSyntax)))
            .OfType<IPropertySymbol>().Select(property => new { name = property.Name, id = property.GetDocumentationCommentId() }).ToArray();
    }

    internal static string? ResourceFamily(ITypeSymbol? type)
    {
        if (type is INamedTypeSymbol named && named.TypeArguments.Length == 1 &&
            named.ContainingNamespace.ToDisplayString() == "System.Threading.Tasks" && named.Name is "Task" or "ValueTask")
            type = named.TypeArguments[0];
        foreach (var (framework, family) in new[] { ("DbConnection", "connection"), ("DbDataSource", "data source"),
                     ("DbTransaction", "transaction"), ("DbCommand", "command"), ("DbDataReader", "reader"), ("DbParameter", "parameter") })
            if (AdoOperations.IsType(type, framework) || type?.TypeKind == TypeKind.Error &&
                type.Name.EndsWith(framework[2..], StringComparison.Ordinal))
                return family;
        return null;
    }

    internal static bool IsList(ITypeSymbol? type) => type is INamedTypeSymbol named &&
        named.ContainingAssembly?.Name == "System.Private.CoreLib" && named.Name == "List" &&
        named.ContainingNamespace.ToDisplayString() == "System.Collections.Generic" && named.TypeArguments.Length == 1;

    internal static string? JsonProperty(IPropertySymbol property)
    {
        if (property.ContainingAssembly.Name != "System.Text.Json") return null;
        return (property.ContainingType.ToDisplayString(), property.Name) switch
        {
            ("System.Text.Json.JsonDocument", "RootElement") => "json-root",
            ("System.Text.Json.Nodes.JsonNode", _) when property.IsIndexer &&
                property.Parameters is [{ Type.SpecialType: SpecialType.System_String }] => "json-key",
            _ => null,
        };
    }

    internal static string? ValueFactory(IMethodSymbol method)
    {
        var assembly = method.ContainingAssembly.Name;
        var owner = method.ContainingType.ToDisplayString();
        return (assembly, owner, method.Name) switch
        {
            ("System.Private.CoreLib", "System.Guid", "NewGuid") when method.Parameters.IsEmpty => "GUID_NEW",
            ("System.Text.Json", "System.Text.Json.JsonSerializer", "Serialize")
                when method.ReturnType.SpecialType == SpecialType.System_String => "JSON_SERIALIZE",
            ("System.Text.Json", "System.Text.Json.JsonSerializer", "Deserialize")
                when method.Arity == 1 && !method.Parameters.IsEmpty => "JSON_DESERIALIZE",
            ("System.Text.Json", "System.Text.Json.JsonDocument" or "System.Text.Json.Nodes.JsonNode", "Parse")
                => "JSON_PARSE",
            ("System.Text.Json", "System.Text.Json.JsonElement", "GetProperty")
                when method.Parameters is [{ Type.SpecialType: SpecialType.System_String }] => "JSON_PROPERTY",
            ("System.Text.Json", "System.Text.Json.JsonElement",
                "GetString" or "GetBoolean" or "GetGuid" or "GetInt32" or "GetInt64" or "GetDecimal" or "GetDouble" or "GetRawText")
                when method.Parameters.IsEmpty => "JSON_VALUE",
            ("System.Text.Json", "System.Text.Json.Nodes.JsonNode" or "System.Text.Json.Nodes.JsonValue", "GetValue")
                when method.Arity == 1 && method.Parameters.IsEmpty => "JSON_VALUE",
            ("System.Linq", "System.Linq.Enumerable", "Select") when method.Parameters.Length == 2 &&
                method.Parameters[1].Type is INamedTypeSymbol { Arity: 2 } => "LINQ_SELECT",
            ("System.Linq", "System.Linq.Enumerable", "FirstOrDefault") when method.Parameters.Length is 1 or 2 &&
                (method.Parameters.Length == 1 || method.Parameters[1].Name == "predicate") => "LINQ_FIRST_OR_DEFAULT",
            _ => null,
        };
    }

    internal static bool NullDefault(ITypeSymbol type) => type.IsReferenceType ||
        type is INamedTypeSymbol value && value.OriginalDefinition.SpecialType == SpecialType.System_Nullable_T;

    internal static bool IsLocalJsonSerialization(IMethodSymbol method, IInvocationOperation invocation)
    {
        // Default serialization of an anonymous scalar payload cannot call application converters or getters.
        if (ValueFactory(method) != "JSON_SERIALIZE" || method.Parameters is not [_, { Name: "options" }]) return false;
        var arguments = invocation.Arguments;
        return arguments.Any(argument => argument.Parameter?.Ordinal == 0 && LocalJsonValue(argument.Value.Type)) &&
            arguments.All(argument => argument.Parameter?.Ordinal == 0 ||
                argument.Parameter?.Ordinal == 1 && argument.Value.ConstantValue is { HasValue: true, Value: null });
    }

    private static bool LocalJsonValue(ITypeSymbol? type, int depth = 0)
    {
        if (type is null || depth > 4) return false;
        if (type.SpecialType is SpecialType.System_Boolean or SpecialType.System_String or SpecialType.System_Char or
            SpecialType.System_Byte or SpecialType.System_SByte or SpecialType.System_Int16 or SpecialType.System_UInt16 or
            SpecialType.System_Int32 or SpecialType.System_UInt32 or SpecialType.System_Int64 or SpecialType.System_UInt64 or
            SpecialType.System_Decimal or SpecialType.System_Single or SpecialType.System_Double or SpecialType.System_DateTime)
            return true;
        if (type is not INamedTypeSymbol named) return false;
        if (named.Name == "Guid" && named.ContainingNamespace.ToDisplayString() == "System" &&
            named.ContainingAssembly.Name == "System.Private.CoreLib") return true;
        if (named.OriginalDefinition.SpecialType == SpecialType.System_Nullable_T)
            return LocalJsonValue(named.TypeArguments[0], depth + 1);
        return named.IsAnonymousType && named.GetMembers().OfType<IPropertySymbol>()
            .All(property => LocalJsonValue(property.Type, depth + 1));
    }
}
