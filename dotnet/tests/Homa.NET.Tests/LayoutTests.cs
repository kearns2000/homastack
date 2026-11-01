using System.Net;
using System.Net.Sockets;
using Xunit;

namespace Homa.Tests;

public class LayoutTests
{
    [Fact]
    public void Managed_structs_match_homa_ffi_h() => NativeLayout.Check();

    [Fact]
    public void Ipv4_endpoint_round_trips()
    {
        var endpoint = new IPEndPoint(IPAddress.Parse("127.0.0.1"), 5000);
        var native = HomaEndpoint.From(endpoint);
        Assert.Equal(HomaEndpoint.IPv4, native.Family);
        Assert.Equal((ushort)5000, native.Port);
        Assert.Equal(0u, native.ScopeId);
        Assert.Equal((byte)127, native.Address[0]);
        Assert.Equal((byte)0, native.Address[1]);
        Assert.Equal((byte)0, native.Address[2]);
        Assert.Equal((byte)1, native.Address[3]);
        for (var i = 4; i < 16; i++)
            Assert.Equal((byte)0, native.Address[i]);
        Assert.Equal(endpoint, HomaEndpoint.ToEndPoint(in native));
    }

    [Fact]
    public void Ipv6_endpoint_keeps_scope_id()
    {
        var address = new IPAddress(new byte[]
        {
            0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
        }, 2);
        var endpoint = new IPEndPoint(address, 5000);
        var native = HomaEndpoint.From(endpoint);
        Assert.Equal(HomaEndpoint.IPv6, native.Family);
        Assert.Equal(2u, native.ScopeId);
        Assert.Equal((byte)0xfe, native.Address[0]);
        Assert.Equal((byte)0x80, native.Address[1]);
        Assert.Equal((byte)1, native.Address[15]);
        Assert.Equal(endpoint, HomaEndpoint.ToEndPoint(in native));
    }

    [Fact]
    public void Unix_family_is_rejected()
    {
        var ex = Assert.Throws<HomaException>(() => HomaEndpoint.FamilyCode(AddressFamily.Unix));
        Assert.Equal(HomaStatus.InvalidEndpoint, ex.Status);
    }

    [Fact]
    public void Server_flag_follows_bind_and_accept_requests()
    {
        var client = new HomaSocketOptions().ToNative(0, forceServer: false);
        Assert.Equal((uint)HomaSocketOptionsNative.Size, client.StructSize);
        Assert.Equal(HomaEndpoint.IPv4, client.Family);
        Assert.Equal(0u, client.Flags);
        Assert.Equal(0u, client.Reserved);

        var server = new HomaSocketOptions
        {
            AcceptRequests = false,
            ReceiveRegionBytes = 64UL * 1024 * 1024,
            Family = AddressFamily.InterNetworkV6,
        }.ToNative(5000, forceServer: true);
        Assert.Equal((ushort)5000, server.BindPort);
        Assert.Equal(HomaEndpoint.IPv6, server.Family);
        Assert.Equal(HomaSocketOptionsNative.ServerFlag, server.Flags);
        Assert.Equal(64UL * 1024 * 1024, server.ReceiveRegionBytes);
    }
}
