using System.Runtime.CompilerServices;

namespace Homa.Tests;

internal static class NativeBootstrap
{
    [ModuleInitializer]
    internal static void Init()
    {
        if (!string.IsNullOrWhiteSpace(Environment.GetEnvironmentVariable("HOMA_FFI_PATH")))
            return;

        if (TrySet(Environment.GetEnvironmentVariable("CARGO_TARGET_DIR")))
            return;

        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        for (var i = 0; i < 8 && dir is not null; i++, dir = dir.Parent)
        {
            if (TrySet(Path.Combine(dir.FullName, "rust", "target")))
                return;
        }
    }

    private static bool TrySet(string? targetDir)
    {
        if (string.IsNullOrWhiteSpace(targetDir))
            return false;

        foreach (var config in new[] { "release", "debug" })
        {
            foreach (var name in new[] { "libhoma_ffi.dylib", "libhoma_ffi.so", "homa_ffi.dll" })
            {
                var path = Path.Combine(targetDir, config, name);
                if (!File.Exists(path))
                    continue;
                Environment.SetEnvironmentVariable("HOMA_FFI_PATH", path);
                return true;
            }
        }

        return false;
    }
}
