# Design: discovery telemetry wire format

Status: **proposed** · Owner: discovery-telemetry · Consumers: ark-discovery, discovery-scope · Date: 2026-06-02

Sharing telemetry from `ark-discovery` firmware to `discovery-scope` (and future
`*-discovery` boards / host tools) over USB CDC serial.

## Decision summary

1. **Binary is the primary format.** Firmware emits **postcard-encoded, COBS-framed**
   frames. The scope decodes these directly into its `Sample`/`Log` types — no string
   parsing, no fixed-decimal loss, no parser drift.
2. **Text stays as a debug fallback.** The firmware can toggle between binary and text
   output at runtime, so a plain terminal (`minicom`, `screen`) still shows human-readable
   lines. The scope opts into binary on connect; a human opening a terminal sees text.
3. **A new standalone repo, `discovery-telemetry`,** owns the wire types as a `no_std`
   crate. Both the firmware and the scope depend on it. It is the single source of truth
   for the wire format — neither side hand-writes a parser.
4. **Fusion lives in the firmware.** It already runs `fusion-ahrs` + `fusion-altitude` and
   computes roll/pitch/yaw/alt/vz. This closes the "where does fusion live" open question
   in the scope's CLAUDE.md: the host consumes a fused attitude frame, it does not refuse.

## Goals

- Lossless, typed transport of attitude + air-data + raw sensor frames.
- One definition of the wire format, compiled into both ends — impossible for the scope's
  decoder and the firmware's encoder to disagree.
- Cheap to extend (add a sensor / field) with explicit, checked version negotiation.
- Keep the board debuggable from a dumb terminal.
- `no_std` on the firmware (edition 2024, `heapless`); `std` ergonomics on the host.

## Non-goals

- Not a command/telemetry protocol with acks, retries, or flow control. USB CDC bulk
  transfers already give link-level CRC + retransmit; we add framing + optional integrity,
  not a reliability layer.
