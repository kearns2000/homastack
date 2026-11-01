using System.Net;
using Homa.Rpc;
using Homa.Rpc.Hosting;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;

namespace Homa.Rpc.Tests;

public class HostingTests
{
    [Fact]
    public async Task Dependency_injection_echoes_opens_a_scope_per_call_and_logs()
    {
        var steps = new List<string>();
        var services = new ServiceCollection();
        services.AddSingleton(steps);
        services.AddLogging(logging =>
        {
            logging.AddProvider(new CollectingLoggerProvider(steps));
            logging.SetMinimumLevel(LogLevel.Information);
        });
        services.AddScoped<Tick>();
        services.AddHomaRpc(rpc =>
        {
            rpc.UseMiddleware<MarkMiddleware>();
            rpc.Map<string, string>("echo", (text, context) =>
            {
                var tick = context.GetRequestServices()!.GetRequiredService<Tick>();
                tick.Value++;
                steps.Add("handler:" + tick.Value);
                return new ValueTask<string>(text);
            });
        });
        services.AddSingleton<IRpcCaller>(sp => new PipelineCaller(sp.GetRequiredService<RpcPipeline>()));
        services.AddHomaRpcClient();
        services.AddHomaRpcServer(new IPEndPoint(IPAddress.Loopback, 4000));

        using var provider = services.BuildServiceProvider();
        var hosted = Assert.Single(provider.GetServices<IHostedService>().OfType<HomaRpcServerHostedService>());
        Assert.NotNull(hosted);
        var client = provider.GetRequiredService<HomaRpcClient>();

        Assert.Equal("one", await client.CallAsync<string, string>(Echo.Peer, "echo", "one"));
        Assert.Equal("two", await client.CallAsync<string, string>(Echo.Peer, "echo", "two"));

        Assert.Equal(2, steps.Count(step => step == "handler:1"));
        Assert.Equal(
            ["log:started", "mark-before", "handler:1", "mark-after", "log:completed"],
            steps.Where(step => step.StartsWith("log:", StringComparison.Ordinal) || step.StartsWith("mark", StringComparison.Ordinal) || step.StartsWith("handler", StringComparison.Ordinal)).Take(5).ToArray());
    }

    [Fact]
    public void Hosted_service_can_be_constructed_without_starting()
    {
        var services = new ServiceCollection();
        services.AddLogging();
        services.AddHomaRpc(rpc => rpc.DisableRequestLogging().Map<string, string>("echo", (text, _) => new ValueTask<string>(text)));
        services.AddHomaRpcServer(new IPEndPoint(IPAddress.IPv6Loopback, 4001));

        using var provider = services.BuildServiceProvider();
        var hosted = provider.GetRequiredService<IHostedService>();

        Assert.IsType<HomaRpcServerHostedService>(hosted);
        var binding = provider.GetRequiredService<HomaRpcServerBinding>();
        Assert.Equal(4001, binding.Listen.Port);
    }

    private sealed class Tick
    {
        public int Value { get; set; }
    }

    private sealed class MarkMiddleware : IRpcMiddleware
    {
        private readonly List<string> _steps;

        public MarkMiddleware(List<string> steps) => _steps = steps;

        public async ValueTask InvokeAsync(RpcInvocation invocation, RpcDispatchDelegate next)
        {
            _steps.Add("mark-before");
            await next(invocation);
            _steps.Add("mark-after");
        }
    }

    private sealed class CollectingLoggerProvider(List<string> steps) : ILoggerProvider
    {
        public ILogger CreateLogger(string categoryName) => new CollectingLogger(steps);

        public void Dispose()
        {
        }
    }

    private sealed class CollectingLogger(List<string> steps) : ILogger
    {
        public IDisposable? BeginScope<TState>(TState state) where TState : notnull => null;

        public bool IsEnabled(LogLevel logLevel) => true;

        public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter)
        {
            var text = formatter(state, exception);
            if (text.Contains("started", StringComparison.Ordinal))
                steps.Add("log:started");
            else if (text.Contains("completed", StringComparison.Ordinal))
                steps.Add("log:completed");
        }
    }
}
