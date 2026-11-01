using System.Net;
using Homa.Rpc;
using Microsoft.Extensions.DependencyInjection;

namespace Homa.Rpc.Hosting;

/// <summary>Registers the RPC pipeline, the hosted server, and a client.</summary>
public static class HomaRpcServiceCollectionExtensions
{
    /// <summary>
    /// Registers <see cref="RpcPipeline"/>, <see cref="IRpcSerializer"/>, and <see cref="RpcMetrics"/>.
    /// Does not open a socket. Call <see cref="AddHomaRpcServer"/> to listen, and register
    /// an <see cref="IRpcCaller"/> before <see cref="AddHomaRpcClient"/>.
    /// </summary>
    public static IServiceCollection AddHomaRpc(this IServiceCollection services, Action<HomaRpcBuilder> configure)
    {
        ArgumentNullException.ThrowIfNull(services);
        ArgumentNullException.ThrowIfNull(configure);
        var builder = new HomaRpcBuilder();
        configure(builder);
        builder.Register(services);
        return services;
    }

    /// <summary>
    /// Registers <see cref="HomaRpcServerHostedService"/>. The socket is opened in
    /// <see cref="Microsoft.Extensions.Hosting.IHostedService.StartAsync"/>, which needs Linux with <c>homa.ko</c>.
    /// That start has not been executed in this repository.
    /// </summary>
    public static IServiceCollection AddHomaRpcServer(this IServiceCollection services, IPEndPoint listen, Homa.HomaSocketOptions? socketOptions = null)
    {
        ArgumentNullException.ThrowIfNull(services);
        ArgumentNullException.ThrowIfNull(listen);
        services.AddSingleton(new HomaRpcServerBinding(listen, socketOptions));
        services.AddHostedService<HomaRpcServerHostedService>();
        return services;
    }

    /// <summary>
    /// Registers <see cref="HomaRpcClient"/> from <see cref="IRpcCaller"/>, <see cref="IRpcSerializer"/>,
    /// and <see cref="RpcMetrics"/>. The caller is not created here, so a test can supply one
    /// that never opens a socket. An application that should dial Homa registers
    /// <c>services.AddSingleton&lt;IRpcCaller&gt;(_ =&gt; HomaRpcCaller.Open())</c>.
    /// </summary>
    public static IServiceCollection AddHomaRpcClient(this IServiceCollection services)
    {
        ArgumentNullException.ThrowIfNull(services);
        services.AddSingleton(sp => new HomaRpcClient(
            sp.GetRequiredService<IRpcCaller>(),
            sp.GetRequiredService<IRpcSerializer>(),
            sp.GetRequiredService<RpcMetrics>()));
        return services;
    }
}
