using System.Runtime.InteropServices;

namespace Homa;

/// <summary>
/// Owns one <c>homa_socket_t*</c>. The bit pattern is a generation-checked
/// registry key, never a pointer to free. <see cref="ReleaseHandle"/> calls
/// <c>homa_socket_close</c> exactly once.
/// </summary>
internal sealed class HomaSocketHandle : SafeHandle
{
    public HomaSocketHandle()
        : base(IntPtr.Zero, ownsHandle: true)
    {
    }

    internal static HomaSocketHandle Adopt(IntPtr raw)
    {
        var handle = new HomaSocketHandle();
        handle.SetHandle(raw);
        return handle;
    }

    public override bool IsInvalid => handle == IntPtr.Zero;

    protected override bool ReleaseHandle()
    {
        _ = HomaNative.SocketClose(handle);
        return true;
    }
}

/// <summary>
/// Owns one <c>homa_message_t*</c>. <see cref="ReleaseHandle"/> calls
/// <c>homa_message_free</c> exactly once. A null free is a native no-op.
/// </summary>
internal sealed class HomaMessageHandle : SafeHandle
{
    public HomaMessageHandle()
        : base(IntPtr.Zero, ownsHandle: true)
    {
    }

    internal static HomaMessageHandle Adopt(IntPtr raw)
    {
        var handle = new HomaMessageHandle();
        handle.SetHandle(raw);
        return handle;
    }

    public override bool IsInvalid => handle == IntPtr.Zero;

    protected override bool ReleaseHandle()
    {
        HomaNative.MessageFree(handle);
        return true;
    }
}
