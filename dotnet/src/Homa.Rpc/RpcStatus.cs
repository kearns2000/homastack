namespace Homa.Rpc;

/// <summary>
/// Outcome of an RPC call above the Homa transport. Zero is success.
/// These values are written into the RPC frame. They are not <c>HomaStatus</c> codes.
/// </summary>
public enum RpcStatus
{
    /// <summary>The handler returned a response.</summary>
    Ok = 0,

    /// <summary>No handler is registered for the method name.</summary>
    MethodNotFound = 1,

    /// <summary>The handler threw, or the response could not be serialized.</summary>
    HandlerFault = 2,

    /// <summary>The call was cancelled. A local cancelled token surfaces as <see cref="OperationCanceledException"/> instead.</summary>
    Cancelled = 3,

    /// <summary>The frame or the payload could not be read.</summary>
    BadRequest = 4,

    /// <summary>The transport wait reached its deadline. This is not cancellation.</summary>
    Timeout = 5,

    /// <summary>The encoded frame would exceed <see cref="Homa.HomaSocket.MaxPayloadLength"/>.</summary>
    PayloadTooLarge = 6,

    /// <summary>The transport failed before a response frame arrived.</summary>
    Transport = 7,
}

/// <summary>
/// A failed RPC call. Transport failures are wrapped with
/// <see cref="RpcStatus.Transport"/> or <see cref="RpcStatus.Timeout"/> and
/// keep the <see cref="Homa.HomaException"/> as <see cref="Exception.InnerException"/>.
/// </summary>
public sealed class RpcException : Exception
{
    /// <summary>Creates an exception for <paramref name="status"/>.</summary>
    public RpcException(RpcStatus status, string? detail, Exception? innerException = null)
        : base(Format(status, detail), innerException)
    {
        Status = status;
        Detail = detail ?? "";
    }

    /// <summary>Frame or local status.</summary>
    public RpcStatus Status { get; }

    /// <summary>Human-readable explanation. May be empty. Not a stack trace.</summary>
    public string Detail { get; }

    private static string Format(RpcStatus status, string? detail)
    {
        var head = $"{status} ({(int)status})";
        return string.IsNullOrEmpty(detail) ? head : head + ": " + detail;
    }
}
