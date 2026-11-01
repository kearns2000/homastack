using System.Net;

namespace Homa;

/// <summary>
/// A Homa socket backed by <c>libhoma_ffi</c>. Safe to share across threads.
/// At most one thread may wait on a given private RPC id; a second waiter
/// gets <see cref="HomaStatus.InvalidArgument"/> and does not cancel the first
/// wait. Several threads may call <see cref="Recv"/>; each shared message is
/// delivered to one of them.
/// </summary>
/// <remarks>
/// <para>
/// Sockets exist on Linux when the <c>homa</c> kernel module is loaded.
/// Elsewhere <see cref="Open"/> fails with
/// <see cref="HomaStatus.UnsupportedPlatform"/>. Linux without the module
/// fails with <see cref="HomaStatus.Unavailable"/>. This repository has not
/// executed a live RPC through this wrapper.
/// </para>
/// <para>
/// <see cref="RequestAsync"/> and <see cref="RecvResponseAsync"/> block a
/// thread-pool thread for the wait (one wait per call, so different private
/// ids proceed concurrently). Cancelling the token calls <c>homa_abort_rpc</c>
/// for that id. <c>homa_request</c> is not used on that path because it does
/// not return the id until the response arrives. A module built with upstream
/// <c>strip.py</c> has no <c>HOMAIOCABORT</c>, so the abort returns
/// <see cref="HomaStatus.InvalidArgument"/> and the wait continues until the
/// kernel times the RPC out or the socket is shut down.
/// <see cref="RecvAsync"/> has no single id to abort; an already-cancelled
/// token is observed before the wait, and a later cancel does not interrupt
/// it. <see cref="Shutdown"/> wakes shared receivers.
/// </para>
/// <para>
/// A timeout of zero on a private wait (<see cref="RecvResponse"/> and
/// <see cref="Request"/>) is one attempt and then an abort of that RPC.
/// The same zero timeout on <see cref="Recv"/> returns
/// <see cref="HomaStatus.WouldBlock"/> and aborts nothing.
/// </para>
/// </remarks>
public sealed class HomaSocket : IDisposable, IAsyncDisposable
{
    /// <summary>ABI of <c>libhoma_ffi</c> this wrapper was built against.</summary>
    public const uint AbiVersion = 1;

    /// <summary>
    /// Largest payload Homa accepts (<c>HOMA_FFI_MAX_MESSAGE_LENGTH</c>).
    /// </summary>
    public const int MaxPayloadLength = 1_000_000;

    /// <summary>Highest port <see cref="Bind"/> accepts. Ports above this are client ports.</summary>
    public const int MaxServerPort = 32767;

    private const uint RequestPrivate = 1;

    private readonly HomaSocketHandle _handle;
    private ushort _localPort;

    private HomaSocket(HomaSocketHandle handle) => _handle = handle;

    /// <summary>
    /// A socket that has not been opened. Argument checks run against it
    /// without calling into the native library.
    /// </summary>
    internal static HomaSocket CreateForValidation() => new(new HomaSocketHandle());

    /// <summary>
    /// True when this build of <c>libhoma_ffi</c> can create sockets (Linux).
    /// The kernel module is checked only by <see cref="Open"/>.
    /// </summary>
    public static bool IsPlatformSupported => HomaNative.Guard(HomaNative.PlatformSupported);

    /// <summary>ABI version reported by the loaded <c>libhoma_ffi</c>.</summary>
    public static uint NativeAbiVersion => HomaNative.Guard(HomaNative.AbiVersion);

    /// <summary>Maximum payload reported by the loaded <c>libhoma_ffi</c>.</summary>
    public static uint NativeMaxMessageLength => HomaNative.Guard(HomaNative.MaxMessageLength);

