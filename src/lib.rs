//! Shared telemetry wire format for the `discovery-*` flight-controller family.
//!
//! Single source of truth for the bytes on the wire between firmware (`ark-discovery`,
//! `no_std`) and host tools (`discovery-scope`, `std`). Neither side hand-writes a parser;
//! both compile these types.
//!
//! Layering (see `docs/telemetry-protocol.md` for the full design):
//!
//! ```text
//! USB CDC byte stream
//!   └─ COBS frame   (0x00 delimiter; self-synchronizing)
//!        └─ postcard bytes
//!             └─ Frame { t_ms, msg }
//! ```
//!
//! Encode is `no_std` (firmware). Decode via [`codec::Decoder`] is `no_std` too; the `std`
//! feature adds an alloc-based convenience ([`codec::decode_all`]).
//!
//! All angular/SI units are explicit in field names. Floats are little-endian (postcard);
//! both ARM and x86 hosts are LE, so no byte-swap is needed.

#![no_std]

#[cfg(feature = "std")]
extern crate std;

pub mod codec;
pub mod frames;

/// Wire-format version. Bump on any change an old decoder would misread: reordering
/// [`frames::Msg`] variants, changing a field's type/order, or removing a field.
/// Appending a new `Msg` variant is decoder-safe *only* because [`codec::Decoder`] skips
/// and resyncs on frames it can't deserialize. Negotiated via the [`frames::Hello`] frame.
///
/// v2 appends [`frames::Msg::Status`] (status/event lines).
pub const PROTOCOL_VERSION: u16 = 2;

pub use frames::{Baro, Board, Frame, Fused, Hello, Imu, Level, Mag, Msg, Status};
