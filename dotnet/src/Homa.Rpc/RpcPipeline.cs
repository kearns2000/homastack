using System.Diagnostics;
using System.Net;

namespace Homa.Rpc;

/// <summary>Activity source name <c>Homa.Rpc</c> for client and server calls.</summary>
public static class RpcDiagnostics
{
    /// <summary><see cref="ActivitySource"/> name.</summary>
    public const string SourceName = "Homa.Rpc";

    /// <summary>Source used by <see cref="HomaRpcClient"/> and <see cref="RpcPipeline"/>.</summary>
    public static readonly ActivitySource Source = new(SourceName, "1.0.0");

    internal static Activity? Start(string operationName, ActivityKind kind, string method, IPEndPoint? peer, bool peerIsServer)
    {
        var activity = Source.StartActivity(operationName, kind);
        if (activity is null)
            return null;
        activity.SetTag("rpc.system", "homa");
        activity.SetTag("rpc.method", method);
        if (peer is null)
            return activity;
        if (peerIsServer)
        {
            activity.SetTag("server.address", peer.Address.ToString());
            activity.SetTag("server.port", peer.Port);
        }
        else
        {
            activity.SetTag("client.address", peer.Address.ToString());
            activity.SetTag("client.port", peer.Port);
        }

        return activity;
    }

    internal static void Finish(Activity? activity, RpcStatus status, string? detail)
    {
        if (activity is null)
            return;
        activity.SetTag("rpc.status", status.ToString());
        if (status == RpcStatus.Ok)
            activity.SetStatus(ActivityStatusCode.Ok);
        else
            activity.SetStatus(ActivityStatusCode.Error, detail);
    }
}

/// <summary>
/// Counts calls on one client, one server, or both when the same instance is shared.
/// Safe to update from several threads.
/// </summary>
public sealed class RpcMetrics
{
    private long _clientCalls;
    private long _clientFailures;
    private long _serverCalls;
    private long _serverFailures;

    /// <summary>Client calls that finished, including failures.</summary>
    public long ClientCalls => Interlocked.Read(ref _clientCalls);

    /// <summary>Client calls whose status was not <see cref="RpcStatus.Ok"/>.</summary>
    public long ClientFailures => Interlocked.Read(ref _clientFailures);

    /// <summary>Server dispatches that finished, including rejected frames.</summary>
    public long ServerCalls => Interlocked.Read(ref _serverCalls);

    /// <summary>Server dispatches whose status was not <see cref="RpcStatus.Ok"/>.</summary>
    public long ServerFailures => Interlocked.Read(ref _serverFailures);

    internal void AddClient(RpcStatus status)
    {
        Interlocked.Increment(ref _clientCalls);
        if (status != RpcStatus.Ok)
            Interlocked.Increment(ref _clientFailures);
    }

    internal void AddServer(RpcStatus status)
    {
        Interlocked.Increment(ref _serverCalls);
        if (status != RpcStatus.Ok)
            Interlocked.Increment(ref _serverFailures);
    }
}

/// <summary>
/// Per-call state visible to handlers and middleware. One invocation does not
/// share a context with another. <see cref="Items"/> is for that call only.
/// </summary>
public sealed class RpcContext
{
    /// <summary>Creates a context for one call.</summary>
    public RpcContext(string method, IPEndPoint? peer, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(method);
        Method = method;
        Peer = peer;
        CancellationToken = cancellationToken;
    }

    /// <summary>Method name from the request frame.</summary>
    public string Method { get; }

    /// <summary>Remote endpoint when the transport supplied one.</summary>
    public IPEndPoint? Peer { get; }

    /// <summary>Cancels the handler when the server is stopping, or when the caller cancelled.</summary>
    public CancellationToken CancellationToken { get; }

    /// <summary>Scratch data for middleware on this call.</summary>
    public IDictionary<object, object?> Items { get; } = new Dictionary<object, object?>();
}

