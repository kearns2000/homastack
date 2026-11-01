using Homa.Rpc;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;

namespace Homa.Rpc.Hosting;

/// <summary>
/// Runs <see cref="HomaRpcServer.RunAsync"/> for the application lifetime.
/// <see cref="StartAsync"/> is what opens the Homa socket.
/// </summary>
public sealed class HomaRpcServerHostedService : IHostedService, IDisposable
{
    private readonly RpcPipeline _pipeline;
    private readonly HomaRpcServerBinding _binding;
    private readonly ILogger<HomaRpcServerHostedService> _logger;
    private readonly CancellationTokenSource _stop = new();
    private HomaRpcServer? _server;
    private Task? _loop;
    private int _stopped;
    private int _disposed;

    /// <summary>Creates the service. The socket stays closed until <see cref="StartAsync"/>.</summary>
    public HomaRpcServerHostedService(
        RpcPipeline pipeline,
        HomaRpcServerBinding binding,
        ILogger<HomaRpcServerHostedService> logger)
    {
        ArgumentNullException.ThrowIfNull(pipeline);
        ArgumentNullException.ThrowIfNull(binding);
        ArgumentNullException.ThrowIfNull(logger);
        _pipeline = pipeline;
        _binding = binding;
        _logger = logger;
    }

    /// <summary>Binds the endpoint and starts the receive loop.</summary>
    public Task StartAsync(CancellationToken cancellationToken)
    {
        _server = HomaRpcServer.Bind(_binding.Listen, _pipeline, _binding.SocketOptions);
        _loop = _server.RunAsync(_stop.Token);
        _logger.LogInformation(
            "Homa RPC server listening on {Address}:{Port}.",
            _binding.Listen.Address,
            _binding.Listen.Port);
        return Task.CompletedTask;
    }

    /// <summary>Cancels handlers, wakes the receive, and closes a socket this service opened.</summary>
    public async Task StopAsync(CancellationToken cancellationToken)
    {
        if (Interlocked.Exchange(ref _stopped, 1) != 0)
            return;
        await _stop.CancelAsync().ConfigureAwait(false);
        _server?.Shutdown();
        if (_loop is not null)
        {
            try
            {
                await _loop.WaitAsync(cancellationToken).ConfigureAwait(false);
            }
            catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
            {
            }
        }

        _server?.Dispose();
        _logger.LogInformation("Homa RPC server stopped.");
    }

    /// <summary>Cancels and disposes the server if <see cref="StopAsync"/> did not run.</summary>
    public void Dispose()
    {
        if (Interlocked.Exchange(ref _disposed, 1) != 0)
            return;
        try
        {
            _stop.Cancel();
        }
        catch (ObjectDisposedException)
        {
        }

        _server?.Shutdown();
        _server?.Dispose();
        _stop.Dispose();
    }
}
