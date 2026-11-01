using System.Net;

namespace Homa;

/// <summary>
/// A client socket. <see cref="Socket"/> is the same object <see cref="Dispose"/>
/// releases; disposing either one closes it.
/// </summary>
public sealed class HomaClient : IDisposable, IAsyncDisposable
{
    private readonly HomaSocket _socket;

    /// <summary>Opens a client socket. See <see cref="HomaSocket.Open"/>.</summary>
    public HomaClient(HomaSocketOptions? options = null)
    {
        _socket = HomaSocket.Open(options);
    }

    /// <summary>The socket this client owns.</summary>
    public HomaSocket Socket => _socket;

    /// <inheritdoc cref="HomaSocket.Request"/>
    public HomaMessage Request(IPEndPoint destination, ReadOnlySpan<byte> payload, TimeSpan? timeout = null) =>
        _socket.Request(destination, payload, timeout);

    /// <inheritdoc cref="HomaSocket.RequestAsync"/>
    public ValueTask<HomaMessage> RequestAsync(
        IPEndPoint destination,
        ReadOnlyMemory<byte> payload,
        TimeSpan? timeout = null,
        CancellationToken cancellationToken = default) =>
        _socket.RequestAsync(destination, payload, timeout, cancellationToken);

    /// <summary>Closes the underlying socket.</summary>
    public void Dispose() => _socket.Dispose();

    /// <inheritdoc cref="Dispose"/>
    public ValueTask DisposeAsync()
    {
        Dispose();
        return ValueTask.CompletedTask;
    }
}
