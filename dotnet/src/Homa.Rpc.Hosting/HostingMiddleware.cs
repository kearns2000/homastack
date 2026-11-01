using Homa.Rpc;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;

namespace Homa.Rpc.Hosting;

/// <summary>
/// Writes one line when a call enters the pipeline and one when it leaves.
/// The leaving line runs after the response has been serialized, so a
/// serialization failure is logged with its status. Created once and shared
/// across calls.
/// </summary>
public sealed class LoggingRpcMiddleware : IRpcMiddleware
{
    private readonly Microsoft.Extensions.Logging.ILogger<LoggingRpcMiddleware> _logger;

    /// <summary>Creates the middleware.</summary>
    public LoggingRpcMiddleware(Microsoft.Extensions.Logging.ILogger<LoggingRpcMiddleware> logger)
    {
        ArgumentNullException.ThrowIfNull(logger);
        _logger = logger;
    }

    /// <inheritdoc />
    public async ValueTask InvokeAsync(RpcInvocation invocation, RpcDispatchDelegate next)
    {
        var peer = invocation.Context.Peer?.ToString() ?? "(unknown)";
        _logger.LogInformation("Homa RPC {Method} from {Peer} started.", invocation.Method, peer);
        await next(invocation).ConfigureAwait(false);
        if (invocation.Status == RpcStatus.Ok)
        {
            _logger.LogInformation("Homa RPC {Method} completed.", invocation.Method);
            return;
        }

        _logger.LogWarning(
            "Homa RPC {Method} completed with {Status}: {Detail}",
            invocation.Method,
            invocation.Status,
            invocation.Detail);
    }
}

/// <summary>
/// Opens a dependency-injection scope for one call and stores its
/// <see cref="IServiceProvider"/> on <see cref="RpcContext.Items"/>.
/// The pipeline serializes the response before this scope is disposed.
/// </summary>
public sealed class ScopeRpcMiddleware : IRpcMiddleware
{
    private readonly IServiceScopeFactory _scopes;

    /// <summary>Creates the middleware.</summary>
    public ScopeRpcMiddleware(IServiceScopeFactory scopes)
    {
        ArgumentNullException.ThrowIfNull(scopes);
        _scopes = scopes;
    }

    /// <inheritdoc />
    public async ValueTask InvokeAsync(RpcInvocation invocation, RpcDispatchDelegate next)
    {
        using var scope = _scopes.CreateScope();
        invocation.Context.Items[typeof(IServiceProvider)] = scope.ServiceProvider;
        await next(invocation).ConfigureAwait(false);
    }
}

/// <summary>Reads the scope <see cref="ScopeRpcMiddleware"/> attached to a call.</summary>
public static class RpcContextServiceProviderExtensions
{
    /// <summary>
    /// The scoped provider for this call, or null when the pipeline was built without hosting.
    /// The provider is disposed when the call unwinds, so do not keep it.
    /// </summary>
    public static IServiceProvider? GetRequestServices(this RpcContext context)
    {
        ArgumentNullException.ThrowIfNull(context);
        return context.Items.TryGetValue(typeof(IServiceProvider), out var value)
            ? value as IServiceProvider
            : null;
    }
}
