using System.Buffers.Binary;
using System.Diagnostics.CodeAnalysis;
using System.Text;

namespace Homa.Rpc;

/// <summary>Which RPC message a frame carries. Values are written as <c>uint16</c>.</summary>
public enum RpcFrameKind : ushort
{
    /// <summary>A call from a client.</summary>
    Request = 1,

    /// <summary>A reply from a server, including failures.</summary>
    Response = 2,
}

/// <summary>
/// One decoded frame. <see cref="Payload"/> is a copy of the bytes inside the frame.
/// </summary>
public sealed class DecodedRpcFrame
{
    /// <summary>Request or response.</summary>
    public required RpcFrameKind Kind { get; init; }

    /// <summary>Status. Requests use <see cref="RpcStatus.Ok"/>.</summary>
    public required RpcStatus Status { get; init; }

    /// <summary>Method name. Empty on a failure that never parsed one.</summary>
    public required string Method { get; init; }

    /// <summary>Failure text. Empty on success.</summary>
    public required string Detail { get; init; }

    /// <summary>Serializer payload. Empty when <see cref="Status"/> is not <see cref="RpcStatus.Ok"/>.</summary>
    public required byte[] Payload { get; init; }
}

/// <summary>
/// Length-prefixed RPC frame carried as the Homa message payload.
/// Little-endian. The first four bytes are the ASCII bytes <c>HOMA</c>.
/// </summary>
/// <remarks>
/// <code>
/// offset  0  uint32  magic 0x414D4F48 (bytes 48 4F 4D 41)
/// offset  4  uint16  version (1)
/// offset  6  uint16  kind (1 request, 2 response)
/// offset  8  int32   status (<see cref="RpcStatus"/>)
/// offset 12  uint16  method name length
/// offset 14  uint16  detail length
/// offset 16  uint32  payload length
/// offset 20  method name, detail, payload (UTF-8 text, then raw bytes)
/// </code>
/// The whole frame must fit in <see cref="Homa.HomaSocket.MaxPayloadLength"/>.
/// </remarks>
public static class RpcFrame
{
    /// <summary>Little-endian form of the ASCII bytes <c>HOMA</c>.</summary>
    public const uint Magic = 0x414D4F48;

    /// <summary>Only version this stack reads or writes.</summary>
    public const ushort Version = 1;

    /// <summary>Fixed header size in bytes.</summary>
    public const int HeaderSize = 20;

    /// <summary>Maximum UTF-8 length of a method name.</summary>
    public const int MaxMethodNameBytes = 256;

    /// <summary>Maximum UTF-8 length of a failure detail.</summary>
    public const int MaxDetailBytes = 1024;

    private static readonly UTF8Encoding Utf8 = new(encoderShouldEmitUTF8Identifier: false, throwOnInvalidBytes: true);

    /// <summary>Writes one frame. A detail longer than <see cref="MaxDetailBytes"/> is cut on a UTF-8 boundary.</summary>
    public static byte[] Encode(
        RpcFrameKind kind,
        RpcStatus status,
        string method,
        string? detail,
        ReadOnlySpan<byte> payload)
    {
        ArgumentNullException.ThrowIfNull(method);
        if (kind is not (RpcFrameKind.Request or RpcFrameKind.Response))
            throw new RpcException(RpcStatus.BadRequest, "RPC frame kind is not a request or a response.");

        detail = LimitDetail(detail);
        int methodBytes;
        int detailBytes;
        try
        {
            methodBytes = Utf8.GetByteCount(method);
            detailBytes = Utf8.GetByteCount(detail);
        }
        catch (ArgumentException ex)
        {
            throw new RpcException(RpcStatus.BadRequest, "RPC frame text is not valid Unicode.", ex);
        }

        if (methodBytes > MaxMethodNameBytes)
        {
            throw new RpcException(
                RpcStatus.BadRequest,
                $"RPC method name is {methodBytes} bytes; the maximum is {MaxMethodNameBytes}.");
        }

        var total = (long)HeaderSize + methodBytes + detailBytes + payload.Length;
        if (total > Homa.HomaSocket.MaxPayloadLength)
        {
            throw new RpcException(
                RpcStatus.PayloadTooLarge,
                $"RPC frame is {total} bytes; the maximum is {Homa.HomaSocket.MaxPayloadLength}.");
        }

        var buffer = new byte[total];
        BinaryPrimitives.WriteUInt32LittleEndian(buffer, Magic);
        BinaryPrimitives.WriteUInt16LittleEndian(buffer.AsSpan(4), Version);
        BinaryPrimitives.WriteUInt16LittleEndian(buffer.AsSpan(6), (ushort)kind);
        BinaryPrimitives.WriteInt32LittleEndian(buffer.AsSpan(8), (int)status);
        BinaryPrimitives.WriteUInt16LittleEndian(buffer.AsSpan(12), (ushort)methodBytes);
        BinaryPrimitives.WriteUInt16LittleEndian(buffer.AsSpan(14), (ushort)detailBytes);
        BinaryPrimitives.WriteUInt32LittleEndian(buffer.AsSpan(16), (uint)payload.Length);
        var offset = HeaderSize;
        offset += Utf8.GetBytes(method, buffer.AsSpan(offset));
        offset += Utf8.GetBytes(detail, buffer.AsSpan(offset));
        payload.CopyTo(buffer.AsSpan(offset));
        return buffer;
    }

