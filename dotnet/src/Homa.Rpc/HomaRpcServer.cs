using System.Net;

namespace Homa.Rpc;

/// <summary>A receive or reply the server could not complete.</summary>
public sealed class RpcTransportFailure : EventArgs
{
    /// <summary>Creates a failure report.</summary>
    public RpcTransportFailure(string operation, ulong rpcId, Exception exception)
    {
        ArgumentNullException.ThrowIfNull(operation);
        ArgumentNullException.ThrowIfNull(exception);
        Operation = operation;
        RpcId = rpcId;
        Exception = exception;
    }

    /// <summary><c>recv</c> or <c>reply</c>.</summary>
    public string Operation { get; }

    /// <summary>RPC id when <see cref="Operation"/> is <c>reply</c>. Zero for a receive failure.</summary>
    public ulong RpcId { get; }

    /// <summary>The transport exception.</summary>
    public Exception Exception { get; }
}

/// <summary>
/// Serves an <see cref="RpcPipeline"/> on a bound <see cref="Homa.HomaSocket"/>.
/// <see cref="DispatchAsync"/> also works with no socket, which is how the tests run.
/// </summary>
/// <remarks>
/// <para>
/// <see cref="RunAsync"/> receives shared requests. Each handler runs on the
/// thread pool, up to <see cref="DefaultMaxInFlight"/> at a time, so a synchronous
/// handler does not stall the next receive. Stopping cancels the token passed to
/// handlers and calls <see cref="Homa.HomaSocket.Shutdown"/> so
/// <see cref="Homa.HomaSocket.RecvAsync"/> wakes. A shared receive has no RPC id
/// to abort, matching <c>Homa.NET</c>.
/// </para>
/// <para>
/// A client cancel aborts the caller's RPC. The handler may still be running;
/// <see cref="Homa.HomaSocket.Reply"/> then fails, which increments
/// <see cref="RpcMetrics.ReplyFailures"/> and raises <see cref="TransportFailed"/>.
/// A module without <c>HOMAIOCABORT</c> cannot abort the caller either.
/// <see cref="Bind"/> and <see cref="RunAsync"/> have not been executed against <c>homa.ko</c>.
/// </para>
/// </remarks>
public sealed class HomaRpcServer : IDisposable, IAsyncDisposable
{
    /// <summary>How many handlers may run at once when a caller does not pass a limit.</summary>
    public const int DefaultMaxInFlight = 128;

    private readonly Homa.HomaSocket? _socket;
    private readonly bool _ownsSocket;
    private readonly RpcInFlightGate _gate;
    private int _disposed;

    /// <summary>Raised when a receive fails, or a reply is rejected by the transport.</summary>
    public event EventHandler<RpcTransportFailure>? TransportFailed;

    /// <summary>Dispatch-only server. <see cref="RunAsync"/> throws until a socket is supplied.</summary>
    public HomaRpcServer(RpcPipeline pipeline, int maxInFlight = DefaultMaxInFlight)
    {
        ArgumentNullException.ThrowIfNull(pipeline);
        Pipeline = pipeline;
        _gate = new RpcInFlightGate(maxInFlight);
    }

    /// <summary>
    /// Uses <paramref name="socket"/> without taking ownership.
    /// The caller still closes the socket.
    /// </summary>
    public HomaRpcServer(Homa.HomaSocket socket, RpcPipeline pipeline, int maxInFlight = DefaultMaxInFlight)
        : this(pipeline, maxInFlight)
    {
        ArgumentNullException.ThrowIfNull(socket);
        _socket = socket;
    }

    private HomaRpcServer(Homa.HomaSocket socket, RpcPipeline pipeline, bool ownsSocket, int maxInFlight)
        : this(socket, pipeline, maxInFlight)
    {
        _ownsSocket = ownsSocket;
    }

    /// <summary>Pipeline this server dispatches to.</summary>
    public RpcPipeline Pipeline { get; }

    /// <summary>
    /// Binds a server socket and owns it. Needs Linux with <c>homa.ko</c>.
    /// This repository has not executed that bind.
    /// </summary>
    public static HomaRpcServer Bind(
        IPEndPoint local,
        RpcPipeline pipeline,
        Homa.HomaSocketOptions? options = null,
        int maxInFlight = DefaultMaxInFlight)
    {
        ArgumentNullException.ThrowIfNull(local);
        ArgumentNullException.ThrowIfNull(pipeline);
        var socket = Homa.HomaSocket.Bind(local, options);
        return new HomaRpcServer(socket, pipeline, ownsSocket: true, maxInFlight);
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

        var socket = _socket;
        try
        {
            while (!cancellationToken.IsCancellationRequested)
            {
                Homa.HomaMessage incoming;
                try
                {
                    incoming = await socket.RecvAsync(timeout: null, cancellationToken).ConfigureAwait(false);
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
                catch (Exception ex)
                {
                    ReportTransportFailure("recv", 0, ex);
                    throw;
                }

                if (!await _gate.WaitAsync(cancellationToken).ConfigureAwait(false))
                {
                    incoming.Dispose();
                    break;
                }

                Homa.HomaMessageKind kind;
                ulong id;
                IPEndPoint peer;
                byte[] payload;
                try
                {
                    kind = incoming.Kind;
                    id = incoming.RpcId;
                    peer = incoming.Peer;
                    payload = incoming.CopyPayload();
                }
                catch (Exception ex)
                {
                    incoming.Dispose();
                    _gate.Release();
                    ReportTransportFailure("recv", 0, ex);
                    throw;
                }

                incoming.Dispose();
                if (kind != Homa.HomaMessageKind.Request)
                {
                    _gate.Release();
                    continue;
                }

                _gate.Run(() => DispatchAndReply(socket, id, peer, payload, cancellationToken));
            }
        }
        finally
        {
            await _gate.DrainAsync().ConfigureAwait(false);
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

    /// <summary>Counts a reply failure and raises <see cref="TransportFailed"/>.</summary>
    internal void ReportTransportFailure(string operation, ulong rpcId, Exception exception)
    {
        if (operation == "reply")
            Pipeline.Metrics.AddReplyFailure();
        TransportFailed?.Invoke(this, new RpcTransportFailure(operation, rpcId, exception));
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
        catch (Homa.HomaException ex)
        {
            ReportTransportFailure("reply", rpcId, ex);
        }
        catch (ObjectDisposedException)
        {
        }
    }

    private static string Describe(Exception exception) =>
        string.IsNullOrEmpty(exception.Message) ? exception.GetType().Name : exception.Message;
}
