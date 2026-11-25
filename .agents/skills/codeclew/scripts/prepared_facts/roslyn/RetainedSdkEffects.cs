using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp.Syntax;

namespace RetainedEffects;

/// <summary>Recognize nominal API families when a package is outside the retained compilation.</summary>
internal static class RetainedSdkEffects
{
    private static readonly string[] Scopes =
    ["Confluent.Kafka", "RabbitMQ.Client", "Azure.Messaging.ServiceBus", "Npgsql", "Microsoft.Data.SqlClient", "System.Data.SqlClient"];

    internal static string? Family(SemanticModel model, ExpressionSyntax receiver, string? method, string? resource)
    {
        if (method is null) return null;
        var type = Name(model, receiver, 0);
        if (type is not null && Match(type, method) is { } family) return family;
        if (Dapper(model, receiver, method, resource))
            return "database";
        return null;
    }

    internal static bool Publication(string? method, string? family) => family == "broker" && method is
        "Produce" or "ProduceAsync" or "BasicPublish" or "BasicPublishAsync" or "SendMessageAsync" or "SendMessagesAsync";

    internal static bool KafkaProducer(SemanticModel model, ExpressionSyntax receiver, string? method) =>
        method is "Produce" or "ProduceAsync" &&
        Name(model, receiver, 0) is "Confluent.Kafka.IProducer" or "Confluent.Kafka.Producer";

    internal static int? SelectedKafkaTopic(IMethodSymbol method) =>
        method.Name is "Produce" or "ProduceAsync" &&
        method.ContainingType.Name is "IProducer" or "Producer" &&
        method.ContainingNamespace.ToDisplayString() == "Confluent.Kafka" &&
        method.ContainingAssembly.Name == "Confluent.Kafka"
        ? method.Parameters.FirstOrDefault(parameter => parameter.Name == "topic" &&
            parameter.Type.SpecialType == SpecialType.System_String)?.Ordinal
        : null;

    internal static bool Dapper(SemanticModel model, ExpressionSyntax receiver, string? method, string? resource) =>
        method is ("Query" or "QueryAsync" or "QueryFirst" or "QueryFirstAsync" or "QueryFirstOrDefault" or
            "QueryFirstOrDefaultAsync" or "QuerySingle" or "QuerySingleAsync" or "QuerySingleOrDefault" or
            "QuerySingleOrDefaultAsync" or "QueryMultiple" or "QueryMultipleAsync" or "Execute" or "ExecuteAsync" or
            "ExecuteScalar" or "ExecuteScalarAsync") && Uses(receiver, "Dapper") &&
        (resource == "connection" || Name(model, receiver, 0) is { } type && Match(type, "Open") == "database");

    internal static string? Construction(SemanticModel model, BaseObjectCreationExpressionSyntax creation) =>
        Name(model, creation, 0) is { } type ? Match(type, ".ctor") : null;

