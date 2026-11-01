# dotnet/

[![dotnet-ci](https://github.com/kearns2000/homastack/actions/workflows/dotnet-ci.yml/badge.svg?branch=main)](https://github.com/kearns2000/homastack/actions/workflows/dotnet-ci.yml)
![.NET 10](https://img.shields.io/badge/.NET-10-512BD4?logo=dotnet&logoColor=white)
![Status: Phase 4 Homa.Rpc](https://img.shields.io/badge/status-Phase%204%20Homa.Rpc-0ea5e9)
![Interop: LibraryImport over homa-ffi](https://img.shields.io/badge/interop-LibraryImport%20over%20homa--ffi-1f6feb)

`Homa.NET` (Phase 3) is the managed wrapper over `libhoma_ffi`. `Homa.Rpc` and
`Homa.Rpc.Hosting` (Phase 4) are the RPC layer on top of it: framing, dispatch,
middleware, and DI / `IHostedService`.

```
dotnet/
├── HomaStack.slnx
├── Directory.Build.props
├── src/
│   ├── Homa.NET/            # HomaSocket, HomaClient, HomaMessage, Native/HomaNative.cs
│   ├── Homa.Rpc/            # HomaRpcClient, HomaRpcServer, RpcPipeline
│   └── Homa.Rpc.Hosting/    # AddHomaRpc, AddHomaRpcServer, IHostedService
└── tests/
    ├── Homa.NET.Tests/
    └── Homa.Rpc.Tests/
```

## Build and test

The native library has to exist for the `Homa.NET` tests. The `Homa.Rpc` tests
do not open a socket. None of these tests need `homa.ko`.

```bash
cd rust && cargo build --release -p homa-ffi
cd ../dotnet
export HOMA_FFI_PATH="$PWD/../rust/target/release/libhoma_ffi.so"   # .dylib on macOS
dotnet test HomaStack.slnx
```

If `HOMA_FFI_PATH` is unset, the `Homa.NET` test run looks under
`rust/target/{release,debug}` for `libhoma_ffi`. An application looks at
`HOMA_FFI_PATH`, then `runtimes/<rid>/native/` beside the app, then the default
`homa_ffi` probe.

On macOS, `HomaSocket.Open` throws `HomaException` with status
`UnsupportedPlatform`. On Linux without the module it throws `Unavailable`.
A successful open, and any RPC on a real socket, still requires Linux with
`homa.ko`. That path has not been executed.

## Transport

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

## RPC

A call is one Homa request. The payload is a version-1 frame (ASCII `HOMA`,
method name, status, serializer bytes). `HomaRpcClient.CallAsync` cancels
through `Homa.NET`. A null timeout waits 30 seconds. `HomaRpcServer` runs
handlers on the thread pool, 128 at a time, and replies to shared receives.
Hosting registers handlers in DI, serializes the response inside the call
scope, and runs the server as an `IHostedService`. `Bind` and
`HomaRpcCaller.Open` need `homa.ko`.

```csharp
using System.Net;
using Homa.Rpc;

var pipeline = new RpcPipelineBuilder()
    .Map<string, string>("echo", (text, _) => new ValueTask<string>(text))
    .Build(new BytesRpcSerializer());

await using var server = HomaRpcServer.Bind(
    new IPEndPoint(IPAddress.Any, 4000), pipeline);
```

The packages are not packed. The HomaStack licence has not been selected.
The native contract is [docs/native-interop.md](../docs/native-interop.md).
The frame is described in [docs/architecture.md](../docs/architecture.md).
