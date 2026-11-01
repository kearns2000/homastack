using System.Net;
using Homa.Rpc;
using Homa.Rpc.Hosting;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;

namespace Homa.Rpc.Tests;

public class HostingTests
{
    [Fact]
    public async Task Dependency_injection_echoes_opens_a_scope_per_call_and_logs()
    {
        var steps = new List<string>();
        var services = new ServiceCollection();
        services.AddSingleton(steps);
        services.AddSingleton<IHostApplicationLifetime>(new TestLifetime());
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
        services.AddSingleton<IHostApplicationLifetime>(new TestLifetime());
        services.AddHomaRpc(rpc => rpc.DisableRequestLogging().Map<string, string>("echo", (text, _) => new ValueTask<string>(text)));
        services.AddHomaRpcServer(new IPEndPoint(IPAddress.IPv6Loopback, 4001));

        using var provider = services.BuildServiceProvider();
        var hosted = provider.GetRequiredService<IHostedService>();

        Assert.IsType<HomaRpcServerHostedService>(hosted);
        var binding = provider.GetRequiredService<HomaRpcServerBinding>();
        Assert.Equal(4001, binding.Listen.Port);
        Assert.Equal(HomaRpcServer.DefaultMaxInFlight, binding.MaxInFlight);
    }

    [Fact]
    public async Task Response_serialization_runs_before_the_call_scope_is_disposed()
    {
        var services = new ServiceCollection();
        services.AddLogging();
        services.AddSingleton<IHostApplicationLifetime>(new TestLifetime());
        services.AddScoped<Probe>();
        services.AddHomaRpc(rpc =>
        {
            rpc.Map<string, TouchResponse>("touch", (_, context) =>
            {
                var probe = context.GetRequestServices()!.GetRequiredService<Probe>();
                return new ValueTask<TouchResponse>(new TouchResponse(probe));
            });
        });
        services.AddSingleton<IRpcCaller>(sp => new PipelineCaller(sp.GetRequiredService<RpcPipeline>()));
        services.AddHomaRpcClient();

        using var provider = services.BuildServiceProvider();
        var client = provider.GetRequiredService<HomaRpcClient>();

        var reply = await client.CallAsync<string, TouchBody>(Echo.Peer, "touch", "x");

        Assert.Equal("live", reply.Text);
    }

    [Fact]
    public void A_faulted_receive_loop_stops_the_host()
    {
        var lifetime = new TestLifetime();
        var hosted = new HomaRpcServerHostedService(
            Echo.Pipeline(),
            new HomaRpcServerBinding(new IPEndPoint(IPAddress.Loopback, 9)),
            NullLogger<HomaRpcServerHostedService>.Instance,
            lifetime);
        var loop = Task.FromException(new InvalidOperationException("recv failed"));

        hosted.ObserveLoop(loop);

        Assert.True(lifetime.Stop.Wait(TimeSpan.FromSeconds(2)));
    }

    [Fact]
    public void A_rejected_reply_is_counted_and_raised()
    {
        var server = new HomaRpcServer(Echo.Pipeline());
        RpcTransportFailure? reported = null;
        server.TransportFailed += (_, failure) => reported = failure;

        server.ReportTransportFailure("reply", 9, new InvalidOperationException("send failed"));

        Assert.Equal(1, server.Pipeline.Metrics.ReplyFailures);
        Assert.NotNull(reported);
        Assert.Equal("reply", reported.Operation);
        Assert.Equal(9UL, reported.RpcId);
        Assert.Equal("send failed", reported.Exception.Message);
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

    private sealed class Probe : IDisposable
    {
        public bool Disposed { get; private set; }

        public string Value => Disposed ? throw new ObjectDisposedException(nameof(Probe)) : "live";

        public void Dispose() => Disposed = true;
    }

    private sealed class TouchResponse(Probe probe)
    {
        public string Text => probe.Value;
    }

    private sealed record TouchBody(string Text);

    private sealed class TestLifetime : IHostApplicationLifetime
    {
        private readonly ManualResetEventSlim _stop = new();

        public CancellationToken ApplicationStarted => CancellationToken.None;

        public CancellationToken ApplicationStopping => CancellationToken.None;

        public CancellationToken ApplicationStopped => CancellationToken.None;

        public ManualResetEventSlim Stop => _stop;

        public void StopApplication() => _stop.Set();
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
