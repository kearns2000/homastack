using System.Net;
using System.Net.Sockets;

namespace Homa;

internal static class HomaEndpoint
{
    internal const ushort IPv4 = 4;
    internal const ushort IPv6 = 6;

    internal static ushort FamilyCode(AddressFamily family) => family switch
    {
        AddressFamily.InterNetwork => IPv4,
        AddressFamily.InterNetworkV6 => IPv6,
        _ => throw new HomaException(
            HomaStatus.InvalidEndpoint,
            0,
            $"address family {family} is not supported; use IPv4 or IPv6."),
    };

    internal static AddressFamily ToAddressFamily(ushort family) => family switch
    {
        IPv4 => AddressFamily.InterNetwork,
        IPv6 => AddressFamily.InterNetworkV6,
        _ => throw new HomaException(
            HomaStatus.InvalidEndpoint,
            0,
            $"address family {family} is not IPv4 or IPv6."),
    };

    internal static HomaEndpointNative From(IPEndPoint endpoint)
    {
        ArgumentNullException.ThrowIfNull(endpoint);
        if (endpoint.Port is < 0 or > 65535)
        {
            throw new HomaException(
                HomaStatus.InvalidEndpoint,
                0,
                "port is outside 0..=65535.");
        }

        var native = new HomaEndpointNative
        {
            Family = FamilyCode(endpoint.AddressFamily),
            Port = (ushort)endpoint.Port,
        };
        var bytes = endpoint.Address.GetAddressBytes();
        if (endpoint.AddressFamily == AddressFamily.InterNetwork)
        {
            if (bytes.Length != 4)
            {
                throw new HomaException(
                    HomaStatus.InvalidEndpoint,
                    0,
                    "IPv4 address must be 4 bytes.");
            }

            for (var i = 0; i < 4; i++)
                native.Address[i] = bytes[i];
            return native;
        }

        if (bytes.Length != 16)
        {
            throw new HomaException(
                HomaStatus.InvalidEndpoint,
                0,
                "IPv6 address must be 16 bytes.");
        }

        for (var i = 0; i < 16; i++)
            native.Address[i] = bytes[i];
        var scope = endpoint.Address.ScopeId;
        if (scope < 0 || scope > uint.MaxValue)
        {
            throw new HomaException(
                HomaStatus.InvalidEndpoint,
                0,
                "IPv6 scope id does not fit in uint32.");
        }

        native.ScopeId = (uint)scope;
        return native;
    }

    internal static IPEndPoint ToEndPoint(in HomaEndpointNative native)
    {
        if (native.Family == IPv4)
        {
            var bytes = new byte[4];
            for (var i = 0; i < 4; i++)
                bytes[i] = native.Address[i];
            return new IPEndPoint(new IPAddress(bytes), native.Port);
        }

        if (native.Family == IPv6)
        {
            var bytes = new byte[16];
            for (var i = 0; i < 16; i++)
                bytes[i] = native.Address[i];
            return new IPEndPoint(new IPAddress(bytes, native.ScopeId), native.Port);
        }

        throw new HomaException(
            HomaStatus.InvalidEndpoint,
            0,
            $"address family {native.Family} is not IPv4 or IPv6.");
    }
}