    /// <summary>
    /// Reads one frame. <paramref name="error"/> explains a rejection.
    /// Trailing bytes, a bad version, and invalid UTF-8 are rejected.
    /// </summary>
    public static bool TryDecode(
        ReadOnlySpan<byte> bytes,
        [NotNullWhen(true)] out DecodedRpcFrame? frame,
        out string error)
    {
        frame = null;
        if (bytes.Length < HeaderSize)
        {
            error = "RPC frame is shorter than the header.";
            return false;
        }

        if (bytes.Length > Homa.HomaSocket.MaxPayloadLength)
        {
            error = "RPC frame is longer than HOMA_FFI_MAX_MESSAGE_LENGTH.";
            return false;
        }

        if (BinaryPrimitives.ReadUInt32LittleEndian(bytes) != Magic)
        {
            error = "RPC frame magic is not HOMA.";
            return false;
        }

        var version = BinaryPrimitives.ReadUInt16LittleEndian(bytes[4..]);
        if (version != Version)
        {
            error = $"RPC frame version {version} is not supported.";
            return false;
        }

        var kindRaw = BinaryPrimitives.ReadUInt16LittleEndian(bytes[6..]);
        if (kindRaw is not ((ushort)RpcFrameKind.Request or (ushort)RpcFrameKind.Response))
        {
            error = "RPC frame kind is not a request or a response.";
            return false;
        }

        var statusRaw = BinaryPrimitives.ReadInt32LittleEndian(bytes[8..]);
        var methodLength = BinaryPrimitives.ReadUInt16LittleEndian(bytes[12..]);
        var detailLength = BinaryPrimitives.ReadUInt16LittleEndian(bytes[14..]);
        var payloadLength = BinaryPrimitives.ReadUInt32LittleEndian(bytes[16..]);
        if (methodLength > MaxMethodNameBytes)
        {
            error = "RPC method name is longer than 256 bytes.";
            return false;
        }

        if (detailLength > MaxDetailBytes)
        {
            error = "RPC detail is longer than 1024 bytes.";
            return false;
        }

        var total = (long)HeaderSize + methodLength + detailLength + payloadLength;
        if (total != bytes.Length)
        {
            error = "RPC frame lengths do not match the buffer.";
            return false;
        }

        var offset = HeaderSize;
        string method;
        string detail;
        try
        {
            method = Utf8.GetString(bytes.Slice(offset, methodLength));
            offset += methodLength;
            detail = Utf8.GetString(bytes.Slice(offset, detailLength));
            offset += detailLength;
        }
        catch (ArgumentException)
        {
            error = "RPC frame text is not UTF-8.";
            return false;
        }

        if (method.Contains('\0'))
        {
            error = "RPC method name contains a null character.";
            return false;
        }

        var payload = payloadLength == 0 ? [] : bytes.Slice(offset, (int)payloadLength).ToArray();
        frame = new DecodedRpcFrame
        {
            Kind = (RpcFrameKind)kindRaw,
            Status = (RpcStatus)statusRaw,
            Method = method,
            Detail = detail,
            Payload = payload,
        };
        error = "";
        return true;
    }

    internal static string RequireMethod(string? method)
    {
        ArgumentNullException.ThrowIfNull(method);
        if (method.Length == 0)
            throw new ArgumentException("RPC method name is empty.", nameof(method));
        if (method.Contains('\0'))
            throw new ArgumentException("RPC method name contains a null character.", nameof(method));
        int bytes;
        try
        {
            bytes = Utf8.GetByteCount(method);
        }
        catch (ArgumentException ex)
        {
            throw new ArgumentException("RPC method name is not valid Unicode.", nameof(method), ex);
        }

        if (bytes > MaxMethodNameBytes)
        {
            throw new ArgumentException(
                $"RPC method name is {bytes} bytes; the maximum is {MaxMethodNameBytes}.",
                nameof(method));
        }

        return method;
    }

    internal static void CheckTimeout(TimeSpan? timeout)
    {
        if (timeout is not { } value || value == Timeout.InfiniteTimeSpan || value >= TimeSpan.Zero)
            return;
        throw new ArgumentOutOfRangeException(
            nameof(timeout),
            "Timeout must be zero, positive, or infinite.");
    }

    private static string LimitDetail(string? detail)
    {
        if (string.IsNullOrEmpty(detail))
            return "";
        try
        {
            if (Utf8.GetByteCount(detail) <= MaxDetailBytes)
                return detail;
        }
        catch (ArgumentException)
        {
            return "RPC detail is not valid Unicode.";
        }

        var bytes = 0;
        var chars = 0;
        foreach (var rune in detail.EnumerateRunes())
        {
            var next = bytes + rune.Utf8SequenceLength;
            if (next > MaxDetailBytes)
                break;
            bytes = next;
            chars += rune.Utf16SequenceLength;
        }

        return detail[..chars];
    }
}