    /// <summary>
    /// Opens a socket. <paramref name="options"/> null uses the defaults in
    /// <see cref="HomaSocketOptions"/>.
    /// </summary>
    public static HomaSocket Open(HomaSocketOptions? options = null)
    {
        options ??= new HomaSocketOptions();
        if (options.BindPort > MaxServerPort)
        {
            throw new HomaException(
                HomaStatus.InvalidEndpoint,
                0,
                $"bind port {options.BindPort} is outside 0..={MaxServerPort} (0 assigns a client port).");
        }

        var native = options.ToNative(options.BindPort, forceServer: options.BindPort != 0);
        return OpenCore(in native, bindPort: null);
    }

    /// <summary>
    /// Opens a socket bound to <paramref name="local"/>'s port (1 through
    /// <see cref="MaxServerPort"/>) and marks it as a server. Homa ignores the
    /// IP; the socket receives on every interface of that family. When
    /// <paramref name="options"/> is null the family is taken from
    /// <paramref name="local"/> and requests are accepted.
    /// </summary>
    public static HomaSocket Bind(IPEndPoint local, HomaSocketOptions? options = null)
    {
        ArgumentNullException.ThrowIfNull(local);
        if (local.Port is < 1 or > MaxServerPort)
        {
            throw new HomaException(
                HomaStatus.InvalidEndpoint,
                0,
                $"server port {local.Port} is outside 1..={MaxServerPort}.");
        }

        var effective = options ?? new HomaSocketOptions
        {
            Family = local.AddressFamily,
            AcceptRequests = true,
        };
        if (options is not null && options.Family != local.AddressFamily)
        {
            throw new HomaException(
                HomaStatus.InvalidEndpoint,
                0,
                "HomaSocketOptions.Family does not match the bind endpoint.");
        }

        var native = effective.ToNative((ushort)local.Port, forceServer: true);
        return OpenCore(in native, bindPort: (ushort)local.Port);
    }

    /// <summary>Local Homa port. Bound, or assigned by the kernel (32768 or higher).</summary>
    public ushort LocalPort
    {
        get
        {
            if (_localPort != 0)
                return _localPort;
            Prepare();
            ushort port;
            int status;
            try
            {
                status = HomaNative.LocalPort(_handle, out port);
            }
            catch (Exception ex) when (HomaNative.IsLoadFailure(ex))
            {
                throw HomaNative.LibraryMissing(ex);
            }

            if (status != 0)
                throw HomaException.FromNative(status);
            _localPort = port;
            return port;
        }
    }

    /// <summary>Diagnostic snapshot. The port is the one just reported by the kernel.</summary>
    public HomaSocketInfo GetInfo()
    {
        Prepare();
        var info = new HomaSocketInfoNative { StructSize = HomaSocketInfoNative.Size };
        int status;
        try
        {
            status = HomaNative.SocketInfo(_handle, ref info);
        }
        catch (Exception ex) when (HomaNative.IsLoadFailure(ex))
        {
            throw HomaNative.LibraryMissing(ex);
        }

        if (status != 0)
            throw HomaException.FromNative(status);
        return new HomaSocketInfo(
            info.Port,
            HomaEndpoint.ToAddressFamily(info.Family),
            info.ActiveRpcs,
            info.PendingBpages,
            info.BufferAvailableBytes,
            info.ReceiveRegionBytes);
    }

    /// <summary>
    /// Sends a request and returns its RPC id without waiting.
    /// <paramref name="privateResponse"/> asks for a response that only
    /// <see cref="RecvResponse"/> will observe. Otherwise the response arrives
    /// through <see cref="Recv"/>.
    /// </summary>
    public ulong SendRequest(
        IPEndPoint destination,
        ReadOnlySpan<byte> payload,
        ulong completionCookie = 0,
        bool privateResponse = true)
    {
        ArgumentNullException.ThrowIfNull(destination);
        PayloadRules.Check(payload);
        Prepare();
        var dest = HomaEndpoint.From(destination);
        var flags = privateResponse ? RequestPrivate : 0u;
        ulong id;
        int status;
        try
        {
            status = HomaNative.SendRequest(
                _handle,
                in dest,
                payload,
                (ulong)payload.Length,
                completionCookie,
                flags,
                out id);
        }
        catch (Exception ex) when (HomaNative.IsLoadFailure(ex))
        {
            throw HomaNative.LibraryMissing(ex);
        }

        if (status != 0)
            throw HomaException.FromNative(status);
        return id;
    }

