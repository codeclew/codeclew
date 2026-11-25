using Microsoft.AspNetCore.Builder;
using Microsoft.Extensions.DependencyInjection;

var builder = WebApplication.CreateBuilder(args);
builder.Services.AddHostedService<ParcelSweep>();
var app = builder.Build();
app.MapGet("/health", () => "ok");
app.Run();
