using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

namespace Homa;

[InlineArray(16)]
internal struct Byte16
{
    private byte _element0;
}

/// <summary>
/// <c>homa_endpoint_t</c>. 24 bytes, alignment 4.
/// <c>addr</c> is network byte order. IPv4 uses the first 4 bytes.
/// </summary>
[StructLayout(LayoutKind.Sequential)]
internal struct HomaEndpointNative
{
    public ushort Family;
    public ushort Port;
    public uint ScopeId;
    public Byte16 Address;
}

/// <summary><c>homa_socket_options_t</c>. 24 bytes, alignment 8.</summary>
[StructLayout(LayoutKind.Sequential)]
internal struct HomaSocketOptionsNative
{
    internal const int Size = 24;
    internal const uint ServerFlag = 1;

    public uint StructSize;
    public ushort Family;
    public ushort BindPort;
    public ulong ReceiveRegionBytes;
    public uint Flags;
    public uint Reserved;
}

/// <summary><c>homa_socket_info_t</c>. 32 bytes, alignment 8.</summary>
[StructLayout(LayoutKind.Sequential)]
internal struct HomaSocketInfoNative
{
    internal const int Size = 32;

    public uint StructSize;
    public ushort Port;
    public ushort Family;
    public uint ActiveRpcs;
    public uint PendingBpages;
    public ulong BufferAvailableBytes;
    public ulong ReceiveRegionBytes;
}

/// <summary>
/// <c>homa_message_info_t</c>. 72 bytes, alignment 8.
/// <see cref="Payload"/> is owned by the native message until it is freed.
/// </summary>
[StructLayout(LayoutKind.Sequential)]
internal struct HomaMessageInfoNative
{
    internal const int Size = 72;

    public uint StructSize;
    public uint Kind;
    public ulong RpcId;
    public ulong CompletionCookie;
    public HomaEndpointNative Peer;
    public IntPtr Payload;
    public ulong PayloadLength;
    public uint Fragments;
    public uint Reserved;
}

/// <summary>
/// Fails fast when the managed structs drift from <c>homa_ffi.h</c> ABI 1.
/// Sizes and offsets are the ones asserted by <c>homa-ffi</c>.
/// </summary>
internal static class NativeLayout
{
    internal static void Check()
    {
        ExpectSize<HomaEndpointNative>(24, "homa_endpoint_t");
        ExpectOffset<HomaEndpointNative>(nameof(HomaEndpointNative.Family), 0, "homa_endpoint_t.family");
        ExpectOffset<HomaEndpointNative>(nameof(HomaEndpointNative.Port), 2, "homa_endpoint_t.port");
        ExpectOffset<HomaEndpointNative>(nameof(HomaEndpointNative.ScopeId), 4, "homa_endpoint_t.scope_id");
        ExpectOffset<HomaEndpointNative>(nameof(HomaEndpointNative.Address), 8, "homa_endpoint_t.addr");

        ExpectSize<HomaSocketOptionsNative>(HomaSocketOptionsNative.Size, "homa_socket_options_t");
        ExpectOffset<HomaSocketOptionsNative>(nameof(HomaSocketOptionsNative.StructSize), 0, "homa_socket_options_t.struct_size");
        ExpectOffset<HomaSocketOptionsNative>(nameof(HomaSocketOptionsNative.Family), 4, "homa_socket_options_t.family");
        ExpectOffset<HomaSocketOptionsNative>(nameof(HomaSocketOptionsNative.BindPort), 6, "homa_socket_options_t.bind_port");
        ExpectOffset<HomaSocketOptionsNative>(nameof(HomaSocketOptionsNative.ReceiveRegionBytes), 8, "homa_socket_options_t.receive_region_bytes");
        ExpectOffset<HomaSocketOptionsNative>(nameof(HomaSocketOptionsNative.Flags), 16, "homa_socket_options_t.flags");
        ExpectOffset<HomaSocketOptionsNative>(nameof(HomaSocketOptionsNative.Reserved), 20, "homa_socket_options_t.reserved");

        ExpectSize<HomaSocketInfoNative>(HomaSocketInfoNative.Size, "homa_socket_info_t");
        ExpectOffset<HomaSocketInfoNative>(nameof(HomaSocketInfoNative.Port), 4, "homa_socket_info_t.port");
        ExpectOffset<HomaSocketInfoNative>(nameof(HomaSocketInfoNative.ActiveRpcs), 8, "homa_socket_info_t.active_rpcs");
        ExpectOffset<HomaSocketInfoNative>(nameof(HomaSocketInfoNative.PendingBpages), 12, "homa_socket_info_t.pending_bpages");
        ExpectOffset<HomaSocketInfoNative>(nameof(HomaSocketInfoNative.BufferAvailableBytes), 16, "homa_socket_info_t.buffer_available_bytes");
        ExpectOffset<HomaSocketInfoNative>(nameof(HomaSocketInfoNative.ReceiveRegionBytes), 24, "homa_socket_info_t.receive_region_bytes");

        ExpectSize<HomaMessageInfoNative>(HomaMessageInfoNative.Size, "homa_message_info_t");
        ExpectOffset<HomaMessageInfoNative>(nameof(HomaMessageInfoNative.Kind), 4, "homa_message_info_t.kind");
        ExpectOffset<HomaMessageInfoNative>(nameof(HomaMessageInfoNative.RpcId), 8, "homa_message_info_t.rpc_id");
        ExpectOffset<HomaMessageInfoNative>(nameof(HomaMessageInfoNative.CompletionCookie), 16, "homa_message_info_t.completion_cookie");
        ExpectOffset<HomaMessageInfoNative>(nameof(HomaMessageInfoNative.Peer), 24, "homa_message_info_t.peer");
        ExpectOffset<HomaMessageInfoNative>(nameof(HomaMessageInfoNative.Payload), 48, "homa_message_info_t.payload");
        ExpectOffset<HomaMessageInfoNative>(nameof(HomaMessageInfoNative.PayloadLength), 56, "homa_message_info_t.payload_len");
        ExpectOffset<HomaMessageInfoNative>(nameof(HomaMessageInfoNative.Fragments), 64, "homa_message_info_t.fragments");
    }

    private static void ExpectSize<T>(int expected, string what)
        where T : struct
    {
        var actual = Marshal.SizeOf<T>();
        if (actual != expected)
        {
            throw new InvalidOperationException(
                $"{what} is {actual} bytes in this process; homa_ffi.h ABI 1 requires {expected}.");
        }
    }

    private static void ExpectOffset<T>(string field, int expected, string what)
        where T : struct
    {
        var actual = Marshal.OffsetOf<T>(field);
        if (actual != expected)
        {
            throw new InvalidOperationException(
                $"{what} is at offset {actual}; homa_ffi.h ABI 1 requires {expected}.");
        }
    }
}
