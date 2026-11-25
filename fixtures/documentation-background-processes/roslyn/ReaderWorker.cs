using System.Data.Common;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Confluent.Kafka;
using Microsoft.Extensions.Hosting;
using Npgsql;

sealed class ReaderWorker(string settings, IProducer<string, string> relay, bool enabled) : BackgroundService
{
    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        while (true)
        {
            if (enabled) await ShipAsync(settings, relay, stoppingToken);
            await Task.Delay(50, stoppingToken);
        }
    }

    private static async Task ShipAsync(string settings, IProducer<string, string> relay, CancellationToken token)
    {
        await using var db = await ConnectAsync(settings, token);
        await using var tx = await db.BeginTransactionAsync(token);
        await using var pending = db.CreateCommand();
        pending.Transaction = tx;
        pending.CommandText = "SELECT id, payload FROM shipments WHERE pending = true LIMIT 1";
        await using var reader = await pending.ExecuteReaderAsync(token);
        if (!await reader.ReadAsync(token)) return;
        var id = reader.GetGuid(0);
        var payload = reader.GetString(1);
        using var document = JsonDocument.Parse(payload);
        var routingKey = document.RootElement.GetProperty("routing").GetProperty("key").GetString();
        await relay.ProduceAsync("shipment.available", new Message<string, string> { Key = routingKey, Value = payload }, token);
        await reader.CloseAsync();
        await using var update = db.CreateCommand();
        update.Transaction = tx;
        update.CommandText = "UPDATE shipments SET pending = false WHERE id = @id";
        var selectedId = update.CreateParameter();
        selectedId.ParameterName = "id";
        selectedId.Value = id;
        update.Parameters.Add(selectedId);
        await update.ExecuteNonQueryAsync(token);
        await tx.CommitAsync(token);
    }

    private static async Task<DbConnection> ConnectAsync(string settings, CancellationToken token)
    {
        var db = new NpgsqlConnection(settings);
        try
        {
            await db.OpenAsync(token);
            return db;
        }
        catch
        {
            await db.DisposeAsync();
            throw;
        }
    }
}
