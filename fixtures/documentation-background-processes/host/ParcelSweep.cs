using System;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Extensions.Hosting;

sealed class ParcelSweep : BackgroundService
{
    private int processed;

    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        while (!stoppingToken.IsCancellationRequested)
        {
            try
            {
                if (await TryReserveAsync(stoppingToken)) processed += 1;
            }
            catch (InvalidOperationException)
            {
                return;
            }

            await Task.Delay(100, stoppingToken);
        }
    }

    private static Task<bool> TryReserveAsync(CancellationToken token) =>
        Task.FromResult(!token.IsCancellationRequested);
}

sealed class UnregisteredSweep : BackgroundService
{
    protected override Task ExecuteAsync(CancellationToken stoppingToken) =>
        Task.CompletedTask;
}
