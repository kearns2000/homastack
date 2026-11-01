using System.Diagnostics;
using Homa.Rpc;

namespace Homa.Rpc.Tests;

public class DispatchTests
{
    [Fact]
    public async Task Echo_round_trips_utf8()
    {
        var pipeline = Echo.Pipeline();
        var client = Echo.Client(pipeline);

        var reply = await client.CallAsync<string, string>(Echo.Peer, "echo", "héllo");

        Assert.Equal("héllo", reply);
        Assert.Equal(1, client.Metrics.ClientCalls);
        Assert.Equal(0, client.Metrics.ClientFailures);
        Assert.Equal(1, pipeline.Metrics.ServerCalls);
        Assert.Equal(0, pipeline.Metrics.ServerFailures);
    }

    [Fact]
    public async Task Unknown_method_is_a_client_and_server_failure()
    {
        var pipeline = Echo.Pipeline();
        var client = Echo.Client(pipeline);

        var ex = await Assert.ThrowsAsync<RpcException>(() =>
            client.CallAsync<string, string>(Echo.Peer, "missing", "x").AsTask());

        Assert.Equal(RpcStatus.MethodNotFound, ex.Status);
        Assert.Contains("missing", ex.Detail, StringComparison.Ordinal);
        Assert.Equal(1, client.Metrics.ClientFailures);
        Assert.Equal(1, pipeline.Metrics.ServerFailures);
    }

    [Fact]
    public async Task Handler_exception_becomes_handler_fault_without_a_stack()
    {
        var pipeline = new RpcPipelineBuilder()
            .Map<string, string>("boom", (_, _) => throw new InvalidOperationException("nope"))
            .Build(new BytesRpcSerializer());
        var client = Echo.Client(pipeline);

        var ex = await Assert.ThrowsAsync<RpcException>(() =>
            client.CallAsync<string, string>(Echo.Peer, "boom", "x").AsTask());

        Assert.Equal(RpcStatus.HandlerFault, ex.Status);
        Assert.Equal("nope", ex.Detail);
        Assert.DoesNotContain("at ", ex.Detail, StringComparison.Ordinal);
    }

    [Fact]
    public async Task Handler_cancellation_is_cancelled_when_the_caller_token_is_not()
    {
        var pipeline = new RpcPipelineBuilder()
            .Map<string, string>("stop", (_, _) => ValueTask.FromException<string>(new OperationCanceledException()))
            .Build(new BytesRpcSerializer());
        var client = Echo.Client(pipeline);

        var ex = await Assert.ThrowsAsync<RpcException>(() =>
            client.CallAsync<string, string>(Echo.Peer, "stop", "x").AsTask());

        Assert.Equal(RpcStatus.Cancelled, ex.Status);
    }

    [Fact]
    public async Task Already_cancelled_dispatch_skips_middleware_and_the_handler()
    {
        var steps = new List<string>();
        var ran = false;
        var pipeline = new RpcPipelineBuilder()
            .Use(new ProbeMiddleware("probe", steps))
            .Map<string, string>("echo", (text, _) =>
            {
                ran = true;
                return new ValueTask<string>(text);
            })
            .Build(new BytesRpcSerializer());
        var frame = RpcFrame.Encode(RpcFrameKind.Request, RpcStatus.Ok, "echo", "", "hi"u8);

        var response = await pipeline.DispatchAsync(frame, Echo.Peer, new CancellationToken(canceled: true));

        Assert.False(ran);
        Assert.Empty(steps);
        Assert.True(RpcFrame.TryDecode(response, out var decoded, out _));
        Assert.Equal(RpcStatus.Cancelled, decoded!.Status);
        Assert.Equal(1, pipeline.Metrics.ServerFailures);
    }

