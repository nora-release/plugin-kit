# Process plugin protocol

A process plugin is a program the host starts and talks to over its stdin and
stdout. This page is the wire contract between them; the SDKs in this
repository implement the plugin side of it:

| SDK | Where | Version 2 (streams) |
| --- | --- | --- |
| Rust | [`crates/nora-plugin-runtime-sdk`](crates/nora-plugin-runtime-sdk) | yes |
| Go | [`sdk-go`](sdk-go) (`github.com/nora-release/plugin-kit/sdk-go`) | yes |
| Swift | [`sdk-swift`](sdk-swift) | yes |

The shared message types are [`crates/nora-plugin-protocol`](crates/nora-plugin-protocol).
How a host supervises plugins (grants, restarts, which manifest fields it
reads) is the host's business; Nora's is described in its own repository.

## Framing

- One JSON-RPC 2.0 message per line (JSONL), UTF-8, no line breaks inside a
  message.
- A frame is at most 1 MiB unless the host and plugin agree on another limit.
  A longer frame is a protocol error.
- At most 32 requests are in flight in each direction by default; a request
  past the limit is answered with error `-32002`.

## Version 1

| Message | Direction | Params | Result |
| --- | --- | --- | --- |
| `initialize` | host → plugin | `{protocolVersion, pluginId, config, grantedServices}` | `{protocolVersion, tools, services}` |
| `tools/call` | host → plugin | `{name, arguments}` | `{content, isError, details?}` |
| `services/call` | host → plugin | `{service, method, arguments}` | any JSON value |
| `services/call` | plugin → host | `{service, method, arguments, parentRequestId}` | any JSON value |
| `notifications/cancelled` | either | `{requestId}` | (notification) |
| `shutdown` | host → plugin | none | `null`, then the plugin exits |

- `initialize` comes first and once. `tools` are `{name, description,
  inputSchema}`; `services` are the names of the services the plugin
  provides. A host may check both against the plugin's manifest and refuse
  a plugin that publishes something else.
- `content` blocks are `{"type": "text", "text"}` or `{"type": "image",
  "data", "mimeType"}`.
- A plugin calls a host service only while one of the host's calls to it is
  running, and names that call in `parentRequestId`. It may call only the
  services the host granted in `grantedServices`; anything else is answered
  with `-32003`. The SDKs bind `parentRequestId` themselves: a plugin cannot
  pick another.
- `notifications/cancelled` names a request the sender made. The receiver
  stops working on it and may answer it with `-32800`; a reply that crosses
  the cancellation on the wire is ignored. Request ids are never reused.

Error codes in use:

| Code | Meaning |
| --- | --- |
| `-32601` | method or service not supported |
| `-32602` | invalid parameters |
| `-32002` | too many requests in flight |
| `-32003` | no grant for the service |
| `-32004` | unsupported protocol version |
| `-32800` | the request was cancelled |

## Version 2: streams

Version 2 lets a call carry streams of bytes beside its arguments and its
result, for an upstream reply that streams in and a request body past the
frame limit. Everything in version 1 stays as it is.

### Version

- The plugin's manifest says which version it speaks (in Nora,
  `runtime.protocol`; 1 when left out). The host sends that version in
  `initialize`, and the plugin answers the same one, or it fails to start.
  A host never offers 2 to a plugin that didn't ask for it, so version 1
  plugins and SDKs keep working unchanged.
- The SDKs speak both and answer the version the host gave.
- In version 2, `initialize`'s params and result each carry
  `streamWindow`: the credit, in bytes, the sender gives each stream it
  reads. Between 64 KiB and 64 MiB; 1 MiB when left out.

### Streams and calls

- `tools/call` and `services/call` take optional `inputStream` and
  `outputStream`: stream ids. The caller writes `inputStream`; the callee
  writes `outputStream`.
- The side that sends the call allocates its streams' ids: the host odd
  ones, the plugin even ones, increasing within a connection and never
  reused.
- The receiver of a call takes its streams as it reads the call, before it
  reads the next frame: the call's first data may come right behind it, in
  the same write. A receiver that took them only once it got round to the
  call would see that data as for an unknown stream.
- A stream belongs to the call that named it. The call is done once it is
  answered and all its streams have ended; until then it holds its place
  among the requests in flight and its deadline.
- The callee may answer first (say an HTTP status and headers) and go on
  writing `outputStream`, or write it all and then answer.
- Once answered, the caller stops writing `inputStream`: a callee may answer
  without reading all of its input, as an HTTP server may answer without
  reading the whole request body.
- A caller that no longer wants `outputStream` cancels the call. A callee
  that no longer reads `inputStream` should answer soon.
- `notifications/cancelled` for a call closes all its streams; both sides
  drop what they hold of them. There is no message that closes one stream
  alone.

### Messages

| Message | Params | |
| --- | --- | --- |
| `streams/data` | `{stream, data?, end?, error?}` | `data` is base64. `end: true` ends the stream. `error` comes only with `end`: the writer gave up, and the reader takes it as its read's error. |
| `streams/credit` | `{stream, bytes}` | The reader takes `bytes` more. |

### Credit

- Each `streams/data` costs its decoded bytes plus 1024, so a flood of tiny
  frames cannot fill the reader's queue. A frame that ends the stream and
  carries no data is free: there is one per stream.
- A writer waits for credit rather than drop data. It splits data to fit the
  frame limit (the SDKs put at most 256 KiB of raw bytes in a frame), and
  when its credit has no room for a whole chunk it sends what fits rather
  than wait for more.
- A reader returns credit at the latest once what it has read and not yet
  returned reaches half its window.
- Together these keep the two from waiting on each other: while the reader
  holds back less than half the window, the writer has at least half of it.

### Errors

These are protocol errors and end the connection: a stream id never
allocated; data from a stream's reader, or credit from its writer; data past
the credit; `error` without `end`; any stream message in a version 1
connection.

A message for a stream that ended, or whose call was cancelled or failed
(its id was allocated), is ignored: cancellation and data can cross on the
wire.

### Scope

The SDKs read and write the streams of calls the host makes to a plugin. A
plugin's own calls to host services take no streams yet: a host answers such
a call naming a stream with `-32602`.