    /// <summary>
    /// Waits for the response to a private request.
    /// A second waiter on the same id, or an id that was not sent as private,
    /// fails with <see cref="HomaStatus.InvalidArgument"/> and does not abort
    /// the live RPC. A timeout aborts it. Zero is one attempt, then an abort.
    /// </summary>
    public HomaMessage RecvResponse(ulong rpcId, TimeSpan? timeout = null)
    {
        var milliseconds = Timeouts.ToMilliseconds(timeout);
        Prepare();
        IntPtr raw;
        int status;
        try
        {
            status = HomaNative.RecvResponse(_handle, rpcId, milliseconds, out raw);
        }
        catch (Exception ex) when (HomaNative.IsLoadFailure(ex))
        {
            throw HomaNative.LibraryMissing(ex);
        }

        return TakeMessage(status, raw);
    }

    /// <summary>
    /// Private request and wait, as one native call. Cancellation is not
    /// possible here because the RPC id is not known until the call returns;
    /// use <see cref="RequestAsync"/> when a token must abort the RPC.
    /// </summary>
    public HomaMessage Request(IPEndPoint destination, ReadOnlySpan<byte> payload, TimeSpan? timeout = null)
    {
        ArgumentNullException.ThrowIfNull(destination);
        PayloadRules.Check(payload);
        var milliseconds = Timeouts.ToMilliseconds(timeout);
        Prepare();
        var dest = HomaEndpoint.From(destination);
        IntPtr raw;
        int status;
        try
        {
            status = HomaNative.Request(
                _handle,
                in dest,
                payload,
                (ulong)payload.Length,
                milliseconds,
                out raw);
        }
        catch (Exception ex) when (HomaNative.IsLoadFailure(ex))
        {
            throw HomaNative.LibraryMissing(ex);
        }

        return TakeMessage(status, raw);
    }

    /// <summary>
    /// Next shared message: an incoming request, or the response to a
    /// non-private request. Zero returns <see cref="HomaStatus.WouldBlock"/>
    /// when nothing is queued. A positive timeout returns
    /// <see cref="HomaStatus.Timeout"/> without aborting anything.
    /// </summary>
    public HomaMessage Recv(TimeSpan? timeout = null)
    {
        var milliseconds = Timeouts.ToMilliseconds(timeout);
        Prepare();
        IntPtr raw;
        int status;
        try
        {
            status = HomaNative.Recv(_handle, milliseconds, out raw);
        }
        catch (Exception ex) when (HomaNative.IsLoadFailure(ex))
        {
            throw HomaNative.LibraryMissing(ex);
        }

        return TakeMessage(status, raw);
    }

    /// <summary>
    /// <see cref="Recv"/> with a zero timeout. Returns null on
    /// <see cref="HomaStatus.WouldBlock"/>.
    /// </summary>
    public HomaMessage? TryRecv()
    {
        try
        {
            return Recv(TimeSpan.Zero);
        }
        catch (HomaException ex) when (ex.Status == HomaStatus.WouldBlock)
        {
            return null;
        }
    }

    /// <summary>Replies to <paramref name="rpcId"/> at <paramref name="peer"/>.</summary>
    public void Reply(ulong rpcId, IPEndPoint peer, ReadOnlySpan<byte> payload)
    {
        ArgumentNullException.ThrowIfNull(peer);
        PayloadRules.Check(payload);
        Prepare();
        var dest = HomaEndpoint.From(peer);
        int status;
        try
        {
            status = HomaNative.Reply(
                _handle,
                rpcId,
                in dest,
                payload,
                (ulong)payload.Length);
        }
        catch (Exception ex) when (HomaNative.IsLoadFailure(ex))
        {
            throw HomaNative.LibraryMissing(ex);
        }

        if (status != 0)
            throw HomaException.FromNative(status);
    }

