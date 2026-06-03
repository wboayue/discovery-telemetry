//! Frame envelope and payload types. These mirror the firmware's sensor structs 1:1
//! (`ImuSample`/`BaroSample`/`MagSample`/`FusedState`) so emit code is a field copy.

use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

/// The versioned envelope. Every frame carries a board-monotonic timestamp so host plots
/// key on capture time, not host arrival time.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// Milliseconds since board boot. Wraps at ~49.7 days (`u32` ms).
    pub t_ms: u32,
    pub msg: Msg,
}

/// One telemetry message. postcard encodes the variant discriminant as a 1-byte varint.
///
/// Variant order is part of the wire format — only ever append, never reorder or remove
/// without bumping [`crate::PROTOCOL_VERSION`].
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Msg {
    /// Sent first on entering binary mode / on (re)connect.
    Hello(Hello),
    /// Heartbeat counter (the old `tick N` line).
    Tick(u32),
    /// Raw 6-axis IMU sample.
    Imu(Imu),
    /// Barometric pressure + temperature.
    Baro(Baro),
    /// Magnetometer field + temperature.
    Mag(Mag),
    /// Fused attitude + air data — drives the PFD.
    Fused(Fused),
    /// Human-readable status / event line (mode acks, bring-up identity, errors). The binary
    /// home for the firmware's non-data text lines so they survive a binary-mode stream.
    Status(Status),
}

/// Connection handshake / version gate.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Hello {
    /// Equals [`crate::PROTOCOL_VERSION`] on the firmware. Host rejects a mismatch.
    pub proto: u16,
    /// Short firmware commit hash, ASCII, NUL-padded.
    pub fw_git: [u8; 8],
    pub board: Board,
}

/// Which board emitted the stream (family, not one board — see project charter).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Board {
    ArkDiscovery,
    HolybroDiscovery,
    /// Reported by a board the host build doesn't know about.
    Unknown,
}

/// Raw IMU sample. Mirrors firmware `ImuSample` (IIM-42653, ±16 g / ±2000 dps).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Imu {
    /// X, Y, Z acceleration in g (gravitational units).
    pub accel_g: [f32; 3],
    /// X, Y, Z angular rate in degrees/second.
    pub gyro_dps: [f32; 3],
    /// Die temperature, Celsius.
    pub temp_c: f32,
}

/// Barometer sample. Mirrors firmware `BaroSample` (BMP388/390).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Baro {
    /// Atmospheric pressure, hectopascals.
    pub pressure_hpa: f32,
    /// Die temperature, Celsius.
    pub temp_c: f32,
}

/// Magnetometer sample. Mirrors firmware `MagSample` (IIS2MDC/LIS2MDL).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Mag {
    /// X, Y, Z magnetic field, microtesla.
    pub field_ut: [f32; 3],
    /// Die temperature, Celsius.
    pub temp_c: f32,
}

/// Fused attitude + air data. Mirrors firmware `FusedState`; maps field-for-field onto the
/// scope's `Sample` (minus the diagnostic `baro_residual_m`). Conversion to display units
/// (ft, ft/min) stays at the PFD boundary — this is SI/degrees on the wire.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Fused {
    /// Roll, right-wing-down positive. Degrees.
    pub roll_deg: f32,
    /// Pitch, nose-up positive. Degrees.
    pub pitch_deg: f32,
    /// Yaw / magnetic heading, clockwise from north. Degrees, 0..360.
    pub yaw_deg: f32,
    /// Barometric altitude (ISA). Metres.
    pub altitude_m: f32,
    /// Vertical speed, climb positive. Metres/second.
    pub vertical_speed_mps: f32,
    /// Baro innovation (filter residual). Metres. Diagnostic.
    pub baro_residual_m: f32,
}

/// Severity of a [`Status`] line. Lets the host route/colour without parsing the text.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

/// A human-readable status / event line — the binary equivalent of the firmware's non-data
/// text output (mode acks like `diag on`, sensor bring-up identity, error/warning conditions).
/// Without this the firmware's status lines have no binary form; in binary mode they would be
/// injected as raw text and dropped by the decoder. `text` is fixed-size so the frame stays
/// `Copy` and `no_std`-friendly; it bounds the largest frame (see [`crate::codec::MAX_FRAME`]).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Status {
    pub level: Level,
    /// ASCII message, NUL-padded. Trailing NULs are not part of the text.
    #[serde(with = "BigArray")]
    pub text: [u8; 48],
}
