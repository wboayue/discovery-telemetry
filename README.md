# discovery-telemetry

Shared wire-format types for the `discovery-*` flight-controller family. Single source of
truth for the bytes on the wire between firmware (`ark-discovery`, `no_std`) and host tools
(`discovery-scope`, `std`) — neither side hand-writes a parser.

Transport: **postcard** over **COBS** framing, on the board's USB CDC serial.

```text
USB CDC byte stream
  └─ COBS frame   (0x00 delimiter; self-synchronizing)
       └─ postcard bytes
            └─ Frame { t_ms, msg }
```

`Msg` is `Hello | Tick | Imu | Baro | Mag | Fused`. Payloads mirror the firmware sensor
structs 1:1; `Fused` (roll/pitch/yaw/alt/vspeed) drives the PFD. SI/degrees on the wire;
display-unit conversion stays at the host's display boundary.

## Use

```toml
# firmware (no_std)
discovery-telemetry = { git = "https://github.com/wboayue/discovery-telemetry", tag = "v0.1.0" }

# host (std)
discovery-telemetry = { git = "https://github.com/wboayue/discovery-telemetry", tag = "v0.1.0", features = ["std"] }
```

Encode (firmware):

```rust
use discovery_telemetry::{codec, Frame, Msg, Fused};
let mut buf = [0u8; codec::MAX_FRAME];
let wire = codec::encode(&Frame { t_ms, msg: Msg::Fused(fused) }, &mut buf)?;
serial.write(wire); // COBS-delimited
```

Decode (host serial thread):

```rust
let mut dec = codec::Decoder::new();
dec.push(&read_buf[..n], |frame| { /* update Sample / Log */ });
```

## Versioning

`PROTOCOL_VERSION` gates compatibility; the firmware reports it in the `Hello` frame and the
host rejects a mismatch. Only ever **append** `Msg` variants — reordering or removing fields
requires a version bump. See `docs/telemetry-protocol.md` in `discovery-scope` for the full
design.

## Develop

```bash
cargo build                 # no_std core
cargo test --features std   # round-trip + stream-resync tests
```

MIT licensed.
