using System.Net;
using Homa.Rpc;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;

namespace Homa.Rpc.Hosting;

/// <summary>Where <see cref="HomaRpcServerHostedService"/> listens. Resolving this does not open a socket.</summary>
public sealed class HomaRpcServerBinding
{
    /// <summary>Stores the listen endpoint. <see cref="Homa.HomaSocket.Bind"/> rejects port 0 and ports above 32767.</summary>
    public HomaRpcServerBinding(
        IPEndPoint listen,
        Homa.HomaSocketOptions? socketOptions = null,
        int maxInFlight = HomaRpcServer.DefaultMaxInFlight)
    {
        ArgumentNullException.ThrowIfNull(listen);
        if (maxInFlight < 1)
            throw new ArgumentOutOfRangeException(nameof(maxInFlight), "At least one in-flight RPC is required.");
        Listen = listen;
        SocketOptions = socketOptions;
        MaxInFlight = maxInFlight;
    }

    /// <summary>Local endpoint passed to <see cref="HomaRpcServer.Bind"/>.</summary>
    public IPEndPoint Listen { get; }

    /// <summary>Socket options. Null lets the bind endpoint choose the address family.</summary>
    public Homa.HomaSocketOptions? SocketOptions { get; }

    /// <summary>Handler cap passed to <see cref="HomaRpcServer.Bind"/>.</summary>
    public int MaxInFlight { get; }
}

/// <summary>
/// Collects method and middleware registrations for <see cref="HomaRpcServiceCollectionExtensions.AddHomaRpc"/>.
/// The delegates run when the <see cref="RpcPipeline"/> singleton is created.
/// Middleware types are activated from dependency injection, one instance each.
/// </summary>
public sealed class HomaRpcBuilder
{
    private readonly RpcPipelineBuilder _pipeline = new();
    private readonly List<Type> _middleware = [];
    private Type _serializer = typeof(JsonRpcSerializer);
    private bool _logging = true;
    private bool _scope = true;

    /// <summary>Replaces the default <see cref="JsonRpcSerializer"/>.</summary>
    public HomaRpcBuilder UseSerializer<TSerializer>()
        where TSerializer : class, IRpcSerializer
    {
        _serializer = typeof(TSerializer);
        return this;
    }

    /// <summary>
    /// Adds middleware inside logging and the call scope.
    /// The first call is the outermost of these user middleware.
    /// </summary>
    public HomaRpcBuilder UseMiddleware<TMiddleware>()
        where TMiddleware : class, IRpcMiddleware
    {
        _middleware.Add(typeof(TMiddleware));
        return this;
    }

    /// <summary>Turns off <see cref="LoggingRpcMiddleware"/>.</summary>
    public HomaRpcBuilder DisableRequestLogging()
    {
        _logging = false;
        return this;
    }

    /// <summary>Turns off the per-call dependency-injection scope.</summary>
    public HomaRpcBuilder DisableRequestScope()
    {
        _scope = false;
        return this;
    }

    /// <inheritdoc cref="RpcPipelineBuilder.Map{TRequest, TResponse}"/>
    public HomaRpcBuilder Map<TRequest, TResponse>(
        string method,
        Func<TRequest, RpcContext, ValueTask<TResponse>> handler)
    {
        _pipeline.Map(method, handler);
        return this;
    }

    internal void Register(IServiceCollection services)
    {
        services.TryAddSingleton<RpcMetrics>();
        services.AddSingleton(typeof(IRpcSerializer), _serializer);
        var pipeline = _pipeline;
        var middleware = _middleware.ToArray();
        var logging = _logging;
        var scope = _scope;
        services.AddSingleton(sp =>
        {
            var leading = new List<IRpcMiddleware>(middleware.Length + 2);
            if (logging)
                leading.Add(ActivatorUtilities.CreateInstance<LoggingRpcMiddleware>(sp));
            if (scope)
                leading.Add(ActivatorUtilities.CreateInstance<ScopeRpcMiddleware>(sp));
            foreach (var type in middleware)
                leading.Add((IRpcMiddleware)ActivatorUtilities.CreateInstance(sp, type));
            return pipeline.Build(
                sp.GetRequiredService<IRpcSerializer>(),
                leading,
                sp.GetRequiredService<RpcMetrics>());
        });
    }
}
