# dotnet/

[![dotnet-ci](https://github.com/kearns2000/homastack/actions/workflows/dotnet-ci.yml/badge.svg?branch=main)](https://github.com/kearns2000/homastack/actions/workflows/dotnet-ci.yml)
![.NET 10](https://img.shields.io/badge/.NET-10-512BD4?logo=dotnet&logoColor=white)
![Status: Phase 3 Homa.NET](https://img.shields.io/badge/status-Phase%203%20Homa.NET-0ea5e9)
![Interop: LibraryImport over homa-ffi](https://img.shields.io/badge/interop-LibraryImport%20over%20homa--ffi-1f6feb)

`Homa.NET` (Phase 3) is the managed wrapper over `libhoma_ffi`. `Homa.Rpc` and
`Homa.Rpc.Hosting` (Phase 4) are still empty.

```
dotnet/
├── HomaStack.slnx
├── Directory.Build.props
├── src/
│   ├── Homa.NET/            # HomaSocket, HomaClient, HomaMessage, Native/HomaNative.cs
│   ├── Homa.Rpc/            # not started
│   └── Homa.Rpc.Hosting/    # not started
└── tests/
    ├── Homa.NET.Tests/
    └── Homa.Rpc.Tests/      # not started
```

## Build and test

The native library has to exist first. These tests do not need `homa.ko`.

```bash
cd rust && cargo build --release -p homa-ffi
cd ../dotnet
export HOMA_FFI_PATH="$PWD/../rust/target/release/libhoma_ffi.so"   # .dylib on macOS
dotnet test HomaStack.slnx
```

If `HOMA_FFI_PATH` is unset, the test run looks under `rust/target/{release,debug}`
for `libhoma_ffi`. An application looks at `HOMA_FFI_PATH`, then
`runtimes/<rid>/native/` beside the app, then the default `homa_ffi` probe.

On macOS, `Open` throws `HomaException` with status `UnsupportedPlatform`.
On Linux without the module it throws `Unavailable`. A successful open, and
any RPC, still requires Linux with `homa.ko`. That path has not been executed.

## Using it

```csharp
using System.Net;
using Homa;

using var client = new HomaClient();
using var response = client.Request(
    new IPEndPoint(IPAddress.Parse("127.0.0.1"), 5000),
    "Hello Homa"u8);
```

`RequestAsync` does the same wait on a thread-pool thread and aborts that RPC
if the `CancellationToken` fires. `HomaSocket` is the full surface (`Bind`,
`Recv`, `Reply`, `Abort`, `Shutdown`). Messages own a copied payload:
`Payload` is valid until `Dispose`, and `CopyPayload` returns a `byte[]`.

The package is not packed. The HomaStack licence has not been selected.
The native contract is [docs/native-interop.md](../docs/native-interop.md).
