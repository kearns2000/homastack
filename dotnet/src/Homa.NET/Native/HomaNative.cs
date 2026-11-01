using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;

namespace Homa;

/// <summary>
/// <c>LibraryImport</c> surface of <c>libhoma_ffi</c> ABI 1. Declarations follow
/// <c>rust/crates/homa-ffi/include/homa_ffi.h</c>. The logical library name is
/// <c>homa_ffi</c>.
/// </summary>
internal static partial class HomaNative
{
    private const string LibraryName = "homa_ffi";
    private static int _abiOk;

    static HomaNative()
    {
        NativeLayout.Check();
        NativeLibrary.SetDllImportResolver(typeof(HomaNative).Assembly, Resolve);
    }

    internal static void EnsureAbi()
    {
        if (Volatile.Read(ref _abiOk) == 1)
            return;

        var version = Guard(AbiVersion);
        if (version != HomaSocket.AbiVersion)
        {
            throw new HomaException(
                HomaStatus.InvalidArgument,
                0,
                $"libhoma_ffi reports ABI {version}; Homa.NET requires ABI {HomaSocket.AbiVersion}.");
        }

        Volatile.Write(ref _abiOk, 1);
    }

    internal static bool TryLoad(out string error)
    {
        try
        {
            _ = AbiVersion();
            error = "";
            return true;
        }
        catch (Exception ex) when (IsLoadFailure(ex))
        {
            error = ex.Message;
            return false;
        }
    }

    internal static T Guard<T>(Func<T> call)
    {
        try
        {
            return call();
        }
        catch (Exception ex) when (IsLoadFailure(ex))
        {
            throw LibraryMissing(ex);
        }
    }

    internal static string ReadLastErrorMessage()
    {
        var buffer = new byte[512];
        var status = LastErrorMessage(buffer, (ulong)buffer.Length, out var required);
        if (status == (int)HomaStatus.BufferTooSmall)
        {
            if (required is 0 or > 1024 * 1024)
                return "";
            buffer = new byte[checked((int)required)];
            status = LastErrorMessage(buffer, (ulong)buffer.Length, out _);
        }

        if (status != (int)HomaStatus.Ok)
            return "";

        var length = Array.IndexOf(buffer, (byte)0);
        if (length < 0)
            length = buffer.Length;
        return Encoding.UTF8.GetString(buffer, 0, length);
    }

    internal static string StatusText(int status)
    {
        var ptr = StatusName(status);
        return Marshal.PtrToStringUTF8(ptr) ?? "";
    }

    internal static bool IsLoadFailure(Exception ex) =>
        ex is DllNotFoundException or BadImageFormatException or EntryPointNotFoundException;

    internal static HomaException LibraryMissing(Exception ex) =>
        new(
            HomaStatus.Unavailable,
            0,
            "libhoma_ffi could not be loaded. Set HOMA_FFI_PATH to libhoma_ffi.so (Linux) or libhoma_ffi.dylib (macOS), or place it under runtimes/<rid>/native/.",
            ex);

    /// <summary>
    /// Search order: <c>HOMA_FFI_PATH</c>, then
    /// <c>runtimes/&lt;rid&gt;/native</c> beside the application, then the
    /// default loader probe for <c>homa_ffi</c>.
    /// </summary>
    private static IntPtr Resolve(string libraryName, Assembly assembly, DllImportSearchPath? searchPath)
    {
        _ = assembly;
        _ = searchPath;
        if (libraryName is not ("homa_ffi" or "libhoma_ffi.so" or "libhoma_ffi.dylib" or "homa_ffi.dll"))
            return IntPtr.Zero;

        var configured = Environment.GetEnvironmentVariable("HOMA_FFI_PATH");
        if (!string.IsNullOrWhiteSpace(configured))
        {
            if (!File.Exists(configured))
            {
                throw new DllNotFoundException(
                    $"HOMA_FFI_PATH is set to '{configured}', which does not exist.");
            }

            return NativeLibrary.Load(configured);
        }

        var packaged = Path.Combine(
            AppContext.BaseDirectory,
            "runtimes",
            RuntimeInformation.RuntimeIdentifier,
            "native",
            SharedLibraryFileName());
        if (File.Exists(packaged))
            return NativeLibrary.Load(packaged);

        return IntPtr.Zero;
    }

