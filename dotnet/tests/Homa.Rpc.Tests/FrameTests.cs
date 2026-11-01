using System.Text;
using Homa;
using Homa.Rpc;

namespace Homa.Rpc.Tests;

public class FrameTests
{
    [Fact]
    public void Round_trip_keeps_kind_status_method_detail_and_payload()
    {
        var payload = new byte[] { 0, 1, 2, 255 };
        var frame = RpcFrame.Encode(RpcFrameKind.Response, RpcStatus.HandlerFault, "echo", "boom", payload);

        Assert.Equal(RpcFrame.HeaderSize + Encoding.UTF8.GetByteCount("echo") + Encoding.UTF8.GetByteCount("boom") + payload.Length, frame.Length);
        Assert.Equal((byte)'H', frame[0]);
        Assert.Equal((byte)'O', frame[1]);
        Assert.Equal((byte)'M', frame[2]);
        Assert.Equal((byte)'A', frame[3]);

        Assert.True(RpcFrame.TryDecode(frame, out var decoded, out var error));
        Assert.Equal("", error);
        Assert.NotNull(decoded);
        Assert.Equal(RpcFrameKind.Response, decoded.Kind);
        Assert.Equal(RpcStatus.HandlerFault, decoded.Status);
        Assert.Equal("echo", decoded.Method);
        Assert.Equal("boom", decoded.Detail);
        Assert.Equal(payload, decoded.Payload);
    }

    [Fact]
    public void Rejects_a_short_buffer_bad_magic_bad_version_and_trailing_bytes()
    {
        Assert.False(RpcFrame.TryDecode([], out _, out var shortError));
        Assert.Contains("header", shortError, StringComparison.Ordinal);

        var frame = RpcFrame.Encode(RpcFrameKind.Request, RpcStatus.Ok, "echo", "", "hi"u8);
        frame[0] = (byte)'X';
        Assert.False(RpcFrame.TryDecode(frame, out _, out var magicError));
        Assert.Contains("magic", magicError, StringComparison.Ordinal);

        frame = RpcFrame.Encode(RpcFrameKind.Request, RpcStatus.Ok, "echo", "", "hi"u8);
        frame[4] = 2;
        Assert.False(RpcFrame.TryDecode(frame, out _, out var versionError));
        Assert.Contains("version", versionError, StringComparison.Ordinal);

        frame = RpcFrame.Encode(RpcFrameKind.Request, RpcStatus.Ok, "echo", "", "hi"u8);
        var trailed = new byte[frame.Length + 1];
        frame.CopyTo(trailed);
        Assert.False(RpcFrame.TryDecode(trailed, out _, out var trailError));
        Assert.Contains("lengths", trailError, StringComparison.Ordinal);
    }

    [Fact]
    public void Rejects_a_frame_larger_than_the_homa_payload_limit()
    {
        var payload = new byte[HomaSocket.MaxPayloadLength];
        var ex = Assert.Throws<RpcException>(() =>
            RpcFrame.Encode(RpcFrameKind.Request, RpcStatus.Ok, "m", "", payload));
        Assert.Equal(RpcStatus.PayloadTooLarge, ex.Status);
    }

    [Fact]
    public void Truncates_detail_on_a_character_boundary()
    {
        var detail = new string('é', RpcFrame.MaxDetailBytes);
        var frame = RpcFrame.Encode(RpcFrameKind.Response, RpcStatus.HandlerFault, "m", detail, []);
        Assert.True(RpcFrame.TryDecode(frame, out var decoded, out _));
        Assert.NotNull(decoded);
        Assert.True(Encoding.UTF8.GetByteCount(decoded.Detail) <= RpcFrame.MaxDetailBytes);
        Assert.StartsWith("é", decoded.Detail, StringComparison.Ordinal);
    }
}
