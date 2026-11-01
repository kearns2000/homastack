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
    private readonly IHostApplicationLifetime _lifetime;
    private readonly CancellationTokenSource _stop = new();
    private HomaRpcServer? _server;
    private Task? _loop;
    private int _stopped;
    private int _disposed;

    /// <summary>Creates the service. The socket stays closed until <see cref="StartAsync"/>.</summary>
    public HomaRpcServerHostedService(
        RpcPipeline pipeline,
        HomaRpcServerBinding binding,
        ILogger<HomaRpcServerHostedService> logger,
        IHostApplicationLifetime lifetime)
    {
        ArgumentNullException.ThrowIfNull(pipeline);
        ArgumentNullException.ThrowIfNull(binding);
        ArgumentNullException.ThrowIfNull(logger);
        ArgumentNullException.ThrowIfNull(lifetime);
        _pipeline = pipeline;
        _binding = binding;
        _logger = logger;
        _lifetime = lifetime;
    }

    /// <summary>Binds the endpoint and starts the receive loop.</summary>
    public Task StartAsync(CancellationToken cancellationToken)
    {
        _server = HomaRpcServer.Bind(
            _binding.Listen,
            _pipeline,
            _binding.SocketOptions,
            _binding.MaxInFlight);
        _server.TransportFailed += OnTransportFailed;
        _loop = _server.RunAsync(_stop.Token);
        ObserveLoop(_loop);
        _logger.LogInformation(
            "Homa RPC server listening on {Address}:{Port}.",
            _binding.Listen.Address,
            _binding.Listen.Port);
        return Task.CompletedTask;
    }

    /// <summary>
    /// When the receive loop faults, stop the host. Reply failures are logged
    /// from <see cref="HomaRpcServer.TransportFailed"/> and do not fault the loop.
    /// </summary>
    internal void ObserveLoop(Task loop)
    {
        ArgumentNullException.ThrowIfNull(loop);
        _ = loop.ContinueWith(
            finished =>
            {
                if (!finished.IsFaulted || Volatile.Read(ref _stopped) != 0)
                    return;
                _ = finished.Exception;
                _lifetime.StopApplication();
            },
            CancellationToken.None,
            TaskContinuationOptions.ExecuteSynchronously,
            TaskScheduler.Default);
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
            catch (Exception) when (_loop.IsFaulted)
            {
                // TransportFailed already logged the receive error.
            }
        }

        if (_server is not null)
            _server.TransportFailed -= OnTransportFailed;
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
        if (_server is not null)
            _server.TransportFailed -= OnTransportFailed;
        _server?.Dispose();
        _stop.Dispose();
    }

    private void OnTransportFailed(object? sender, RpcTransportFailure failure)
    {
        _logger.LogError(
            failure.Exception,
            "Homa RPC {Operation} failed for rpc {RpcId}.",
            failure.Operation,
            failure.RpcId);
    }
}