/// <summary>
/// The request currently moving through <see cref="IRpcMiddleware"/>.
/// Setting <see cref="Status"/> and skipping <c>next</c> short-circuits the handler.
/// A short-circuit <see cref="RpcStatus.Ok"/> must set <see cref="Response"/> to the response type.
/// </summary>
public sealed class RpcInvocation
{
    internal RpcInvocation(string method, RpcContext context, Type requestType, Type responseType)
    {
        Method = method;
        Context = context;
        RequestType = requestType;
        ResponseType = responseType;
    }

    /// <summary>Registered method name.</summary>
    public string Method { get; }

    /// <summary>Call context.</summary>
    public RpcContext Context { get; }

    /// <summary>CLR type the request payload was decoded as.</summary>
    public Type RequestType { get; }

    /// <summary>CLR type the response payload will be encoded as.</summary>
    public Type ResponseType { get; }

    /// <summary>Decoded request. Null only if the serializer produced null, which the built-in serializers reject.</summary>
    public object? Request { get; set; }

    /// <summary>Handler or middleware response. Required when <see cref="Status"/> stays <see cref="RpcStatus.Ok"/>.</summary>
    public object? Response { get; set; }

    /// <summary>Outcome. The terminal delegate sets this from the handler.</summary>
    public RpcStatus Status { get; set; } = RpcStatus.Ok;

    /// <summary>Failure text sent in the response frame.</summary>
    public string Detail { get; set; } = "";
}

/// <summary>Continues the rest of the pipeline, ending at the handler.</summary>
public delegate ValueTask RpcDispatchDelegate(RpcInvocation invocation);

/// <summary>
/// Wraps the handler. Middleware is created once and may run on overlapping calls.
/// The first middleware passed to <see cref="RpcPipelineBuilder"/> is the outermost.
/// </summary>
public interface IRpcMiddleware
{
    /// <summary>Runs around <paramref name="next"/>.</summary>
    ValueTask InvokeAsync(RpcInvocation invocation, RpcDispatchDelegate next);
}

/// <summary>
/// Registers methods and middleware, then freezes them into a <see cref="RpcPipeline"/>.
/// Not safe to share across threads while methods are still being added.
/// </summary>
public sealed class RpcPipelineBuilder
{
    private readonly Dictionary<string, RpcMethod> _methods = new(StringComparer.Ordinal);
    private readonly List<IRpcMiddleware> _middleware = [];
    private bool _built;

    /// <summary>Adds middleware. The first call is the outermost wrapper.</summary>
    public RpcPipelineBuilder Use(IRpcMiddleware middleware)
    {
        ArgumentNullException.ThrowIfNull(middleware);
        ThrowIfBuilt();
        _middleware.Add(middleware);
        return this;
    }

    /// <summary>Registers <paramref name="method"/>. A second registration of the same name throws.</summary>
    public RpcPipelineBuilder Map<TRequest, TResponse>(
        string method,
        Func<TRequest, RpcContext, ValueTask<TResponse>> handler)
    {
        ArgumentNullException.ThrowIfNull(handler);
        ThrowIfBuilt();
        method = RpcFrame.RequireMethod(method);
        var entry = new RpcMethod(
            typeof(TRequest),
            typeof(TResponse),
            static (serializer, payload) => serializer.Deserialize<TRequest>(payload),
            static (serializer, response) => serializer.Serialize((TResponse)response!),
            async (request, context) =>
            {
                var typed = (TRequest)request!;
                var response = await handler(typed, context).ConfigureAwait(false);
                return response;
            });
        if (!_methods.TryAdd(method, entry))
            throw new InvalidOperationException($"RPC method '{method}' is already registered.");
        return this;
    }

