# Changelog

All notable changes to `discovery-telemetry` are documented here.
Format loosely follows [Keep a Changelog](https://keepachangelog.com/); the crate follows
semver, and `PROTOCOL_VERSION` tracks wire compatibility independently.

## [Unreleased]

## [0.2.0] - 2026-06-02

Adds a status/event message so the firmware's non-data lines (mode acks, sensor bring-up
identity, errors) have a binary form. `PROTOCOL_VERSION` → 2.

### Added

- `Msg::Status` (appended — decoder-safe) carrying `Status { level: Level, text: [u8; 48] }`.
  `Level = Info | Warn | Error`. The binary equivalent of the firmware's text status lines;
  without it those lines would be dropped as garbage in a binary-mode stream.
- Re-exports of `Status` and `Level`.

### Changed

- `PROTOCOL_VERSION` 1 → 2 (wire surface grew; `Hello.proto` advertises it).
- `MAX_FRAME` unchanged at 64, but `Status` is now the largest payload (~57 B framed);
  doc-comment updated.

## [0.1.0] - 2026-06-02

First tagged release. Wire-format design is **accepted** — this is the format the
`discovery-*` series uses (`PROTOCOL_VERSION = 1`).

### Added

- `Frame { t_ms, msg }` envelope with a board-monotonic millisecond timestamp.
- `Msg` payload enum: `Hello | Tick | Imu | Baro | Mag | Fused`. Payloads mirror the
  firmware sensor structs 1:1 (`Imu`, `Baro`, `Mag`, `Fused`, `Hello`/`Board`).
- `codec::encode` — `no_std` postcard-over-COBS encode into a caller buffer.
- `codec::Decoder` — `no_std` streaming decoder; resyncs past garbage and version-skew
  frames at the next `0x00` delimiter instead of tearing down the stream.
- `codec::decode_all` — `std`-only batch convenience.
- `codec::MAX_FRAME = 64` — upper bound on one COBS-framed frame.
- `PROTOCOL_VERSION = 1`, negotiated via the `Hello` frame; host rejects a mismatch.
- `std` feature flag for host conveniences; core stays `no_std`.

### Notes

- Transport: postcard over COBS framing on USB CDC serial. SI/degrees on the wire;
  display-unit conversion stays at the host boundary.
- Compatibility contract: only ever **append** `Msg` variants. Reordering/removing
  variants or fields requires a `PROTOCOL_VERSION` bump. See
  [`docs/telemetry-protocol.md`](docs/telemetry-protocol.md).

[0.1.0]: https://github.com/wboayue/discovery-telemetry/releases/tag/v0.1.0