    /// <summary>Replies using the id and peer of a received request.</summary>
    public void Reply(HomaMessage request, ReadOnlySpan<byte> payload)
    {
        ArgumentNullException.ThrowIfNull(request);
        if (request.Kind != HomaMessageKind.Request)
        {
            throw new HomaException(
                HomaStatus.InvalidArgument,
                0,
                "Reply expects a request message.");
        }

        PayloadRules.Check(payload);
        Prepare();
        ObjectDisposedException.ThrowIf(request.IsClosed, request);
        int status;
        try
        {
            status = HomaNative.ReplyToMessage(
                _handle,
                request.Handle,
                payload,
                (ulong)payload.Length);
        }
        catch (Exception ex) when (HomaNative.IsLoadFailure(ex))
        {
            throw HomaNative.LibraryMissing(ex);
        }

        if (status != 0)
            throw HomaException.FromNative(status);
    }

    /// <summary>
    /// Aborts one outgoing RPC, or every outgoing RPC when <paramref name="rpcId"/>
    /// is 0. An unknown id returns <see cref="HomaStatus.InvalidArgument"/>.
    /// On a module built with <c>strip.py</c> every abort returns that status
    /// and the RPC stays until the kernel's own timeout.
    /// </summary>
    public void Abort(ulong rpcId = 0)
    {
        Prepare();
        int status;
        try
        {
            status = HomaNative.Abort(_handle, rpcId);
        }
        catch (Exception ex) when (HomaNative.IsLoadFailure(ex))
        {
            throw HomaNative.LibraryMissing(ex);
        }

        if (status != 0)
            throw HomaException.FromNative(status);
    }

    /// <summary>
    /// Wakes blocked receivers with <see cref="HomaStatus.Shutdown"/>.
    /// The handle stays open until <see cref="Dispose"/>.
    /// </summary>
    public void Shutdown()
    {
        Prepare();
        int status;
        try
        {
            status = HomaNative.Shutdown(_handle);
        }
        catch (Exception ex) when (HomaNative.IsLoadFailure(ex))
        {
            throw HomaNative.LibraryMissing(ex);
        }

        if (status != 0)
            throw HomaException.FromNative(status);
    }

    /// <summary>
    /// Sends a private request and waits for the response on a thread-pool thread.
    /// Cancelling <paramref name="cancellationToken"/> aborts that RPC id.
    /// </summary>
    public async ValueTask<HomaMessage> RequestAsync(
        IPEndPoint destination,
        ReadOnlyMemory<byte> payload,
        TimeSpan? timeout = null,
        CancellationToken cancellationToken = default)
    {
        cancellationToken.ThrowIfCancellationRequested();
        ArgumentNullException.ThrowIfNull(destination);
        PayloadRules.Check(payload.Span);
        var id = SendRequest(destination, payload.Span, completionCookie: 0, privateResponse: true);
        if (cancellationToken.IsCancellationRequested)
        {
            TryAbort(id);
            throw new OperationCanceledException(cancellationToken);
        }

        return await WaitAsync(
            () => RecvResponse(id, timeout),
            id,
            cancellationToken).ConfigureAwait(false);
    }

    /// <summary>
    /// <see cref="RecvResponse"/> on a thread-pool thread. Cancelling
    /// <paramref name="cancellationToken"/> aborts <paramref name="rpcId"/>.
    /// </summary>
    public async ValueTask<HomaMessage> RecvResponseAsync(
        ulong rpcId,
        TimeSpan? timeout = null,
        CancellationToken cancellationToken = default)
    {
        cancellationToken.ThrowIfCancellationRequested();
        return await WaitAsync(
            () => RecvResponse(rpcId, timeout),
            rpcId,
            cancellationToken).ConfigureAwait(false);
    }

