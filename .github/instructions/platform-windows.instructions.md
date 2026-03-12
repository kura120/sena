---
applyTo: "platform/windows/**"
---

# platform/windows — Copilot Instructions

The Windows platform layer is a C#/.NET process that provides OS-level integrations for Sena on Windows. It interfaces with WinRT and Win32 APIs and communicates with daemon-bus exclusively via gRPC. It is a thin integration layer — it collects OS signals and executes OS actions. It never makes decisions.

These rules are traps specific to this subsystem. Global rules in `.github/copilot-instructions.md` also apply in full.

---

## Ownership Boundaries

platform/windows owns:
- File system monitoring and access
- Screen capture and visual context extraction
- App and process lifecycle (launch, close, monitor)
- Camera and microphone access
- Browser integration hooks
- Keyboard and mouse automation
- Translating OS events into gRPC messages sent to daemon-bus

platform/windows does not own:
- Any decision-making about what to do with OS data — that is agents
- Any memory reads or writes
- Any model calls
- Any UI — that is the Freya ui subsystem
- Any business logic — this layer observes and executes only

---

## C# / .NET Traps

### Async All the Way — Never .Result or .Wait()
Never block an async operation with `.Result` or `.Wait()`. This causes deadlocks in the WinRT message loop. Always `await`.

```csharp
// bad — deadlocks
var result = GetScreenContentAsync().Result;

// good
var result = await GetScreenContentAsync();
```

### WinRT Exceptions Are COMException — Always Translate
WinRT errors surface as `COMException`. Never let a `COMException` propagate outside this subsystem. Always catch and translate to a Sena error type.

```csharp
// bad — raw COMException propagates
var content = await captureElement.CaptureAsync();

// good — translated
try {
    var content = await captureElement.CaptureAsync();
}
catch (COMException ex) {
    throw new SenaOsException(
        OsErrorCode.ScreenCaptureFailed,
        ex.Message,
        ex
    );
}
```

### Always Dispose IDisposable Resources
Always use `using` blocks for `IDisposable` OS handles. Never rely on GC for cleanup — OS handles leak silently and are never collected promptly.

```csharp
// bad — handle leaks
var handle = OpenFileHandle(path);
var content = await ReadAsync(handle);

// good
using var handle = OpenFileHandle(path);
var content = await ReadAsync(handle);
```

### Never Suppress Warnings With #pragma
If a compiler warning appears, fix the root cause. Never use `#pragma warning disable`. Warnings in OS integration code are frequently signs of real problems.

---

## gRPC Client Traps

### This Layer Is a gRPC Client — Never a Server
platform/windows only sends events to and receives commands from daemon-bus. It never hosts a gRPC server.

### Always Use the Proto-Generated Client — Never Raw HTTP
All gRPC calls use the generated client from daemon-bus/proto definitions. Never construct gRPC calls manually.

```csharp
// bad — manual HTTP
var client = new HttpClient();
await client.PostAsync("http://localhost:50051/...", payload);

// good — generated client
var response = await _daemonBusClient.PublishOsEventAsync(
    new OsEventRequest { EventType = OsEventType.FileChanged, ... }
);
```

### Handle gRPC Disconnection Gracefully
daemon-bus may restart during error recovery. The platform layer must handle gRPC disconnection and reconnect automatically without crashing.

```csharp
// good — retry on disconnect
private async Task PublishWithRetry(OsEventRequest request) {
    for (int attempt = 0; attempt < 3; attempt++) {
        try {
            await _client.PublishOsEventAsync(request);
            return;
        }
        catch (RpcException ex) when (ex.StatusCode == StatusCode.Unavailable) {
            await Task.Delay(TimeSpan.FromSeconds(Math.Pow(2, attempt)));
            await ReconnectAsync();
        }
    }
    _logger.LogError("daemon_bus_unreachable_after_retries");
}
```

---

## OS Integration Traps

### All Input Is Sanitized Before Forwarding
Any data received from the OS (file paths, process names, screen content) must be sanitized before being sent to daemon-bus. Never forward raw OS input.

### Screen Capture Requires User Consent at First Use
Screen capture must check for and request user permission before first use on Windows. Never attempt screen capture without confirming permission is granted.

```csharp
// good — check permission first
var access = await GraphicsCaptureAccess.RequestAccessAsync(
    GraphicsCaptureAccessKind.Programmatic
);
if (access != AppCapabilityAccessStatus.Allowed) {
    throw new SenaOsException(OsErrorCode.PermissionDenied, "Screen capture permission denied");
}
```

### Peripheral Automation Is Rate-Limited
Keyboard and mouse automation events must be rate-limited. Never fire automation events in a tight loop without a minimum delay between actions.

---

## Logging

Use `Microsoft.Extensions.Logging` with structured logging. Never use `Console.WriteLine`. Required fields on every log event:

- `subsystem` — always "platform.windows"
- `event_type` — os_event_received, grpc_published, permission_requested, error
- `os_integration` — file, screen, process, browser, peripheral, camera, microphone

Errors from WinRT/Win32 must be logged at `Error` level with the `COMException` HRESULT code included.
