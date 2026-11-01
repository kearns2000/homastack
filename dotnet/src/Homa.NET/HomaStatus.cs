namespace Homa;

/// <summary>
/// Status codes from <c>libhoma_ffi</c> ABI 1. Zero is success.
/// Numeric values match <c>homa_ffi.h</c> and are not reused for another meaning.
/// </summary>
public enum HomaStatus
{
    /// <summary>The call succeeded.</summary>
    Ok = 0,

    /// <summary>A pointer, length, flag or other argument was rejected.</summary>
    InvalidArgument = 1,

    /// <summary>This build of <c>libhoma_ffi</c> cannot create Homa sockets.</summary>
    UnsupportedPlatform = 2,

    /// <summary>The Homa kernel module is not available, or the native library did not load.</summary>
    Unavailable = 3,

    /// <summary>The address family or port cannot be used with Homa.</summary>
    InvalidEndpoint = 4,

    /// <summary>The wait reached its deadline.</summary>
    Timeout = 5,

    /// <summary>The wait was interrupted.</summary>
    Interrupted = 6,

    /// <summary>The kernel failed the RPC (for example after an abort or its own timeout).</summary>
    RpcFailed = 7,

    /// <summary>The socket was shut down or closed while a call was in progress.</summary>
    Shutdown = 8,

    /// <summary>The process could not allocate memory the transport needs.</summary>
    NoMemory = 9,

    /// <summary>A non-blocking receive found nothing queued.</summary>
    WouldBlock = 10,

    /// <summary>The server port is already taken.</summary>
    AddressInUse = 11,

    /// <summary>The process is not allowed to perform the operation.</summary>
    PermissionDenied = 12,

    /// <summary>No server is bound to the destination port.</summary>
    NoServer = 13,

    /// <summary>The destination could not be reached.</summary>
    Unreachable = 14,

    /// <summary>The payload is larger than <see cref="HomaSocket.MaxPayloadLength"/>.</summary>
    MessageTooLarge = 15,

    /// <summary>A caller buffer was too small. Used by the native last-error copy.</summary>
    BufferTooSmall = 16,

    /// <summary>An operating-system error that does not map to a more specific status.</summary>
    Io = 17,

    /// <summary>The native library hit an internal failure (including a caught panic).</summary>
    Internal = 99,
}
