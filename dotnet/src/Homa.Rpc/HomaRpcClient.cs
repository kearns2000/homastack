using System.Diagnostics;
using System.Net;

namespace Homa.Rpc;

/// <summary>
/// Calls one registered method on a remote <see cref="RpcPipeline"/>.
/// Serialization, the RPC frame, and translation of transport errors happen here.
/// The bytes move through <see cref="IRpcCaller"/>.
/// </summary>
/// <remarks>
/// A token that is already cancelled throws <see cref="OperationCanceledException"/>
/// before the caller is contacted. A token cancelled during
/// <see cref="HomaRpcCaller"/> aborts that private RPC. Timeout is
/// <see cref="RpcStatus.Timeout"/>, not cancellation.
/// A null timeout uses <see cref="DefaultCallTimeout"/>. <see cref="Timeout.InfiniteTimeSpan"/>
/// waits without a deadline. A cancelled token is not recorded in <see cref="Metrics"/>
/// until the call has started.
/// </remarks>
public sealed class HomaRpcClient : IDisposable, IAsyncDisposable
{
    /// <summary>
    /// Deadline used when <see cref="CallAsync{TRequest, TResponse}"/> is given a null timeout.
    /// Pass <see cref="Timeout.InfiniteTimeSpan"/> to wait without a deadline.
    /// </summary>
    public static readonly TimeSpan DefaultCallTimeout = TimeSpan.FromSeconds(30);

    private readonly IRpcCaller _caller;
    private readonly IRpcSerializer _serializer;
    private readonly bool _disposeCaller;

    /// <summary>Creates a client. <paramref name="disposeCaller"/> closes <paramref name="caller"/> when this client is disposed.</summary>
    public HomaRpcClient(IRpcCaller caller, IRpcSerializer serializer, RpcMetrics? metrics = null, bool disposeCaller = false)
    {
        ArgumentNullException.ThrowIfNull(caller);
        ArgumentNullException.ThrowIfNull(serializer);
        _caller = caller;
        _serializer = serializer;
        _disposeCaller = disposeCaller;
        Metrics = metrics ?? new RpcMetrics();
    }

    /// <summary>
    /// Opens a <see cref="HomaRpcCaller"/> and owns it. Needs Linux with <c>homa.ko</c>.
    /// <c>Homa.KernelSmoke</c> has done that once; see
    /// <c>tests/integration/results/20261110T224800Z-lima-homa.log</c>.
    /// </summary>
    public static HomaRpcClient Open(IRpcSerializer? serializer = null, Homa.HomaSocketOptions? options = null) =>
        new(HomaRpcCaller.Open(options), serializer ?? new JsonRpcSerializer(), disposeCaller: true);

    /// <summary>Serializer used for requests and responses.</summary>
    public IRpcSerializer Serializer => _serializer;

    /// <summary>Client-side counters for calls that started.</summary>
    public RpcMetrics Metrics { get; }

    /// <summary>
    /// Encodes <paramref name="request"/>, waits for a response frame, and decodes it.
    /// <paramref name="timeout"/> null uses <see cref="DefaultCallTimeout"/>.
    /// <see cref="Timeout.InfiniteTimeSpan"/> waits without a deadline.
    /// Zero is one non-blocking transport attempt.
    /// </summary>
    public async ValueTask<TResponse> CallAsync<TRequest, TResponse>(
        IPEndPoint destination,
        string method,
        TRequest request,
        TimeSpan? timeout = null,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(destination);
        method = RpcFrame.RequireMethod(method);
        var effectiveTimeout = timeout ?? DefaultCallTimeout;
        RpcFrame.CheckTimeout(effectiveTimeout);
        cancellationToken.ThrowIfCancellationRequested();

        var status = RpcStatus.Ok;
        var detail = "";
        using var activity = RpcDiagnostics.Start("homa.rpc.client", ActivityKind.Client, method, destination, peerIsServer: true);
        try
        {
            byte[] body;
            try
            {
                body = _serializer.Serialize(request);
            }
            catch (Exception ex) when (ex is not RpcException and not OperationCanceledException)
            {
                throw new RpcException(RpcStatus.BadRequest, ex.Message, ex);
            }

            var frame = RpcFrame.Encode(RpcFrameKind.Request, RpcStatus.Ok, method, "", body);
            ReadOnlyMemory<byte> responseBytes;
            try
            {
                responseBytes = await _caller.CallAsync(destination, frame, effectiveTimeout, cancellationToken)
                    .ConfigureAwait(false);
            }
            catch (Homa.HomaException ex) when (ex.Status == Homa.HomaStatus.Timeout)
            {
                throw new RpcException(RpcStatus.Timeout, ex.Detail, ex);
            }
            catch (Homa.HomaException ex)
            {
                throw new RpcException(RpcStatus.Transport, ex.Detail, ex);
            }

            if (!RpcFrame.TryDecode(responseBytes.Span, out var decoded, out var error))
                throw new RpcException(RpcStatus.BadRequest, error);
            if (decoded.Kind != RpcFrameKind.Response)
                throw new RpcException(RpcStatus.BadRequest, "RPC response kind is not a response.");
            if (decoded.Method.Length != 0 && !string.Equals(decoded.Method, method, StringComparison.Ordinal))
            {
                throw new RpcException(
                    RpcStatus.BadRequest,
                    $"RPC response method '{decoded.Method}' does not match '{method}'.");
            }

            if (decoded.Status == RpcStatus.Cancelled && cancellationToken.IsCancellationRequested)
                throw new OperationCanceledException(decoded.Detail, cancellationToken);
            if (decoded.Status != RpcStatus.Ok)
                throw new RpcException(decoded.Status, decoded.Detail);

            try
            {
                return _serializer.Deserialize<TResponse>(decoded.Payload);
            }
            catch (Exception ex) when (ex is not RpcException)
            {
                throw new RpcException(RpcStatus.BadRequest, ex.Message, ex);
            }
        }
        catch (Exception ex)
        {
            status = ex switch
            {
                RpcException rpc => rpc.Status,
                OperationCanceledException => RpcStatus.Cancelled,
                _ => RpcStatus.Transport,
            };
            detail = ex.Message;
            throw;
        }
        finally
        {
            RpcDiagnostics.Finish(activity, status, detail);
            Metrics.AddClient(status);
        }
    }

    /// <summary>Disposes <see cref="IRpcCaller"/> when this client was asked to own it.</summary>
    public void Dispose()
    {
        if (_disposeCaller && _caller is IDisposable disposable)
            disposable.Dispose();
    }

    /// <inheritdoc cref="Dispose"/>
    public ValueTask DisposeAsync()
    {
        Dispose();
        return ValueTask.CompletedTask;
    }
}
