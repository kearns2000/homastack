using System.Net;
using Xunit;

namespace Homa.Tests;

public class ValidationTests
{
    [Fact]
    public void Bind_rejects_port_zero_and_client_ports()
    {
        foreach (var port in new[] { 0, 32768, 40000 })
        {
            var ex = Assert.Throws<HomaException>(() =>
                HomaSocket.Bind(new IPEndPoint(IPAddress.Loopback, port)));
            Assert.Equal(HomaStatus.InvalidEndpoint, ex.Status);
        }
    }

    [Fact]
    public void Bind_rejects_a_null_endpoint()
    {
        Assert.Throws<ArgumentNullException>(() => HomaSocket.Bind(null!));
    }

    [Fact]
    public void Bind_rejects_a_family_mismatch()
    {
        var ex = Assert.Throws<HomaException>(() => HomaSocket.Bind(
            new IPEndPoint(IPAddress.IPv6Loopback, 4000),
            new HomaSocketOptions { Family = System.Net.Sockets.AddressFamily.InterNetwork }));
        Assert.Equal(HomaStatus.InvalidEndpoint, ex.Status);
    }

    [Fact]
    public void Open_rejects_a_port_above_32767_and_a_unix_family()
    {
        var port = Assert.Throws<HomaException>(() =>
            HomaSocket.Open(new HomaSocketOptions { BindPort = 32768 }));
        Assert.Equal(HomaStatus.InvalidEndpoint, port.Status);

        var family = Assert.Throws<HomaException>(() =>
            HomaSocket.Open(new HomaSocketOptions { Family = System.Net.Sockets.AddressFamily.Unix }));
        Assert.Equal(HomaStatus.InvalidEndpoint, family.Status);
    }

    [Fact]
    public void Empty_and_oversized_payloads_fail_before_a_socket_exists()
    {
        using var socket = HomaSocket.CreateForValidation();
        var destination = new IPEndPoint(IPAddress.Loopback, 1);

        var empty = Assert.Throws<HomaException>(() => socket.SendRequest(destination, ReadOnlySpan<byte>.Empty));
        Assert.Equal(HomaStatus.InvalidArgument, empty.Status);

        var huge = new byte[HomaSocket.MaxPayloadLength + 1];
        var tooLarge = Assert.Throws<HomaException>(() => socket.SendRequest(destination, huge));
        Assert.Equal(HomaStatus.MessageTooLarge, tooLarge.Status);

        Assert.Throws<ArgumentNullException>(() => socket.SendRequest(null!, new byte[] { 1 }));
        Assert.Throws<ArgumentNullException>(() => socket.Reply(null!, new byte[] { 1 }));
    }

    [Fact]
    public async Task Cancelled_token_fails_before_any_native_call()
    {
        using var socket = HomaSocket.CreateForValidation();
        using var cts = new CancellationTokenSource();
        cts.Cancel();
        var destination = new IPEndPoint(IPAddress.Loopback, 1);
        var payload = new ReadOnlyMemory<byte>(new byte[] { 1 });

        await Assert.ThrowsAsync<OperationCanceledException>(async () =>
            await socket.RequestAsync(destination, payload, cancellationToken: cts.Token));
        await Assert.ThrowsAsync<OperationCanceledException>(async () =>
            await socket.RecvResponseAsync(2, cancellationToken: cts.Token));
        await Assert.ThrowsAsync<OperationCanceledException>(async () =>
            await socket.RecvAsync(cancellationToken: cts.Token));
    }

    [Fact]
    public void Negative_timeout_is_rejected_and_sub_millisecond_rounds_up()
    {
        using var socket = HomaSocket.CreateForValidation();
        Assert.Throws<ArgumentOutOfRangeException>(() => socket.Recv(TimeSpan.FromTicks(-2)));
        Assert.Equal(-1, Timeouts.ToMilliseconds(null));
        Assert.Equal(-1, Timeouts.ToMilliseconds(Timeout.InfiniteTimeSpan));
        Assert.Equal(0, Timeouts.ToMilliseconds(TimeSpan.Zero));
        Assert.Equal(1, Timeouts.ToMilliseconds(TimeSpan.FromMicroseconds(500)));
        Assert.Equal(1500, Timeouts.ToMilliseconds(TimeSpan.FromMilliseconds(1500)));
    }

    [Fact]
    public void Disposing_an_unopened_socket_does_not_throw()
    {
        var socket = HomaSocket.CreateForValidation();
        socket.Dispose();
        socket.Dispose();
    }
}