    [Fact]
    public async Task Middleware_runs_outside_in_and_can_short_circuit()
    {
        var steps = new List<string>();
        var builder = new RpcPipelineBuilder()
            .Use(new ProbeMiddleware("outer", steps))
            .Use(new ProbeMiddleware("inner", steps))
            .Map<string, string>("echo", (text, _) =>
            {
                steps.Add("handler:" + text);
                return new ValueTask<string>(text);
            });
        var pipeline = builder.Build(new BytesRpcSerializer());
        var client = Echo.Client(pipeline);

        var reply = await client.CallAsync<string, string>(Echo.Peer, "echo", "ping");

        Assert.Equal("ping", reply);
        Assert.Equal(["outer-before", "inner-before", "handler:ping", "inner-after", "outer-after"], steps);

        steps.Clear();
        var shortCircuit = new RpcPipelineBuilder()
            .Use(new ShortMiddleware(steps))
            .Map<string, string>("echo", (_, _) =>
            {
                steps.Add("handler");
                return new ValueTask<string>("no");
            })
            .Build(new BytesRpcSerializer());
        var shortClient = Echo.Client(shortCircuit);
        var replaced = await shortClient.CallAsync<string, string>(Echo.Peer, "echo", "ping");
        Assert.Equal("short", replaced);
        Assert.Equal(["short"], steps);
    }

    [Fact]
    public async Task Records_a_client_and_a_server_activity()
    {
        var seen = new List<(string Name, string? Method, string? Status)>();
        using var listener = new ActivityListener
        {
            ShouldListenTo = source => source.Name == RpcDiagnostics.SourceName,
            Sample = (ref ActivityCreationOptions<ActivityContext> _) => ActivitySamplingResult.AllDataAndRecorded,
            ActivityStopped = activity => seen.Add((
                activity.OperationName,
                activity.GetTagItem("rpc.method") as string,
                activity.GetTagItem("rpc.status") as string)),
        };
        ActivitySource.AddActivityListener(listener);
        var client = Echo.Client(Echo.Pipeline());

        await client.CallAsync<string, string>(Echo.Peer, "echo", "hi");

        Assert.Contains(seen, item => item.Name == "homa.rpc.client" && item.Method == "echo" && item.Status == nameof(RpcStatus.Ok));
        Assert.Contains(seen, item => item.Name == "homa.rpc.server" && item.Method == "echo" && item.Status == nameof(RpcStatus.Ok));
    }

    [Fact]
    public void Rejects_a_duplicate_or_empty_method()
    {
        var builder = new RpcPipelineBuilder()
            .Map<string, string>("echo", (text, _) => new ValueTask<string>(text));
        Assert.Throws<InvalidOperationException>(() =>
            builder.Map<string, string>("echo", (text, _) => new ValueTask<string>(text)));
        Assert.Throws<ArgumentException>(() =>
            builder.Map<string, string>("", (text, _) => new ValueTask<string>(text)));
    }

    [Fact]
    public async Task Json_serializer_round_trips_a_record()
    {
        var pipeline = new RpcPipelineBuilder()
            .Map<EchoBody, EchoBody>("echo", (body, _) => new ValueTask<EchoBody>(new EchoBody(body.Text)))
            .Build(new JsonRpcSerializer());
        var client = new HomaRpcClient(new PipelineCaller(pipeline), pipeline.Serializer);

        var reply = await client.CallAsync<EchoBody, EchoBody>(Echo.Peer, "echo", new EchoBody("hi"));

        Assert.Equal("hi", reply.Text);
        var encoded = pipeline.Serializer.Serialize(new EchoBody("hi"));
        Assert.Contains("\"text\":\"hi\"", EncodingOf(encoded), StringComparison.Ordinal);
    }

    private static string EncodingOf(byte[] utf8) => System.Text.Encoding.UTF8.GetString(utf8);

    private sealed record EchoBody(string Text);

    private sealed class ShortMiddleware(List<string> steps) : IRpcMiddleware
    {
        public ValueTask InvokeAsync(RpcInvocation invocation, RpcDispatchDelegate next)
        {
            steps.Add("short");
            invocation.Response = "short";
            invocation.Status = RpcStatus.Ok;
            return ValueTask.CompletedTask;
        }
    }
}