    /// <summary>
    /// Freezes the registrations. <paramref name="leadingMiddleware"/> runs outside
    /// middleware added with <see cref="Use"/>, still outermost first.
    /// </summary>
    public RpcPipeline Build(
        IRpcSerializer serializer,
        IReadOnlyList<IRpcMiddleware>? leadingMiddleware = null,
        RpcMetrics? metrics = null)
    {
        ArgumentNullException.ThrowIfNull(serializer);
        ThrowIfBuilt();
        _built = true;
        var leading = leadingMiddleware ?? Array.Empty<IRpcMiddleware>();
        var all = new IRpcMiddleware[leading.Count + _middleware.Count];
        for (var i = 0; i < leading.Count; i++)
            all[i] = leading[i];
        for (var i = 0; i < _middleware.Count; i++)
            all[leading.Count + i] = _middleware[i];
        var methods = new Dictionary<string, RpcMethod>(_methods, StringComparer.Ordinal);
        return new RpcPipeline(serializer, methods, all, metrics ?? new RpcMetrics());
    }

    private void ThrowIfBuilt()
    {
        if (_built)
            throw new InvalidOperationException("This RPC pipeline is already built.");
    }
}

/// <summary>
/// Decodes a request frame, runs middleware and the handler, and encodes the response frame.
/// Safe to call from several threads. Does not open a Homa socket.
/// </summary>
public sealed class RpcPipeline
{
    private readonly IRpcSerializer _serializer;
    private readonly Dictionary<string, RpcMethod> _methods;
    private readonly IRpcMiddleware[] _middleware;

    internal RpcPipeline(
        IRpcSerializer serializer,
        Dictionary<string, RpcMethod> methods,
        IRpcMiddleware[] middleware,
        RpcMetrics metrics)
    {
        _serializer = serializer;
        _methods = methods;
        _middleware = middleware;
        Metrics = metrics;
        Serializer = serializer;
    }

    /// <summary>Serializer used for every method on this pipeline.</summary>
    public IRpcSerializer Serializer { get; }

    /// <summary>Server-side counters.</summary>
    public RpcMetrics Metrics { get; }

    /// <summary>
    /// Handles one request frame. Failures come back as a response frame, not as thrown
    /// <see cref="RpcException"/>, so a server can still reply.
    /// A token that is already cancelled does not run middleware or the handler.
    /// </summary>
    public async ValueTask<byte[]> DispatchAsync(
        ReadOnlyMemory<byte> frame,
        IPEndPoint? peer,
        CancellationToken cancellationToken = default)
    {
        var status = RpcStatus.BadRequest;
        var methodName = "";
        var detail = "";
        using var activity = RpcDiagnostics.Start("homa.rpc.server", ActivityKind.Server, methodName, peer, peerIsServer: false);
        try
        {
            if (!RpcFrame.TryDecode(frame.Span, out var decoded, out var error))
            {
                detail = error;
                status = RpcStatus.BadRequest;
                return RpcFrame.Encode(RpcFrameKind.Response, status, methodName, detail, []);
            }

            methodName = decoded.Method;
            activity?.SetTag("rpc.method", methodName);
            if (decoded.Kind != RpcFrameKind.Request)
            {
                detail = "RPC server expected a request frame.";
                status = RpcStatus.BadRequest;
                return Fault(methodName, status, detail);
            }

            if (decoded.Method.Length == 0)
            {
                detail = "RPC method name is empty.";
                status = RpcStatus.BadRequest;
                return Fault(methodName, status, detail);
            }

            if (decoded.Status != RpcStatus.Ok)
            {
                detail = "RPC request status must be Ok.";
                status = RpcStatus.BadRequest;
                return Fault(methodName, status, detail);
            }

            if (!_methods.TryGetValue(decoded.Method, out var method))
            {
                detail = $"RPC method '{decoded.Method}' is not registered.";
                status = RpcStatus.MethodNotFound;
                return Fault(methodName, status, detail);
            }

            object? request;
            try
            {
                request = method.Read(_serializer, decoded.Payload);
            }
            catch (Exception ex)
            {
                detail = Describe(ex);
                status = ex is RpcException rpc ? rpc.Status : RpcStatus.BadRequest;
                return Fault(methodName, status, detail);
            }

            if (cancellationToken.IsCancellationRequested)
            {
                detail = "The request was cancelled.";
                status = RpcStatus.Cancelled;
                return Fault(methodName, status, detail);
            }

            var context = new RpcContext(decoded.Method, peer, cancellationToken);
            var invocation = new RpcInvocation(decoded.Method, context, method.RequestType, method.ResponseType)
            {
                Request = request,
            };
            await Invoke(method, invocation).ConfigureAwait(false);

            var payload = Array.Empty<byte>();
            if (invocation.Status == RpcStatus.Ok)
            {
                try
                {
                    payload = method.Write(_serializer, invocation.Response);
                }
                catch (Exception ex)
                {
                    invocation.Status = RpcStatus.HandlerFault;
                    invocation.Detail = Describe(ex);
                    payload = [];
                }
            }

            status = invocation.Status;
            detail = invocation.Detail;
            try
            {
                return RpcFrame.Encode(RpcFrameKind.Response, status, methodName, detail, payload);
            }
            catch (RpcException ex) when (ex.Status == RpcStatus.PayloadTooLarge)
            {
                status = RpcStatus.PayloadTooLarge;
                detail = ex.Detail;
                return Fault(methodName, status, detail);
            }
        }
        catch (Exception ex)
        {
            status = ex is OperationCanceledException ? RpcStatus.Cancelled : RpcStatus.HandlerFault;
            detail = Describe(ex);
            return Fault(methodName, status, detail);
        }
        finally
        {
            activity?.SetTag("rpc.method", methodName);
            RpcDiagnostics.Finish(activity, status, detail);
            Metrics.AddServer(status);
        }
    }

