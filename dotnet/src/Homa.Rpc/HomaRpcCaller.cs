using System.Net;

namespace Homa.Rpc;

/// <summary>
/// Sends one already-encoded request frame and returns the response frame.
/// <see cref="HomaRpcCaller"/> is the <see cref="Homa.HomaClient"/> implementation.
/// Tests supply an in-memory caller. This interface does not open a socket.
/// </summary>
public interface IRpcCaller
{
    /// <summary>
    /// Delivers <paramref name="request"/> to <paramref name="destination"/>.
    /// <paramref name="timeout"/> null waits until the transport finishes.
    /// A cancelled <paramref name="cancellationToken"/> throws <see cref="OperationCanceledException"/>.
    /// </summary>
    ValueTask<ReadOnlyMemory<byte>> CallAsync(
        IPEndPoint destination,
        ReadOnlyMemory<byte> request,
        TimeSpan? timeout,
        CancellationToken cancellationToken);
}

/// <summary>
/// <see cref="IRpcCaller"/> over a private <see cref="Homa.HomaClient"/> request.
/// Cancelling the token aborts that RPC through <c>Homa.NET</c>.
/// A module built without <c>HOMAIOCABORT</c> cannot honour the abort.
/// </summary>
/// <remarks>
/// <see cref="Open"/> creates a Homa socket and needs Linux with <c>homa.ko</c>.
/// <c>Homa.KernelSmoke</c> has done that once; see
/// <c>tests/integration/results/20261110T224800Z-lima-homa.log</c>.
/// </remarks>
public sealed class HomaRpcCaller : IRpcCaller, IDisposable, IAsyncDisposable
{
    private readonly Homa.HomaClient _client;
    private readonly bool _ownsClient;

    /// <summary>Uses <paramref name="client"/> without taking ownership.</summary>
    public HomaRpcCaller(Homa.HomaClient client)
    {
        ArgumentNullException.ThrowIfNull(client);
        _client = client;
        _ownsClient = false;
    }

    private HomaRpcCaller(Homa.HomaClient client, bool ownsClient)
    {
        _client = client;
        _ownsClient = ownsClient;
    }

    /// <summary>Opens a client socket. See <see cref="Homa.HomaClient"/>.</summary>
    public static HomaRpcCaller Open(Homa.HomaSocketOptions? options = null) =>
        new(new Homa.HomaClient(options), ownsClient: true);

    /// <summary>The socket owner. Disposing this caller closes it only when <see cref="Open"/> created it.</summary>
    public Homa.HomaClient Client => _client;

    /// <inheritdoc />
    public async ValueTask<ReadOnlyMemory<byte>> CallAsync(
        IPEndPoint destination,
        ReadOnlyMemory<byte> request,
        TimeSpan? timeout,
        CancellationToken cancellationToken)
    {
        using var message = await _client.RequestAsync(destination, request, timeout, cancellationToken)
            .ConfigureAwait(false);
        return message.CopyPayload();
    }

    /// <summary>Closes the client when this caller opened it.</summary>
    public void Dispose()
    {
        if (_ownsClient)
            _client.Dispose();
    }

    /// <inheritdoc cref="Dispose"/>
    public ValueTask DisposeAsync()
    {
        Dispose();
        return ValueTask.CompletedTask;
    }
}