    private static string SharedLibraryFileName()
    {
        if (OperatingSystem.IsWindows())
            return "homa_ffi.dll";
        if (OperatingSystem.IsMacOS())
            return "libhoma_ffi.dylib";
        return "libhoma_ffi.so";
    }

    [LibraryImport(LibraryName, EntryPoint = "homa_ffi_abi_version")]
    internal static partial uint AbiVersion();

    [LibraryImport(LibraryName, EntryPoint = "homa_ffi_platform_supported")]
    [return: MarshalAs(UnmanagedType.U1)]
    internal static partial bool PlatformSupported();

    [LibraryImport(LibraryName, EntryPoint = "homa_ffi_max_message_length")]
    internal static partial uint MaxMessageLength();

    [LibraryImport(LibraryName, EntryPoint = "homa_status_name")]
    internal static partial IntPtr StatusName(int status);

    [LibraryImport(LibraryName, EntryPoint = "homa_last_error_message")]
    internal static partial int LastErrorMessage(Span<byte> buffer, ulong bufferLength, out ulong required);

    [LibraryImport(LibraryName, EntryPoint = "homa_last_error_errno")]
    internal static partial int LastErrorErrno();

    [LibraryImport(LibraryName, EntryPoint = "homa_socket_open")]
    internal static partial int SocketOpen(in HomaSocketOptionsNative options, out IntPtr socket);

    [LibraryImport(LibraryName, EntryPoint = "homa_socket_bind")]
    internal static partial int SocketBind(in HomaSocketOptionsNative options, ushort port, out IntPtr socket);

    [LibraryImport(LibraryName, EntryPoint = "homa_socket_close")]
    internal static partial int SocketClose(IntPtr socket);

    [LibraryImport(LibraryName, EntryPoint = "homa_socket_shutdown")]
    internal static partial int Shutdown(HomaSocketHandle socket);

    [LibraryImport(LibraryName, EntryPoint = "homa_socket_local_port")]
    internal static partial int LocalPort(HomaSocketHandle socket, out ushort port);

    [LibraryImport(LibraryName, EntryPoint = "homa_socket_info")]
    internal static partial int SocketInfo(HomaSocketHandle socket, ref HomaSocketInfoNative info);

    [LibraryImport(LibraryName, EntryPoint = "homa_send_request")]
    internal static partial int SendRequest(
        HomaSocketHandle socket,
        in HomaEndpointNative destination,
        ReadOnlySpan<byte> payload,
        ulong payloadLength,
        ulong completionCookie,
        uint flags,
        out ulong rpcId);

    [LibraryImport(LibraryName, EntryPoint = "homa_recv_response")]
    internal static partial int RecvResponse(
        HomaSocketHandle socket,
        ulong rpcId,
        long timeoutMilliseconds,
        out IntPtr message);

    [LibraryImport(LibraryName, EntryPoint = "homa_request")]
    internal static partial int Request(
        HomaSocketHandle socket,
        in HomaEndpointNative destination,
        ReadOnlySpan<byte> payload,
        ulong payloadLength,
        long timeoutMilliseconds,
        out IntPtr message);

    [LibraryImport(LibraryName, EntryPoint = "homa_recv")]
    internal static partial int Recv(HomaSocketHandle socket, long timeoutMilliseconds, out IntPtr message);

    [LibraryImport(LibraryName, EntryPoint = "homa_reply")]
    internal static partial int Reply(
        HomaSocketHandle socket,
        ulong rpcId,
        in HomaEndpointNative peer,
        ReadOnlySpan<byte> payload,
        ulong payloadLength);

    [LibraryImport(LibraryName, EntryPoint = "homa_reply_to_message")]
    internal static partial int ReplyToMessage(
        HomaSocketHandle socket,
        HomaMessageHandle request,
        ReadOnlySpan<byte> payload,
        ulong payloadLength);

    [LibraryImport(LibraryName, EntryPoint = "homa_abort_rpc")]
    internal static partial int Abort(HomaSocketHandle socket, ulong rpcId);

    [LibraryImport(LibraryName, EntryPoint = "homa_message_info")]
    internal static partial int MessageInfo(HomaMessageHandle message, ref HomaMessageInfoNative info);

    [LibraryImport(LibraryName, EntryPoint = "homa_message_free")]
    internal static partial void MessageFree(IntPtr message);
}
