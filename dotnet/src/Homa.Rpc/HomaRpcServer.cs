using System.Collections.Concurrent;
using System.Net;

namespace Homa.Rpc;

/// <summary>
/// Serves an <see cref="RpcPipeline"/> on a bound <see cref="Homa.HomaSocket"/>.
/// <see cref="DispatchAsync"/> also works with no socket, which is how the tests run.
/// </summary>
/// <remarks>
/// <para>
/// <see cref="RunAsync"/> receives shared requests and replies on the thread pool,
/// one task per request. Stopping cancels the token passed to handlers and calls
/// <see cref="Homa.HomaSocket.Shutdown"/> so <see cref="Homa.HomaSocket.RecvAsync"/> wakes.
/// A shared receive has no RPC id to abort, matching <c>Homa.NET</c>.
/// </para>
/// <para>
/// A client cancel aborts the caller's RPC. The handler may still be running;
/// <see cref="Homa.HomaSocket.Reply"/> then fails and the failure is ignored.
/// A module without <c>HOMAIOCABORT</c> cannot abort the caller either.
/// <see cref="Bind"/> and <see cref="RunAsync"/> have not been executed against <c>homa.ko</c>.
/// </para>
/// </remarks>
public sealed class HomaRpcServer : IDisposable, IAsyncDisposable
{
    private readonly Homa.HomaSocket? _socket;
    private readonly bool _ownsSocket;
    private readonly ConcurrentDictionary<Task, byte> _inflight = new();
    private int _disposed;

    /// <summary>Dispatch-only server. <see cref="RunAsync"/> throws until a socket is supplied.</summary>
    public HomaRpcServer(RpcPipeline pipeline)
    {
        ArgumentNullException.ThrowIfNull(pipeline);
        Pipeline = pipeline;
    }

    /// <summary>
    /// Uses <paramref name="socket"/> without taking ownership.
    /// The caller still closes the socket.
    /// </summary>
    public HomaRpcServer(Homa.HomaSocket socket, RpcPipeline pipeline)
        : this(pipeline)
    {
        ArgumentNullException.ThrowIfNull(socket);
        _socket = socket;
    }

    private HomaRpcServer(Homa.HomaSocket socket, RpcPipeline pipeline, bool ownsSocket)
        : this(socket, pipeline)
    {
        _ownsSocket = ownsSocket;
    }

    /// <summary>Pipeline this server dispatches to.</summary>
    public RpcPipeline Pipeline { get; }

    /// <summary>
    /// Binds a server socket and owns it. Needs Linux with <c>homa.ko</c>.
    /// This repository has not executed that bind.
    /// </summary>
    public static HomaRpcServer Bind(IPEndPoint local, RpcPipeline pipeline, Homa.HomaSocketOptions? options = null)
    {
        ArgumentNullException.ThrowIfNull(local);
        ArgumentNullException.ThrowIfNull(pipeline);
        var socket = Homa.HomaSocket.Bind(local, options);
        return new HomaRpcServer(socket, pipeline, ownsSocket: true);
    }

    /// <inheritdoc cref="RpcPipeline.DispatchAsync"/>
    public ValueTask<byte[]> DispatchAsync(
        ReadOnlyMemory<byte> frame,
        IPEndPoint? peer,
        CancellationToken cancellationToken = default) =>
        Pipeline.DispatchAsync(frame, peer, cancellationToken);

    /// <summary>
    /// Receives requests until <paramref name="cancellationToken"/> is cancelled,
    /// <see cref="Shutdown"/> is called, or the socket is closed.
    /// </summary>
    public async Task RunAsync(CancellationToken cancellationToken = default)
    {
        if (_socket is null)
            throw new InvalidOperationException("Bind a Homa socket before running the RPC server.");

        try
        {
            while (!cancellationToken.IsCancellationRequested)
            {
                Homa.HomaMessage incoming;
                try
                {
                    incoming = await _socket.RecvAsync(timeout: null, cancellationToken).ConfigureAwait(false);
                }
                catch (OperationCanceledException)
                {
                    break;
                }
                catch (Homa.HomaException ex) when (ex.Status is Homa.HomaStatus.Shutdown or Homa.HomaStatus.Interrupted)
                {
                    break;
                }
                catch (ObjectDisposedException)
                {
                    break;
                }

                var kind = incoming.Kind;
                var id = incoming.RpcId;
                var peer = incoming.Peer;
                var payload = incoming.CopyPayload();
                incoming.Dispose();
                if (kind != Homa.HomaMessageKind.Request)
                    continue;
                StartReply(id, peer, payload, cancellationToken);
            }
        }
        finally
        {
            try
            {
                await Task.WhenAll(_inflight.Keys.ToArray()).ConfigureAwait(false);
            }
            catch (Exception)
            {
                // DispatchAndReply already handles reply failures.
            }
        }
    }

    /// <summary>Wakes <see cref="RunAsync"/>. A server with no socket does nothing.</summary>
    public void Shutdown()
    {
        if (_socket is null)
            return;
        try
        {
            _socket.Shutdown();
        }
        catch (Homa.HomaException)
        {
        }
        catch (ObjectDisposedException)
        {
        }
    }

    /// <summary>Shuts the socket down and closes it when <see cref="Bind"/> created it.</summary>
    public void Dispose()
    {
        if (Interlocked.Exchange(ref _disposed, 1) != 0)
            return;
        Shutdown();
        if (_ownsSocket)
            _socket?.Dispose();
    }

    /// <inheritdoc cref="Dispose"/>
    public ValueTask DisposeAsync()
    {
        Dispose();
        return ValueTask.CompletedTask;
    }

    private void StartReply(ulong rpcId, IPEndPoint peer, byte[] payload, CancellationToken cancellationToken)
    {
        var socket = _socket;
        if (socket is null)
            return;
        var task = DispatchAndReply(socket, rpcId, peer, payload, cancellationToken);
        _inflight.TryAdd(task, 0);
        _ = task.ContinueWith(
            finished =>
            {
                _inflight.TryRemove(finished, out _);
                _ = finished.Exception;
            },
            CancellationToken.None,
            TaskContinuationOptions.ExecuteSynchronously,
            TaskScheduler.Default);
    }

    private async Task DispatchAndReply(
        Homa.HomaSocket socket,
        ulong rpcId,
        IPEndPoint peer,
        byte[] payload,
        CancellationToken cancellationToken)
    {
        byte[] response;
        try
        {
            response = await Pipeline.DispatchAsync(payload, peer, cancellationToken).ConfigureAwait(false);
        }
        catch (Exception ex)
        {
            response = RpcFrame.Encode(RpcFrameKind.Response, RpcStatus.HandlerFault, "", Describe(ex), []);
        }

        try
        {
            socket.Reply(rpcId, peer, response);
        }
        catch (Homa.HomaException)
        {
        }
        catch (ObjectDisposedException)
        {
        }
    }

    private static string Describe(Exception exception) =>
        string.IsNullOrEmpty(exception.Message) ? exception.GetType().Name : exception.Message;
}
