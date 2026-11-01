using System.Net;

namespace Homa;

/// <summary>
/// Whether a received message is an incoming request or a response.
/// Values match <c>HOMA_MESSAGE_REQUEST</c> and <c>HOMA_MESSAGE_RESPONSE</c>.
/// </summary>
public enum HomaMessageKind
{
    /// <summary>An incoming request. The RPC id is odd.</summary>
    Request = 1,

    /// <summary>A response to a request this socket sent.</summary>
    Response = 2,
}

/// <summary>
/// A message received from <c>libhoma_ffi</c>. The native stack has already
/// copied the payload out of the kernel receive region. <see cref="Payload"/>
/// views that native copy and is valid only until <see cref="Dispose"/>, and
/// only while this object is still reachable. <see cref="CopyPayload"/> is the
/// copy that outlives the message. Not zero-copy.
/// </summary>
/// <remarks>
/// Freeing is done by the handle exactly once. Do not pass the native pointer
/// to <c>free</c> or <c>homa_message_free</c>. A message is not safe to use
/// from two threads at once.
/// </remarks>
public sealed class HomaMessage : IDisposable
{
    private readonly HomaMessageHandle _handle;
    private readonly IntPtr _payload;
    private readonly int _length;

    private HomaMessage(HomaMessageHandle handle, in HomaMessageInfoNative info)
    {
        _handle = handle;
        Kind = info.Kind switch
        {
            (uint)HomaMessageKind.Request => HomaMessageKind.Request,
            (uint)HomaMessageKind.Response => HomaMessageKind.Response,
            _ => throw new HomaException(
                HomaStatus.Internal,
                0,
                $"native message kind {info.Kind} is not a request or a response."),
        };
        RpcId = info.RpcId;
        CompletionCookie = info.CompletionCookie;
        Peer = HomaEndpoint.ToEndPoint(in info.Peer);
        FragmentCount = checked((int)info.Fragments);
        if (info.PayloadLength > (ulong)HomaSocket.MaxPayloadLength)
        {
            throw new HomaException(
                HomaStatus.Internal,
                0,
                "native payload is longer than HOMA_FFI_MAX_MESSAGE_LENGTH.");
        }

        if (info.PayloadLength > 0 && info.Payload == IntPtr.Zero)
        {
            throw new HomaException(
                HomaStatus.Internal,
                0,
                "native message has a null payload.");
        }

        _payload = info.Payload;
        _length = (int)info.PayloadLength;
    }

    internal static HomaMessage Adopt(IntPtr raw)
    {
        if (raw == IntPtr.Zero)
        {
            throw new HomaException(
                HomaStatus.Internal,
                0,
                "native call returned a null message.");
        }

        var handle = HomaMessageHandle.Adopt(raw);
        try
        {
            var info = new HomaMessageInfoNative { StructSize = HomaMessageInfoNative.Size };
            var status = HomaNative.MessageInfo(handle, ref info);
            if (status != 0)
                throw HomaException.FromNative(status);
            return new HomaMessage(handle, in info);
        }
        catch
        {
            handle.Dispose();
            throw;
        }
    }

    /// <summary>Request or response.</summary>
    public HomaMessageKind Kind { get; }

    /// <summary>
    /// Kernel RPC id. Client-originated ids are even. The id on an incoming
    /// request is odd. Zero is the shared-wait selector and is not a real id.
    /// </summary>
    public ulong RpcId { get; }

    /// <summary>Cookie supplied with the request, echoed on the response.</summary>
    public ulong CompletionCookie { get; }

    /// <summary>Peer that sent this message.</summary>
    public IPEndPoint Peer { get; }

    /// <summary>How many kernel buffer pages the payload occupied.</summary>
    public int FragmentCount { get; }

    /// <summary>Payload length in bytes.</summary>
    public int PayloadLength => _length;

    /// <summary>
    /// Payload bytes. Valid until <see cref="Dispose"/> and only while this
    /// instance stays reachable. The bytes may contain NUL.
    /// </summary>
    public ReadOnlySpan<byte> Payload
    {
        get
        {
            ObjectDisposedException.ThrowIf(_handle.IsClosed, this);
            if (_length == 0 || _payload == IntPtr.Zero)
                return ReadOnlySpan<byte>.Empty;
            unsafe
            {
                return new ReadOnlySpan<byte>((void*)_payload, _length);
            }
        }
    }

    /// <summary>Copies <see cref="Payload"/> into a new array.</summary>
    public byte[] CopyPayload()
    {
        var copy = Payload.ToArray();
        GC.KeepAlive(this);
        return copy;
    }

    internal HomaMessageHandle Handle => _handle;

    internal bool IsClosed => _handle.IsClosed;

    /// <summary>Releases the native message. A second call does nothing.</summary>
    public void Dispose() => _handle.Dispose();

    /// <summary>Kind, RPC id, peer and length. Does not include payload bytes.</summary>
    public override string ToString() =>
        $"{Kind} rpc={RpcId} peer={Peer} bytes={_length}";
}
