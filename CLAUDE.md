# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`discovery-telemetry` is the single source of truth for the bytes on the wire between
`discovery-*` flight-controller firmware (`ark-discovery`, `no_std`) and host tools
(`discovery-scope`, `std`). Both ends compile these types; neither hand-writes a parser.
It is a small `no_std` library crate — there is no binary.

## Commands

```bash
cargo build                 # no_std core (default, no features)
cargo test --features std   # all tests — they live in src/codec.rs and need std
cargo test --features std roundtrip_each_variant   # a single test by name
cargo clippy --all-targets --features std
```

Tests are `#[cfg(test)]` in `src/codec.rs` and use `extern crate std`, so they **only build
with `--features std`**. `cargo test` alone (no_std) compiles nothing useful.

## Architecture

Three layers, smallest surface possible:

- `frames.rs` — the wire types. `Frame { t_ms, msg }` is the envelope; `Msg` is the
  payload enum (`Hello | Tick | Imu | Baro | Mag | Fused`). Payloads mirror the firmware
  sensor structs 1:1 so the firmware's emit code is a field copy.
- `codec.rs` — framing. `encode` (no_std) writes a `Frame` as postcard-over-COBS into a
  caller buffer. `Decoder` (no_std) is the real-time streaming path: feed raw serial bytes,
  get a callback per valid frame, auto-resync past garbage. `decode_all` (std) is a
  batch/test convenience.
- `lib.rs` — `#![no_std]` root, re-exports, and `PROTOCOL_VERSION`.

Wire stack: `USB CDC bytes → COBS frame (0x00 delim) → postcard bytes → Frame`.

## Wire-format invariants (the whole point of this crate)

These rules are why the crate exists — breaking one silently corrupts both consumers:

- **Append-only `Msg` variants.** Never reorder or remove variants/fields. postcard encodes
  the enum discriminant positionally as a varint, so order *is* the format. Appending a new
  variant is safe only because `Decoder` skips frames it can't deserialize and resyncs.
- **Bump `PROTOCOL_VERSION` (lib.rs:34)** on any change an old decoder would misread:
  reordering variants, changing a field's type/order, removing a field. Appending a variant
  does not require a bump. The version is negotiated via the `Hello` frame; the host rejects
  a mismatch.
- **SI / degrees on the wire.** No display-unit conversion here (ft, ft/min stay at the
  host PFD boundary). Units are explicit in every field name.
- **`MAX_FRAME = 64`** (codec.rs) bounds one COBS frame; firmware encode buffers and the
  decoder's accumulator are sized to it. Re-derive if a larger payload is added.

When changing `frames.rs`, the round-trip tests in `codec.rs` should cover every `Msg`
variant — add a case to `roundtrip_each_variant` for any new one.

## Reference

`docs/telemetry-protocol.md` is the full design doc (rationale: why binary, why postcard +
COBS, mode negotiation, version handling). Read it before changing the protocol shape.
