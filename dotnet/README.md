# dotnet/ - Phase 2 and 3 (not started)

[![dotnet-ci](https://github.com/kearns2000/homastack/actions/workflows/dotnet-ci.yml/badge.svg?branch=main)](https://github.com/kearns2000/homastack/actions/workflows/dotnet-ci.yml)
![.NET 10 planned](https://img.shields.io/badge/.NET-10%20%28planned%29-512BD4?logo=dotnet&logoColor=white)
![Status: Phase 2 not started](https://img.shields.io/badge/status-Phase%202%20not%20started-lightgrey)
![Interop: LibraryImport over homa-ffi](https://img.shields.io/badge/interop-LibraryImport%20%E2%86%92%20homa--ffi-1f6feb)

Phase 1 deliberately contains no .NET code. This tree reserves the layout that
`Homa.NET` (managed wrapper over `homa-ffi`), `Homa.Rpc` and `Homa.Rpc.Hosting`
will use:

```
dotnet/
├── HomaStack.slnx
├── Directory.Build.props
├── Directory.Packages.props
├── src/
│   ├── Homa.NET/            # HomaSocket, HomaClient, Native/HomaNative.cs (LibraryImport)
│   ├── Homa.Rpc/            # HomaRpcClient, HomaRpcServer
│   └── Homa.Rpc.Hosting/    # ServiceExtensions (DI / IHostedService)
└── tests/
    ├── Homa.NET.Tests/
    └── Homa.Rpc.Tests/
```

The planned native surface is described in `docs/native-interop.md`. The
`dotnet-ci.yml` workflow is a no-op until `HomaStack.slnx` exists.
