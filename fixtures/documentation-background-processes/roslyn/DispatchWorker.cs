using System.Threading;
using System.Threading.Tasks;
using Confluent.Kafka;
using Dapper;
using Microsoft.Extensions.Hosting;
using Npgsql;

sealed record DispatchEntry(int Id, string Payload);

sealed class DispatchWorker(NpgsqlDataSource source, IProducer<int, string> broker, bool enabled) : BackgroundService
{
    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        while (true)
        {
            if (enabled) await RelayAsync(source, broker);
            await Task.Delay(50, stoppingToken);
        }
    }

    private static async Task RelayAsync(NpgsqlDataSource source, IProducer<int, string> broker)
    {
        var db = await OpenAsync(source);
        var tx = await db.BeginTransactionAsync();
        var row = await db.QuerySingleOrDefaultAsync<DispatchEntry>(
            "SELECT id, payload FROM dispatch_queue WHERE pending = true LIMIT 1", transaction: tx);
        if (row is null) return;
        await broker.ProduceAsync("dispatch.available", new Message<int, string> { Key = row.Id, Value = row.Payload });
        await db.ExecuteAsync("UPDATE dispatch_queue SET pending = false WHERE id = @id", new { id = row.Id }, transaction: tx);
        await tx.CommitAsync();
    }

    private static async Task<NpgsqlConnection> OpenAsync(NpgsqlDataSource source)
    {
        return await source.OpenConnectionAsync();
    }
}