- Not bidirectional structured commands (yet). Host→board control stays single ASCII bytes
  (`r`, `d`, and the new `b`/`t`) — see [Mode negotiation](#mode-negotiation).
- Not a ground control station. Read-only scope, per the project charter.
- Not a full flight stack. The `discovery-*` series exists to **explore** embedded flight
  control (sensors, fusion, host visualization) — no control loops, no actuation.

## Current state (text, today)

The firmware streams human-readable ASCII lines, `\r\n`-terminated, ≤192 bytes, via direct
`serial.write(msg.as_bytes())` (`ark-discovery/src/main.rs:101`, `:115`). No framing, no
serialization crate. Emit points and rates (`ark-discovery/src/main.rs`, `config.rs`):

| Source | Sensor | Sample → log rate | Example line |
|---|---|---|---|
| Heartbeat | timer | 1 Hz | `hello from RTIC on STM32H743, tick 123` |
| IMU | IIM-42653 | 1000 → 10 Hz | `imu[100] id=0x56(exp 56) accel[g]=0.01,-0.02,-1.03 gyro[dps]=0.1,0.2,-0.3 temp=28.5C` |
| Baro | BMP388/390 | 25 → 5 Hz | `baro press=1013.25hPa temp=25.50C` |
| Mag | IIS2MDC | 50 → 5 Hz | `mag field[uT]=22.5,10.3,-45.2 temp=24.5C` |
| Fusion | AHRS+alt | 250 → 10 Hz | `fus roll=5.2 pitch=3.1 yaw=127.5deg alt=100.45m vz=0.12m/s` |

Aggregate ≈ 2 KB/s. Problems this design fixes: lossy fixed decimals, fragile substring
parsing, log lines interleaved with data, no version handshake, no timestamps.

The firmware structs that frames mirror 1:1 (units already SI-ish):

- `ImuSample { accel_g: [f32;3], gyro_dps: [f32;3], temp_c: f32 }` (`imu.rs:84`)
- `BaroSample { pressure_hpa: f32, temp_c: f32 }` (`baro.rs:113`)
- `MagSample { field_ut: [f32;3], temp_c: f32 }` (`mag.rs:82`)
- `FusedState { roll_deg, pitch_deg, yaw_deg, altitude_m, vertical_velocity, baro_residual }` (`fusion.rs:31`)

## The shared crate: `discovery-telemetry`

New standalone repo, expected as a sibling: `../discovery-telemetry`. `no_std` by default,
`std` opt-in for host conveniences.

```
discovery-telemetry/
  Cargo.toml
  src/
    lib.rs        # PROTOCOL_VERSION, re-exports, the Frame enum
    frames.rs     # Imu/Baro/Mag/Fused/Hello/Tick payload structs
    codec.rs      # encode (no_std) + a std decode accumulator wrapper
```

```toml
# discovery-telemetry/Cargo.toml
[package]
name = "discovery-telemetry"
version = "0.1.0"
edition = "2021"          # widest compiler support for a library both ends share

[features]
default = []
std = []                  # host: Display impls, Vec-based decode helpers

[dependencies]
serde   = { version = "1", default-features = false, features = ["derive"] }
postcard = { version = "1", default-features = false }
```

Why postcard: `no_std`, tiny, non-self-describing (compact — field names are not on the
wire), and ships a COBS accumulator for stream framing. Why a separate repo (not a
workspace member of the firmware): both firmware and scope are first-class consumers and
neither should have to vendor the other; the wire format is its own release unit with its
own `PROTOCOL_VERSION` semver. Consumers pin by git tag.

Dependency direction:

```
ark-discovery (no_std) ─┐
                        ├─► discovery-telemetry (no_std core)
discovery-scope (std) ──┘   (scope enables `std` feature)
```

## Wire format

### Layered

```
USB CDC bulk byte stream
  └─ COBS frame  (0x00 delimiter between frames; resync on any 0x00)
       └─ postcard bytes
            └─ Frame { ... }   // the versioned envelope
```

COBS (Consistent Overhead Byte Stuffing) gives self-synchronizing framing: a `0x00` byte
never appears inside an encoded frame, so it delimits frames and lets the decoder recover
after a mid-stream connect, garbage, or a dropped USB packet. postcard provides this via
`to_slice_cobs` (encode) and `CobsAccumulator` (decode) — we don't hand-roll it.

### Envelope

```rust
// discovery-telemetry/src/lib.rs
pub const PROTOCOL_VERSION: u16 = 1;   // bump on any breaking layout change

// discovery-telemetry/src/frames.rs
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
pub struct Frame {
    /// Board-monotonic timestamp, milliseconds since boot. Wraps at ~49.7 days.
    pub t_ms: u32,
    pub msg: Msg,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
pub enum Msg {        // postcard encodes the discriminant as a 1-byte varint
    Hello(Hello),     // 0 — sent on (re)connect / mode switch
    Tick(u32),        // 1 — heartbeat counter
    Imu(Imu),         // 2
    Baro(Baro),       // 3
    Mag(Mag),         // 4
    Fused(Fused),     // 5
}
```

Putting `t_ms` in the envelope (not per-payload) means every frame is timestamped —
essential for the datalog strip charts, which the text format can't provide. The scope
keys plots on `t_ms`, not host arrival time.

### Payloads (units explicit, mirror firmware structs)

```rust
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
pub struct Hello {
    pub proto: u16,        // == PROTOCOL_VERSION; scope rejects mismatch
    pub fw_git: [u8; 8],   // short firmware commit, ASCII, NUL-padded
    pub board: Board,      // ArkDiscovery, HolybroDiscovery, ...
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
pub struct Imu  { pub accel_g: [f32;3], pub gyro_dps: [f32;3], pub temp_c: f32 }
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
pub struct Baro { pub pressure_hpa: f32, pub temp_c: f32 }
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
pub struct Mag  { pub field_ut: [f32;3], pub temp_c: f32 }

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
pub struct Fused {
    pub roll_deg: f32,        // right-wing-down +
    pub pitch_deg: f32,       // nose-up +
    pub yaw_deg: f32,         // magnetic heading, 0..360
    pub altitude_m: f32,      // ISA barometric
    pub vertical_speed_mps: f32, // climb +
    pub baro_residual_m: f32, // baro innovation; diagnostic
}
```

`Fused` maps field-for-field onto the scope's `Sample` (`src/telemetry.rs:9`), minus the
diagnostic `baro_residual_m`. Conversion to display units (ft, ft/min) stays at the PFD
boundary, unchanged.

Frame sizes (postcard, pre-COBS): `Fused` ≈ 1 (disc) + 4 (t_ms varint) + 6×4 = ~29 B;
`Imu` ≈ 1 + 4 + 7×4 = ~33 B. COBS overhead ≤ 1 byte per 254. At current rates the binary
stream is ~1 KB/s, roughly half the text volume — and lossless.

### Integrity

USB CDC bulk transfers carry a link-layer CRC16 with hardware retransmit, so in-frame
corruption is already very unlikely; COBS handles frame *boundary* recovery. We therefore
ship v1 **without** an application CRC. If field testing shows corruption (e.g. flaky
cables), add a `crc32` field to `Frame` behind a `PROTOCOL_VERSION` bump rather than
retrofitting silently. Tracked as an open question below.

### Versioning rules

- `PROTOCOL_VERSION` is semver-ish for the wire: bump on any change that an old decoder
  would misread (reordering `Msg` variants, changing a field's type/order, removing a field).
- **Adding a new `Msg` variant at the end is non-breaking for the decoder** only if the
  decoder treats unknown discriminants as skippable — postcard will error on an unknown
  variant, so the scope's decode loop must catch per-frame decode errors, log, and resync
  on the next COBS delimiter rather than tearing down the stream. (See scope changes.)
- The `Hello` handshake is the hard gate: scope reads `Hello.proto`; if it != its compiled
  `PROTOCOL_VERSION` it shows a clear "firmware/scope protocol mismatch (fw vN, scope vM)"
  banner and stays in text-passthrough so the user still sees raw lines.

## Mode negotiation

Host→board control stays single ASCII bytes (extends the existing `r`/`d` dispatch in
`ark-discovery/src/main.rs:297`–`317`):

| Byte | Action |
|---|---|
| `r` | reboot to DFU (existing) |
| `d` | toggle diagnostics verbosity (existing) |
| `b` | **new** — switch output to **binary** frames |
| `t` | **new** — switch output to **text** lines |

- **Default at boot: text.** A human opening `screen /dev/cu.usbmodem*` sees readable lines
  with zero setup.
- **The scope sends `b` on connect**, then starts its COBS accumulator, discarding bytes
  until the first clean `0x00` delimiter (natural resync past the in-flight text). The
  firmware replies with a `Hello` frame as the first binary frame so the scope confirms the
  switch and checks the version.
- On scope disconnect the firmware may stay in binary; the next human can send `t`. (Or:
  firmware reverts to text on USB DTR drop — minor, decide during firmware impl.)

This keeps the toggle in the same spirit as the existing `d` diag flag: an atomic enum read
each emit tick selecting the encoder, no new task structure.

## Firmware changes (`ark-discovery`)

1. Add `discovery-telemetry` (git dep, pinned tag) to `Cargo.toml`; add `postcard`.
2. Introduce an `OutputMode { Text, Binary }` atomic (mirrors the existing `DIAG`
   `AtomicBool` at `main.rs:49`). `b`/`t` bytes set it in `usb_irq`.
3. At each existing emit site (`imu_log`, `baro_sample`, `mag_sample`, `fusion_step`,
   `log_tick`), branch on the mode: text path = today's `log_fmt`; binary path = build the
   `Frame`, `postcard::to_slice_cobs` into a `heapless::Vec<u8, N>` (N≈64), `serial.write`.
4. `t_ms` from the RTIC monotonic (`Mono::now()` → millis).
5. Send a `Hello` frame immediately on entering binary mode.

No change to sensor drivers, fusion, or task cadence — only the emit/encoding layer. The
192-byte text buffer and format strings stay for the text path.

## Scope changes (`discovery-scope`)

Builds the currently-unbuilt `serial.rs` (CLAUDE.md "Not built yet"):

1. Add `discovery-telemetry = { git = ..., features = ["std"] }` and `serialport`.
2. Serial thread (per CLAUDE.md architecture): open `--port`, write `b`, then loop reading
   into a `postcard::CobsAccumulator`. For each completed frame, `postcard::from_bytes` →
   `Frame`. On decode error, log + continue (resync on next delimiter) — never panic the
   thread on a bad frame.
3. Map frames → existing channel:
   - `Msg::Fused` → update the `Sample` the PFD renders (drop-in for synthetic).
   - `Msg::Imu/Baro/Mag/Tick` → datalog `Log` ring buffer + (later) `egui_plot` series,
     keyed on `Frame.t_ms`.
   - `Msg::Hello` → version-check; mismatch raises the banner.
4. GUI thread drains the channel each frame and `ctx.request_repaint()` — unchanged from
   the documented design. Synthetic fallback stays for "no port attached."

The PFD and `Sample` shape don't change; only the *source* of `Sample` flips from synthetic
to decoded `Fused`.

## Phasing

1. **Stand up `discovery-telemetry` repo** — frames + codec + round-trip unit tests
   (`encode → COBS → decode` equality). Tag `v0.1.0`.
2. **Firmware binary emit** behind the `b`/`t` toggle; text remains default. Verify with a
   throwaway host script that dumps decoded frames.
3. **Scope `serial.rs`** consuming binary; PFD driven by real `Fused`. Keep synthetic
   fallback.
4. **Datalog plots** on the raw `Imu/Baro/Mag` frames (`egui_plot`).

Each phase is independently shippable; the firmware text path means nothing regresses for
terminal users at any step. Per scope CLAUDE.md branching: feature branch off `main`, land
via PR — never commit to `main`.

## Open questions

- **Application CRC:** ship v1 without it (rely on USB CRC + COBS resync) or include a
  `crc32` from the start? Leaning without; revisit if field corruption appears.
- **Binary persistence across disconnect:** revert firmware to text on DTR drop, or stay
  binary until `t`? Affects the "next human at a terminal" experience.
- **Raw-frame rates on the wire:** keep firmware's current log-decimation (IMU 10 Hz, etc.)
  or raise now that frames are cheaper? Start by matching today's rates.
- **Endianness/float:** postcard is little-endian, both ends ARM/x86 LE — no concern, noted
  for completeness.

## Appendix: one `Fused` frame, end to end

```
firmware:  Frame { t_ms: 12345, msg: Msg::Fused(Fused{ roll:5.2, pitch:3.1, yaw:127.5,
                                                        alt:100.45, vz:0.12, resid:0.05 }) }
  postcard:  B9 60 ...                      (varint t_ms, disc=5, 6×f32 LE)  ~29 bytes
  COBS:      <stuffed bytes> 00             (trailing 0x00 delimiter)
USB CDC ──► scope CobsAccumulator ──► postcard::from_bytes ──► Frame
  scope:   Sample{ roll_deg:5.2, pitch_deg:3.1, yaw_deg:127.5,
                   altitude_m:100.45, vertical_speed_mps:0.12 }   // → PFD
```
