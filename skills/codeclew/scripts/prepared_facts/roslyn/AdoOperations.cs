using Microsoft.CodeAnalysis;

namespace RetainedEffects;

internal static class AdoOperations
{
    internal static bool IsType(ITypeSymbol? type, string name)
    {
        for (var current = type as INamedTypeSymbol; current is not null; current = current.BaseType)
            if (current.Name == name && current.ContainingAssembly?.Name == "System.Data.Common" &&
                current.ContainingNamespace.ToDisplayString() == "System.Data.Common")
                return true;
        return false;
    }

    internal static bool IsMember(string name) =>
        name is "CommandText" or "Connection" or "Transaction" or "CommandType" or "Parameters" or "ParameterName" or "Value";

    internal static string? Property(IPropertySymbol property)
    {
        var owner = property.ContainingType;
        var name = property.Name;
        if (IsType(owner, "DbCommand") &&
            name is "CommandText" or "Connection" or "Transaction" or "CommandType" or "Parameters")
            return name;
        if (IsType(owner, "DbParameter") && name is "ParameterName" or "Value")
            return name;
        if (property.IsIndexer && (IsType(owner, "DbParameterCollection") || IsType(owner, "DbDataReader")))
            return "Item";
        return null;
    }

    internal static bool IsCancellationToken(ITypeSymbol? type) => type is INamedTypeSymbol named &&
        named.TypeKind == TypeKind.Struct && named.Name == "CancellationToken" &&
        named.ContainingAssembly.Name == "System.Private.CoreLib" &&
        named.ContainingNamespace.ToDisplayString() == "System.Threading";

    private static bool IsTransactionCommit(IMethodSymbol method)
    {
        while (method.OverriddenMethod is { } overridden) method = overridden;
        var type = method.ContainingType;
        if (type.Name != "DbTransaction" || !IsType(type, "DbTransaction") || method.IsStatic) return false;
        return method.Name == "Commit" ? method.Parameters.IsEmpty :
            method.Parameters is [{ RefKind: RefKind.None } parameter] && IsCancellationToken(parameter.Type);
    }

    internal static string? Role(IMethodSymbol target)
    {
        var name = target.Name;
        var owner = target.ContainingType;
        if (IsType(owner, "DbConnection"))
            return name switch
            {
                "CreateCommand" => "ADO_CREATE_COMMAND",
                "BeginTransaction" or "BeginTransactionAsync" => "TRANSACTION_BEGIN_REQUEST",
                _ => null,
            };
        if (IsType(owner, "DbDataSource") && name == "CreateCommand") return "ADO_CREATE_DATA_SOURCE_COMMAND";
        if (IsType(owner, "DbTransaction"))
            return name switch
            {
                "Commit" or "CommitAsync" when IsTransactionCommit(target) => "TRANSACTION_COMMIT_REQUEST",
                "Rollback" or "RollbackAsync" => "TRANSACTION_ROLLBACK_REQUEST",
                _ => null,
            };
        if (IsType(owner, "DbCommand"))
            return name switch
            {
                "CreateParameter" => "ADO_CREATE_PARAMETER",
                "ExecuteReader" or "ExecuteReaderAsync" => "ADO_EXECUTE_READER",
                "ExecuteScalar" or "ExecuteScalarAsync" => "ADO_EXECUTE_SCALAR",
                "ExecuteNonQuery" or "ExecuteNonQueryAsync" => "ADO_EXECUTE_WRITE",
                _ => null,
            };
        if (IsType(owner, "DbParameterCollection") && name == "Add") return "ADO_ADD_PARAMETER";
        if (IsType(owner, "DbDataReader"))
            return name switch
            {
                "Read" or "ReadAsync" => "ADO_READER_READ",
                "NextResult" or "NextResultAsync" => "ADO_READER_NEXT_RESULT",
                "GetOrdinal" => "ADO_READER_ORDINAL",
                "GetValue" or "GetFieldValue" or "GetFieldValueAsync" or "GetString" or "GetGuid" or "GetBoolean"
                    or "GetByte" or "GetInt16" or "GetInt32" or "GetInt64" or "GetDecimal" or "GetDouble" or "GetFloat"
                    => "ADO_READER_VALUE",
                _ => null,
            };
        return null;
    }
}
