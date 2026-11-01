using Homa.Rpc;

namespace Homa.Rpc.Tests;

public class ClientTests
{
    [Fact]
    public async Task Cancelled_token_does_not_call_the_transport()
    {
        var caller = new ScriptedCaller();
        var client = new HomaRpcClient(caller, new BytesRpcSerializer());

        await Assert.ThrowsAsync<OperationCanceledException>(() =>
            client.CallAsync<string, string>(Echo.Peer, "echo", "x", cancellationToken: new CancellationToken(canceled: true)).AsTask());

        Assert.Equal(0, caller.Calls);
        Assert.Equal(0, client.Metrics.ClientCalls);
    }

    [Fact]
    public async Task Negative_timeout_is_rejected_before_the_transport()
    {
        var caller = new ScriptedCaller();
        var client = new HomaRpcClient(caller, new BytesRpcSerializer());

        await Assert.ThrowsAsync<ArgumentOutOfRangeException>(() =>
            client.CallAsync<string, string>(Echo.Peer, "echo", "x", TimeSpan.FromMilliseconds(-2)).AsTask());

        Assert.Equal(0, caller.Calls);
    }

    [Fact]
    public async Task Passes_a_zero_timeout_through_and_rejects_a_bad_response_frame()
    {
        var caller = new ScriptedCaller
        {
            Handle = (_, _) => new ValueTask<ReadOnlyMemory<byte>>(new byte[] { 1, 2, 3 }),
        };
        var client = new HomaRpcClient(caller, new BytesRpcSerializer());

        var ex = await Assert.ThrowsAsync<RpcException>(() =>
            client.CallAsync<string, string>(Echo.Peer, "echo", "x", TimeSpan.Zero).AsTask());

        Assert.Equal(RpcStatus.BadRequest, ex.Status);
        Assert.Equal(1, caller.Calls);
        Assert.Equal(TimeSpan.Zero, caller.LastTimeout);
        Assert.Equal(1, client.Metrics.ClientFailures);
    }

    [Fact]
    public async Task Payload_over_the_homa_limit_fails_before_the_transport()
    {
        var caller = new ScriptedCaller();
        var client = new HomaRpcClient(caller, new BytesRpcSerializer());
        var payload = new string('a', Homa.HomaSocket.MaxPayloadLength);

        var ex = await Assert.ThrowsAsync<RpcException>(() =>
            client.CallAsync<string, string>(Echo.Peer, "echo", payload).AsTask());

        Assert.Equal(RpcStatus.PayloadTooLarge, ex.Status);
        Assert.Equal(0, caller.Calls);
        Assert.Equal(1, client.Metrics.ClientFailures);
    }

    [Fact]
    public async Task RunAsync_without_a_socket_does_not_open_one()
    {
        var server = new HomaRpcServer(Echo.Pipeline());

        var ex = await Assert.ThrowsAsync<InvalidOperationException>(() => server.RunAsync());

        Assert.Contains("socket", ex.Message, StringComparison.Ordinal);
        server.Shutdown();
        server.Dispose();
    }
}
