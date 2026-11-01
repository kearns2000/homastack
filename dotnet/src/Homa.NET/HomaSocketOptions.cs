using System.Net.Sockets;

namespace Homa;

/// <summary>
/// Options for <see cref="HomaSocket.Open"/> and <see cref="HomaSocket.Bind"/>.
/// A default instance is IPv4, an automatic port, a 64 MiB receive region and
/// requests not accepted.
/// </summary>
public sealed class HomaSocketOptions
{
    /// <summary>IPv4 or IPv6. Other families are rejected.</summary>
    public AddressFamily Family { get; init; } = AddressFamily.InterNetwork;

    /// <summary>
    /// Port for <see cref="HomaSocket.Open"/>. 0 assigns a client port
    /// (32768 or higher). A server port is 1 through
    /// <see cref="HomaSocket.MaxServerPort"/>. <see cref="HomaSocket.Bind"/>
    /// takes the port from its endpoint and ignores this property.
    /// </summary>
    public ushort BindPort { get; init; }

    /// <summary>
    /// Receive-region size in bytes. 0 selects the native default (64 MiB).
    /// The kernel rejects regions that are too small for <c>nr_cpu_ids</c> or
    /// larger than 4 GiB.
    /// </summary>
    public ulong ReceiveRegionBytes { get; init; }

    /// <summary>
    /// When true, the socket accepts incoming requests (<c>HOMA_SOCKET_SERVER</c>).
    /// <see cref="HomaSocket.Bind"/> sets this even when the property is left false,
    /// because a bound Homa port is a server. <see cref="HomaSocket.Open"/> with a
    /// non-zero <see cref="BindPort"/> does the same.
    /// </summary>
    public bool AcceptRequests { get; init; }

    internal HomaSocketOptionsNative ToNative(ushort bindPort, bool forceServer)
    {
        var server = AcceptRequests || forceServer;
        return new HomaSocketOptionsNative
        {
            StructSize = HomaSocketOptionsNative.Size,
            Family = HomaEndpoint.FamilyCode(Family),
            BindPort = bindPort,
            ReceiveRegionBytes = ReceiveRegionBytes,
            Flags = server ? HomaSocketOptionsNative.ServerFlag : 0,
            Reserved = 0,
        };
    }
}
