using Xunit;

namespace Homa.Tests;

public class NativeLibraryTests
{
    [Fact]
    public void Abi_and_status_names_match_the_loaded_library()
    {
        RequireNative();
        Assert.Equal(HomaSocket.AbiVersion, HomaSocket.NativeAbiVersion);
        Assert.Equal(1u, HomaSocket.NativeAbiVersion);
        Assert.Equal((uint)HomaSocket.MaxPayloadLength, HomaSocket.NativeMaxMessageLength);
        Assert.Equal(OperatingSystem.IsLinux(), HomaSocket.IsPlatformSupported);

        Assert.Equal("HOMA_OK", HomaNative.StatusText((int)HomaStatus.Ok));
        Assert.Equal("HOMA_ERR_INVALID_ARGUMENT", HomaNative.StatusText((int)HomaStatus.InvalidArgument));
        Assert.Equal("HOMA_ERR_UNSUPPORTED_PLATFORM", HomaNative.StatusText((int)HomaStatus.UnsupportedPlatform));
        Assert.Equal("HOMA_ERR_UNAVAILABLE", HomaNative.StatusText((int)HomaStatus.Unavailable));
        Assert.Equal("HOMA_ERR_INVALID_ENDPOINT", HomaNative.StatusText((int)HomaStatus.InvalidEndpoint));
        Assert.Equal("HOMA_ERR_TIMEOUT", HomaNative.StatusText((int)HomaStatus.Timeout));
        Assert.Equal("HOMA_ERR_INTERRUPTED", HomaNative.StatusText((int)HomaStatus.Interrupted));
        Assert.Equal("HOMA_ERR_RPC_FAILED", HomaNative.StatusText((int)HomaStatus.RpcFailed));
        Assert.Equal("HOMA_ERR_SHUTDOWN", HomaNative.StatusText((int)HomaStatus.Shutdown));
        Assert.Equal("HOMA_ERR_NO_MEMORY", HomaNative.StatusText((int)HomaStatus.NoMemory));
        Assert.Equal("HOMA_ERR_WOULD_BLOCK", HomaNative.StatusText((int)HomaStatus.WouldBlock));
        Assert.Equal("HOMA_ERR_ADDRESS_IN_USE", HomaNative.StatusText((int)HomaStatus.AddressInUse));
        Assert.Equal("HOMA_ERR_PERMISSION_DENIED", HomaNative.StatusText((int)HomaStatus.PermissionDenied));
        Assert.Equal("HOMA_ERR_NO_SERVER", HomaNative.StatusText((int)HomaStatus.NoServer));
        Assert.Equal("HOMA_ERR_UNREACHABLE", HomaNative.StatusText((int)HomaStatus.Unreachable));
        Assert.Equal("HOMA_ERR_MESSAGE_TOO_LARGE", HomaNative.StatusText((int)HomaStatus.MessageTooLarge));
        Assert.Equal("HOMA_ERR_BUFFER_TOO_SMALL", HomaNative.StatusText((int)HomaStatus.BufferTooSmall));
        Assert.Equal("HOMA_ERR_IO", HomaNative.StatusText((int)HomaStatus.Io));
        Assert.Equal("HOMA_ERR_INTERNAL", HomaNative.StatusText((int)HomaStatus.Internal));
        Assert.Equal("HOMA_ERR_UNKNOWN", HomaNative.StatusText(42));
    }

    [Fact]
    public void Open_fails_closed_without_a_usable_kernel()
    {
        RequireNative();
        var ex = Record.Exception(() =>
        {
            using var socket = HomaSocket.Open();
            _ = socket.LocalPort;
        });

        if (!HomaSocket.IsPlatformSupported)
        {
            var error = Assert.IsType<HomaException>(ex);
            Assert.Equal(HomaStatus.UnsupportedPlatform, error.Status);
            Assert.False(string.IsNullOrWhiteSpace(error.Detail));
            return;
        }

        if (ex is null)
            return;

        var native = Assert.IsType<HomaException>(ex);
        Assert.Equal(HomaStatus.Unavailable, native.Status);
        Assert.False(string.IsNullOrWhiteSpace(native.Detail));
    }

    [Fact]
    public void HomaClient_uses_the_same_open_result()
    {
        RequireNative();
        var ex = Record.Exception(() =>
        {
            using var client = new HomaClient();
        });

        if (!HomaSocket.IsPlatformSupported)
        {
            Assert.Equal(HomaStatus.UnsupportedPlatform, Assert.IsType<HomaException>(ex).Status);
            return;
        }

        if (ex is null)
            return;

        Assert.Equal(HomaStatus.Unavailable, Assert.IsType<HomaException>(ex).Status);
    }

    private static void RequireNative()
    {
        var configured = Environment.GetEnvironmentVariable("HOMA_FFI_PATH");
        if (HomaNative.TryLoad(out var error))
            return;

        var hint = string.IsNullOrWhiteSpace(configured)
            ? "Build it with 'cargo build --release -p homa-ffi' from rust/, or set HOMA_FFI_PATH."
            : $"HOMA_FFI_PATH is '{configured}'.";
        Assert.Fail($"libhoma_ffi did not load. {hint} {error}");
    }
}
