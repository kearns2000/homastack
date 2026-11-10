using System.Net;
using System.Text;
using Homa;
using Homa.Rpc;

// Loopback smoke for a host that already has homa.ko. `dotnet test` does not
// start this program. scripts/run-integration.sh does, and writes the output
// into tests/integration/results/. One pass is recorded in
// tests/integration/results/20261110T224800Z-lima-homa.log.
const int transportPort = 7601;
const int rpcPort = 7602;
var timeout = TimeSpan.FromSeconds(10);

var transportPeer = new IPEndPoint(IPAddress.Loopback, transportPort);
using (var server = HomaSocket.Bind(transportPeer))
using (var client = new HomaClient())
{
    var accept = Task.Run(() =>
    {
        using var request = server.Recv(timeout);
        var text = Encoding.UTF8.GetString(request.CopyPayload());
        if (text != "ping")
            throw new InvalidOperationException($"transport request was '{text}', expected ping.");
        server.Reply(request, "pong:ping"u8);
    });

    using var response = client.Request(transportPeer, "ping"u8, timeout);
    var reply = Encoding.UTF8.GetString(response.CopyPayload());
    if (reply != "pong:ping")
        throw new InvalidOperationException($"transport reply was '{reply}', expected pong:ping.");
    await accept;
}

Console.WriteLine("transport: ping -> pong:ping");

var pipeline = new RpcPipelineBuilder()
    .Map<string, string>("echo", (text, _) => new ValueTask<string>("rpc:" + text))
    .Build(new BytesRpcSerializer());
var rpcPeer = new IPEndPoint(IPAddress.Loopback, rpcPort);
await using var rpcServer = HomaRpcServer.Bind(rpcPeer, pipeline);
using var stop = new CancellationTokenSource();
var loop = rpcServer.RunAsync(stop.Token);
try
{
    using var rpcClient = HomaRpcClient.Open(new BytesRpcSerializer());
    var echoed = await rpcClient.CallAsync<string, string>(rpcPeer, "echo", "hello", timeout);
    if (echoed != "rpc:hello")
        throw new InvalidOperationException($"rpc reply was '{echoed}', expected rpc:hello.");
}
finally
{
    await stop.CancelAsync();
    rpcServer.Shutdown();
    await loop.WaitAsync(timeout);
}

Console.WriteLine("rpc: echo hello -> rpc:hello");
