using System.Text;
using System.Text.Json;

namespace Homa.Rpc;

/// <summary>
/// Converts a request or response value to the payload inside an RPC frame.
/// Null values are rejected. The payload is still copied by the transport.
/// </summary>
public interface IRpcSerializer
{
    /// <summary>Short name used in diagnostics, such as <c>json</c> or <c>bytes</c>.</summary>
    string Name { get; }

    /// <summary>Encodes <paramref name="value"/>. Throws <see cref="RpcException"/> with <see cref="RpcStatus.BadRequest"/> for a null value.</summary>
    byte[] Serialize<T>(T value);

    /// <summary>Decodes <paramref name="payload"/>. Throws when the bytes are not a <typeparamref name="T"/>.</summary>
    T Deserialize<T>(ReadOnlySpan<byte> payload);
}

/// <summary>
/// UTF-8 JSON via <see cref="JsonSerializerDefaults.Web"/> (camelCase property names).
/// A JSON <c>null</c> is <see cref="RpcStatus.BadRequest"/>.
/// </summary>
public sealed class JsonRpcSerializer : IRpcSerializer
{
    /// <summary>Creates a serializer. Null <paramref name="options"/> uses <see cref="JsonSerializerDefaults.Web"/>.</summary>
    public JsonRpcSerializer(JsonSerializerOptions? options = null)
    {
        Options = options ?? new JsonSerializerOptions(JsonSerializerDefaults.Web);
    }

    /// <summary>Options used for both directions.</summary>
    public JsonSerializerOptions Options { get; }

    /// <inheritdoc />
    public string Name => "json";

    /// <inheritdoc />
    public byte[] Serialize<T>(T value)
    {
        if (value is null)
            throw new RpcException(RpcStatus.BadRequest, "RPC payload is null.");
        return JsonSerializer.SerializeToUtf8Bytes(value, Options);
    }

    /// <inheritdoc />
    public T Deserialize<T>(ReadOnlySpan<byte> payload)
    {
        try
        {
            var value = JsonSerializer.Deserialize<T>(payload, Options);
            if (value is null)
                throw new RpcException(RpcStatus.BadRequest, "RPC payload deserialized to null.");
            return value;
        }
        catch (JsonException ex)
        {
            throw new RpcException(RpcStatus.BadRequest, ex.Message, ex);
        }
    }
}

/// <summary>
/// Pass-through for <see cref="T:byte[]"/> and UTF-8 <see cref="string"/>.
/// Other types throw <see cref="RpcException"/> with <see cref="RpcStatus.BadRequest"/>.
/// </summary>
public sealed class BytesRpcSerializer : IRpcSerializer
{
    /// <inheritdoc />
    public string Name => "bytes";

    /// <inheritdoc />
    public byte[] Serialize<T>(T value)
    {
        if (value is null)
            throw new RpcException(RpcStatus.BadRequest, "RPC payload is null.");
        return value switch
        {
            byte[] bytes => (byte[])bytes.Clone(),
            string text => Encoding.UTF8.GetBytes(text),
            _ => throw new RpcException(
                RpcStatus.BadRequest,
                $"BytesRpcSerializer cannot encode {typeof(T).Name}. Use string or byte[]."),
        };
    }

    /// <inheritdoc />
    public T Deserialize<T>(ReadOnlySpan<byte> payload)
    {
        object result;
        if (typeof(T) == typeof(byte[]))
            result = payload.ToArray();
        else if (typeof(T) == typeof(string))
            result = Encoding.UTF8.GetString(payload);
        else
        {
            throw new RpcException(
                RpcStatus.BadRequest,
                $"BytesRpcSerializer cannot decode {typeof(T).Name}. Use string or byte[].");
        }

        return (T)result;
    }
}