    private static string? Match(string type, string method) => type switch
    {
        "Confluent.Kafka.IProducer" or "Confluent.Kafka.Producer" when method is "Produce" or "ProduceAsync" or "Flush" or
            "InitTransactions" or "BeginTransaction" or "CommitTransaction" or "AbortTransaction" => "broker",
        "Confluent.Kafka.IConsumer" or "Confluent.Kafka.Consumer" when method is "Subscribe" or "Consume" or "Commit" or "Close" => "broker",
        "Confluent.Kafka.ProducerBuilder" or "Confluent.Kafka.ConsumerBuilder" when method is "Build" => "broker",
        "RabbitMQ.Client.IChannel" or "RabbitMQ.Client.IModel" when method is "BasicPublish" or "BasicPublishAsync" or
            "BasicConsume" or "BasicConsumeAsync" or "BasicAck" or "BasicAckAsync" => "broker",
        "RabbitMQ.Client.ConnectionFactory" or "RabbitMQ.Client.IConnection" when method is "CreateConnection" or
            "CreateConnectionAsync" or "CreateChannelAsync" or "CreateModel" => "broker",
        "Azure.Messaging.ServiceBus.ServiceBusSender" when method is "SendMessageAsync" or "SendMessagesAsync" or
            "ScheduleMessageAsync" or "ScheduleMessagesAsync" => "broker",
        "Azure.Messaging.ServiceBus.ServiceBusReceiver" when method is "ReceiveMessageAsync" or "ReceiveMessagesAsync" or
            "CompleteMessageAsync" or "AbandonMessageAsync" or "DeadLetterMessageAsync" => "broker",
        "Azure.Messaging.ServiceBus.ServiceBusClient" when method is ".ctor" or "CreateSender" or "CreateReceiver" or
            "CreateProcessor" => "broker",
        "Azure.Messaging.ServiceBus.ServiceBusProcessor" when method is "StartProcessingAsync" or "StopProcessingAsync" => "broker",
        "Npgsql.NpgsqlConnection" or "Npgsql.NpgsqlDataSource" or "Microsoft.Data.SqlClient.SqlConnection" or
            "System.Data.SqlClient.SqlConnection" when method is ".ctor" or "Open" or "OpenAsync" or "Close" or "CloseAsync" or
                "OpenConnection" or "OpenConnectionAsync" or "CreateCommand" or "BeginTransaction" or "BeginTransactionAsync" => "database",
        "Npgsql.NpgsqlTransaction" or "Microsoft.Data.SqlClient.SqlTransaction" or "System.Data.SqlClient.SqlTransaction" when
            method is "Commit" or "CommitAsync" or "Rollback" or "RollbackAsync" => "database",
        "Npgsql.NpgsqlCommand" or "Microsoft.Data.SqlClient.SqlCommand" or "System.Data.SqlClient.SqlCommand" when method is
            ".ctor" or "ExecuteNonQuery" or "ExecuteNonQueryAsync" or "ExecuteReader" or "ExecuteReaderAsync" or
            "ExecuteScalar" or "ExecuteScalarAsync" or "CreateParameter" => "database",
        "Npgsql.NpgsqlDataReader" or "Microsoft.Data.SqlClient.SqlDataReader" or "System.Data.SqlClient.SqlDataReader" when
            method is "Read" or "ReadAsync" or "NextResult" or "NextResultAsync" => "database",
        _ => null,
    };

    private static string? Name(SemanticModel model, ExpressionSyntax receiver, int depth)
    {
        if (depth > 5) return null;
        var type = model.GetTypeInfo(receiver).Type;
        if (type is { TypeKind: not TypeKind.Error })
        {
            var scope = type.ContainingNamespace?.ToDisplayString();
            return scope is not null && Scopes.Contains(scope) && type.ContainingAssembly is { } assembly &&
                !SymbolEqualityComparer.Default.Equals(assembly, model.Compilation.Assembly) ? scope + "." + type.Name : null;
        }
        if (receiver is ParenthesizedExpressionSyntax parentheses) return Name(model, parentheses.Expression, depth + 1);
        if (receiver is AwaitExpressionSyntax awaited) return Name(model, awaited.Expression, depth + 1);
        if (receiver is ObjectCreationExpressionSyntax creation) return TypeName(creation.Type);
        if (receiver is InvocationExpressionSyntax { Expression: MemberAccessExpressionSyntax access })
        {
            var result = (Name(model, access.Expression, depth + 1), access.Name.Identifier.ValueText) switch
            {
                ("Confluent.Kafka.ProducerBuilder", "Build") => "Confluent.Kafka.IProducer",
                ("Confluent.Kafka.ConsumerBuilder", "Build") => "Confluent.Kafka.IConsumer",
                ("Npgsql.NpgsqlDataSource", "OpenConnection" or "OpenConnectionAsync") => "Npgsql.NpgsqlConnection",
                ("Npgsql.NpgsqlConnection" or "Npgsql.NpgsqlDataSource", "CreateCommand") => "Npgsql.NpgsqlCommand",
                ("Npgsql.NpgsqlConnection", "BeginTransaction" or "BeginTransactionAsync") => "Npgsql.NpgsqlTransaction",
                ("Microsoft.Data.SqlClient.SqlConnection", "CreateCommand") => "Microsoft.Data.SqlClient.SqlCommand",
                ("System.Data.SqlClient.SqlConnection", "CreateCommand") => "System.Data.SqlClient.SqlCommand",
                ("Microsoft.Data.SqlClient.SqlConnection", "BeginTransaction" or "BeginTransactionAsync") => "Microsoft.Data.SqlClient.SqlTransaction",
                ("System.Data.SqlClient.SqlConnection", "BeginTransaction" or "BeginTransactionAsync") => "System.Data.SqlClient.SqlTransaction",
                ("Azure.Messaging.ServiceBus.ServiceBusClient", "CreateSender") => "Azure.Messaging.ServiceBus.ServiceBusSender",
                ("Azure.Messaging.ServiceBus.ServiceBusClient", "CreateReceiver") => "Azure.Messaging.ServiceBus.ServiceBusReceiver",
                _ => null,
            };
            if (result is not null) return result;
        }
        var symbol = model.GetSymbolInfo(receiver).Symbol;
        if (symbol is null || symbol.DeclaringSyntaxReferences.Length != 1) return null;
        return symbol.DeclaringSyntaxReferences[0].GetSyntax() switch
        {
            ParameterSyntax { Type: { } declared } => TypeName(declared),
            PropertyDeclarationSyntax property => TypeName(property.Type),
            MethodDeclarationSyntax method => TypeName(method.ReturnType),
            VariableDeclaratorSyntax { Parent: VariableDeclarationSyntax { Type.IsVar: false } variable } => TypeName(variable.Type),
            VariableDeclaratorSyntax { Initializer.Value: { } value } =>
                Name(model.Compilation.GetSemanticModel(value.SyntaxTree), value, depth + 1),
            _ => null,
        };
    }

