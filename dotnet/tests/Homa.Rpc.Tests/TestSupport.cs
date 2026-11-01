using System.Net;
using Homa.Rpc;

namespace Homa.Rpc.Tests;

internal sealed class PipelineCaller(RpcPipeline pipeline) : IRpcCaller
{
    public async ValueTask<ReadOnlyMemory<byte>> CallAsync(
        IPEndPoint destination,
        ReadOnlyMemory<byte> request,
        TimeSpan? timeout,
        CancellationToken cancellationToken)
    {
        var response = await pipeline.DispatchAsync(request, destination, cancellationToken);
        return response;
    }
}

internal sealed class ScriptedCaller : IRpcCaller
{
    public int Calls { get; private set; }

    public TimeSpan? LastTimeout { get; private set; }

    public Func<ReadOnlyMemory<byte>, CancellationToken, ValueTask<ReadOnlyMemory<byte>>>? Handle { get; set; }

    public ValueTask<ReadOnlyMemory<byte>> CallAsync(
        IPEndPoint destination,
        ReadOnlyMemory<byte> request,
        TimeSpan? timeout,
        CancellationToken cancellationToken)
    {
        Calls++;
        LastTimeout = timeout;
        if (Handle is null)
            throw new InvalidOperationException("No scripted response.");
        return Handle(request, cancellationToken);
    }
}

internal sealed class ProbeMiddleware(string name, List<string> steps) : IRpcMiddleware
{
    public async ValueTask InvokeAsync(RpcInvocation invocation, RpcDispatchDelegate next)
    {
        steps.Add(name + "-before");
        await next(invocation);
        steps.Add(name + "-after");
    }
}

internal static class Echo
{
    public static RpcPipeline Pipeline(List<string>? steps = null, IRpcMiddleware? extra = null)
    {
        var builder = new RpcPipelineBuilder();
        if (extra is not null)
            builder.Use(extra);
        if (steps is not null)
            builder.Use(new ProbeMiddleware("probe", steps));
        builder.Map<string, string>("echo", (text, _) => new ValueTask<string>(text));
        return builder.Build(new BytesRpcSerializer());
    }

    public static HomaRpcClient Client(RpcPipeline pipeline, RpcMetrics? metrics = null) =>
        new(new PipelineCaller(pipeline), pipeline.Serializer, metrics);

    public static IPEndPoint Peer { get; } = new(IPAddress.Parse("10.1.2.3"), 4000);
}