    /// <summary>
    /// <see cref="Recv"/> on a thread-pool thread. A token that is already
    /// cancelled fails immediately. A cancel that arrives during the wait
    /// does not interrupt it; call <see cref="Shutdown"/> to wake shared receivers.
    /// </summary>
    public async ValueTask<HomaMessage> RecvAsync(
        TimeSpan? timeout = null,
        CancellationToken cancellationToken = default)
    {
        cancellationToken.ThrowIfCancellationRequested();
        return await Task.Run(() => Recv(timeout), CancellationToken.None).ConfigureAwait(false);
    }

    /// <summary>Releases the native socket. In-flight calls fail with <see cref="HomaStatus.Shutdown"/> when close can wake them.</summary>
    public void Close() => _handle.Dispose();

    /// <inheritdoc cref="Close"/>
    public void Dispose() => Close();

    /// <inheritdoc cref="Close"/>
    public ValueTask DisposeAsync()
    {
        Dispose();
        return ValueTask.CompletedTask;
    }

    private async ValueTask<HomaMessage> WaitAsync(
        Func<HomaMessage> wait,
        ulong rpcId,
        CancellationToken cancellationToken)
    {
        using var registration = cancellationToken.CanBeCanceled
            ? cancellationToken.Register(static state =>
            {
                var (socket, id) = ((HomaSocket Socket, ulong Id))state!;
                socket.TryAbort(id);
            }, (this, rpcId))
            : default;

        HomaMessage message;
        try
        {
            message = await Task.Run(wait, CancellationToken.None).ConfigureAwait(false);
        }
        catch (HomaException ex) when (cancellationToken.IsCancellationRequested && IsCancellationOutcome(ex.Status))
        {
            throw new OperationCanceledException("The Homa request was cancelled.", ex, cancellationToken);
        }

        if (cancellationToken.IsCancellationRequested)
        {
            message.Dispose();
            throw new OperationCanceledException(cancellationToken);
        }

        return message;
    }

    private void TryAbort(ulong rpcId)
    {
        try
        {
            if (_handle.IsClosed || _handle.IsInvalid)
                return;
            _ = HomaNative.Abort(_handle, rpcId);
        }
        catch (Exception ex) when (ex is ObjectDisposedException or DllNotFoundException or BadImageFormatException)
        {
        }
    }

    private static bool IsCancellationOutcome(HomaStatus status) =>
        status is HomaStatus.RpcFailed
            or HomaStatus.InvalidArgument
            or HomaStatus.Io
            or HomaStatus.Shutdown
            or HomaStatus.Interrupted;

    private static HomaSocket OpenCore(in HomaSocketOptionsNative native, ushort? bindPort)
    {
        HomaNative.EnsureAbi();
        IntPtr raw;
        int status;
        try
        {
            status = bindPort is ushort port
                ? HomaNative.SocketBind(in native, port, out raw)
                : HomaNative.SocketOpen(in native, out raw);
        }
        catch (Exception ex) when (HomaNative.IsLoadFailure(ex))
        {
            throw HomaNative.LibraryMissing(ex);
        }

        if (status != 0)
            throw HomaException.FromNative(status);
        if (raw == IntPtr.Zero)
        {
            throw new HomaException(
                HomaStatus.Internal,
                0,
                "homa_socket_open returned a null handle.");
        }

        return new HomaSocket(HomaSocketHandle.Adopt(raw));
    }

    private HomaMessage TakeMessage(int status, IntPtr raw)
    {
        if (status != 0)
        {
            var error = HomaException.FromNative(status);
            if (raw != IntPtr.Zero)
                HomaNative.MessageFree(raw);
            throw error;
        }

        return HomaMessage.Adopt(raw);
    }

    private void Prepare()
    {
        ObjectDisposedException.ThrowIf(_handle.IsClosed, this);
        if (_handle.IsInvalid)
        {
            throw new HomaException(
                HomaStatus.InvalidArgument,
                0,
                "Homa socket is not open.");
        }

        HomaNative.EnsureAbi();
    }
}
