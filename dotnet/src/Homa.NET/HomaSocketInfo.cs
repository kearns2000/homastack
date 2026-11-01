using System.Net.Sockets;

namespace Homa;

/// <summary>
/// Diagnostic snapshot from <c>homa_socket_info</c>.
/// <see cref="PendingBufferPages"/> is the number of receive pages the
/// application still owns (<c>pending_bpages</c>).
/// </summary>
public readonly record struct HomaSocketInfo(
    ushort Port,
    AddressFamily Family,
    uint ActiveRpcs,
    uint PendingBufferPages,
    ulong BufferAvailableBytes,
    ulong ReceiveRegionBytes);