    private static string? TypeName(TypeSyntax node)
    {
        if (node is NullableTypeSyntax nullable) return TypeName(nullable.ElementType);
        if (node is GenericNameSyntax { Identifier.ValueText: "Task" or "ValueTask" } task &&
            task.TypeArgumentList.Arguments is [var inner]) return TypeName(inner);
        if (node is QualifiedNameSyntax qualified)
        {
            var scope = qualified.Left.ToString().Replace("global::", "", StringComparison.Ordinal);
            scope = Alias(node, scope)?.ToString().Replace("global::", "", StringComparison.Ordinal) ?? scope;
            return Scopes.Contains(scope) ? scope + "." + qualified.Right.Identifier.ValueText : null;
        }
        if (node is not SimpleNameSyntax simple) return null;
        if (Alias(node, simple.Identifier.ValueText) is QualifiedNameSyntax alias)
            return TypeName(alias);
        var scopes = Scopes.Where(scope => Uses(node, scope) && KnownType(scope + "." + simple.Identifier.ValueText)).Take(2).ToArray();
        return scopes.Length == 1 ? scopes[0] + "." + simple.Identifier.ValueText : null;
    }

    private static bool KnownType(string type) => new[] { "Produce", "Consume", "Build", "BasicPublish", "CreateConnection",
        "CreateChannelAsync", "SendMessageAsync", "ReceiveMessageAsync", "StartProcessingAsync", ".ctor", "Open", "Commit", "Read" }
        .Any(method => Match(type, method) is not null);

    private static NameSyntax? Alias(SyntaxNode node, string name) => Imports(node)
        .FirstOrDefault(usingDirective => usingDirective.Alias?.Name.Identifier.ValueText == name)?.Name;

    private static IEnumerable<UsingDirectiveSyntax> Imports(SyntaxNode node) => node.AncestorsAndSelf().SelectMany(ancestor => ancestor switch
    {
        CompilationUnitSyntax compilation => compilation.Usings,
        BaseNamespaceDeclarationSyntax declaration => declaration.Usings,
        _ => default,
    });

    private static bool Uses(SyntaxNode node, string scope) => Imports(node).Any(directive => IsNamespace(directive, scope)) ||
        node.Ancestors().OfType<BaseNamespaceDeclarationSyntax>().Any(declaration => declaration.Name.ToString() == scope);

    private static bool IsNamespace(UsingDirectiveSyntax directive, string scope) =>
        directive.Alias is null && directive.StaticKeyword.RawKind == 0 && directive.Name?.ToString() == scope;
}