    private async ValueTask Invoke(RpcMethod method, RpcInvocation invocation)
    {
        RpcDispatchDelegate chain = async current =>
        {
            try
            {
                if (current.Context.CancellationToken.IsCancellationRequested)
                {
                    current.Status = RpcStatus.Cancelled;
                    current.Detail = "The request was cancelled.";
                    return;
                }

                current.Response = await method.Invoke(current.Request, current.Context).ConfigureAwait(false);
                current.Status = RpcStatus.Ok;
            }
            catch (OperationCanceledException)
            {
                current.Status = RpcStatus.Cancelled;
                current.Detail = "The handler was cancelled.";
            }
            catch (Exception ex)
            {
                current.Status = RpcStatus.HandlerFault;
                current.Detail = Describe(ex);
            }
        };

        for (var i = _middleware.Length - 1; i >= 0; i--)
        {
            var next = chain;
            var middleware = _middleware[i];
            chain = current => middleware.InvokeAsync(current, next);
        }

        try
        {
            await chain(invocation).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            invocation.Status = RpcStatus.Cancelled;
            if (invocation.Detail.Length == 0)
                invocation.Detail = "The request was cancelled.";
        }
        catch (Exception ex)
        {
            invocation.Status = RpcStatus.HandlerFault;
            invocation.Detail = Describe(ex);
        }
    }

    private static byte[] Fault(string method, RpcStatus status, string detail) =>
        RpcFrame.Encode(RpcFrameKind.Response, status, method, detail, []);

    private static string Describe(Exception exception)
    {
        if (exception is RpcException rpc && rpc.Detail.Length != 0)
            return rpc.Detail;
        return string.IsNullOrEmpty(exception.Message) ? exception.GetType().Name : exception.Message;
    }
}

internal sealed class RpcMethod
{
    public RpcMethod(
        Type requestType,
        Type responseType,
        Func<IRpcSerializer, ReadOnlySpan<byte>, object?> read,
        Func<IRpcSerializer, object?, byte[]> write,
        Func<object?, RpcContext, ValueTask<object?>> invoke)
    {
        RequestType = requestType;
        ResponseType = responseType;
        Read = read;
        Write = write;
        Invoke = invoke;
    }

    public Type RequestType { get; }

    public Type ResponseType { get; }

    public Func<IRpcSerializer, ReadOnlySpan<byte>, object?> Read { get; }

    public Func<IRpcSerializer, object?, byte[]> Write { get; }

    public Func<object?, RpcContext, ValueTask<object?>> Invoke { get; }
}
