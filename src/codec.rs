//! Framing codec: postcard + COBS.
//!
//! - [`encode`] is `no_std` — firmware encodes a [`Frame`] into a caller-supplied buffer.
//! - [`Decoder`] is `no_std` — host (or firmware) feeds raw stream bytes and gets frames,
//!   skipping and resyncing past corruption or a mid-stream connect.
//! - [`decode_all`] (feature `std`) is an alloc-based convenience for batch/tests.

use crate::frames::Frame;
use postcard::accumulator::{CobsAccumulator, FeedResult};

/// Upper bound on one COBS-framed frame, bytes. Largest payload is [`crate::Status`]
/// (`[u8;48]` + level disc + msg disc + varint `t_ms`) ≈ 55 B; COBS adds ≤1 B per 254 plus a
/// delimiter → ≈ 57 B. 64 B keeps ~7 B headroom — size firmware encode buffers and the
/// [`Decoder`] to this. (Growing `Status::text` past ~54 bytes would breach this bound.)
pub const MAX_FRAME: usize = 64;

/// Encode `frame` as a COBS-delimited postcard packet into `buf` (recommended `MAX_FRAME`).
/// Returns the used slice, including the trailing `0x00` delimiter. `Err` only if `buf` is
/// too small.
pub fn encode<'a>(frame: &Frame, buf: &'a mut [u8]) -> postcard::Result<&'a mut [u8]> {
    postcard::to_slice_cobs(frame, buf)
}

/// Streaming decoder. Wraps a COBS accumulator sized to [`MAX_FRAME`]; an oversized or
/// malformed frame is dropped and decoding resyncs at the next `0x00` delimiter rather than
/// tearing down the stream — so appending a future `Msg` variant an old build can't read
/// degrades to "skip that frame", not "lose the connection".
pub struct Decoder {
    acc: CobsAccumulator<MAX_FRAME>,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub const fn new() -> Self {
        Self {
            acc: CobsAccumulator::new(),
        }
    }

    /// Feed a chunk of stream bytes; call `on_frame` once per complete, valid frame.
    /// Returns the count of dropped frames (overfull buffer or deserialize error) seen in
    /// this chunk — non-zero means corruption or a version-skew frame was skipped.
    pub fn push(&mut self, mut bytes: &[u8], mut on_frame: impl FnMut(Frame)) -> usize {
        let mut dropped = 0;
        while !bytes.is_empty() {
            bytes = match self.acc.feed::<Frame>(bytes) {
                FeedResult::Consumed => &[],
                FeedResult::OverFull(rest) => {
                    dropped += 1;
                    rest
                }
                FeedResult::DeserError(rest) => {
                    dropped += 1;
                    rest
                }
                FeedResult::Success { data, remaining } => {
                    on_frame(data);
                    remaining
                }
            };
        }
        dropped
    }
}

/// Decode every complete frame in `bytes` into a `Vec`. Convenience for host batch use and
/// tests; the streaming [`Decoder`] is the real-time path.
#[cfg(feature = "std")]
pub fn decode_all(bytes: &[u8]) -> std::vec::Vec<Frame> {
    let mut out = std::vec::Vec::new();
    let mut dec = Decoder::new();
    dec.push(bytes, |f| out.push(f));
    out
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::frames::{Baro, Board, Fused, Hello, Imu, Level, Mag, Msg, Status};
    use crate::PROTOCOL_VERSION;
    use std::vec::Vec;

    fn roundtrip(frame: Frame) {
        let mut buf = [0u8; MAX_FRAME];
        let wire = encode(&frame, &mut buf).expect("encode fits");
        // COBS frames end in the 0x00 delimiter and contain no interior zeros.
        assert_eq!(*wire.last().unwrap(), 0x00);
        assert!(wire[..wire.len() - 1].iter().all(|&b| b != 0x00));

        let mut got = Vec::new();
        let mut dec = Decoder::new();
        let dropped = dec.push(wire, |f| got.push(f));
        assert_eq!(dropped, 0);
        assert_eq!(got, std::vec![frame]);
    }

    #[test]
    fn roundtrip_each_variant() {
        roundtrip(Frame {
            t_ms: 0,
            msg: Msg::Hello(Hello {
                proto: PROTOCOL_VERSION,
                fw_git: *b"7c44b5e\0",
                board: Board::ArkDiscovery,
            }),
        });
        roundtrip(Frame {
            t_ms: 12345,
            msg: Msg::Tick(42),
        });
        roundtrip(Frame {
            t_ms: 1,
            msg: Msg::Imu(Imu {
                accel_g: [0.01, -0.02, -1.03],
                gyro_dps: [0.1, 0.2, -0.3],
                temp_c: 28.5,
            }),
        });
        roundtrip(Frame {
            t_ms: 2,
            msg: Msg::Baro(Baro {
                pressure_hpa: 1013.25,
                temp_c: 25.5,
            }),
        });
        roundtrip(Frame {
            t_ms: 3,
            msg: Msg::Mag(Mag {
                field_ut: [22.5, 10.3, -45.2],
                temp_c: 24.5,
            }),
        });
        roundtrip(Frame {
            t_ms: 12345,
            msg: Msg::Fused(Fused {
                roll_deg: 5.2,
                pitch_deg: 3.1,
                yaw_deg: 127.5,
                altitude_m: 100.45,
                vertical_speed_mps: 0.12,
                baro_residual_m: 0.05,
            }),
        });
        // Status with a full-width text payload exercises the largest frame against MAX_FRAME.
        let mut text = [0u8; 48];
        text[..11].copy_from_slice(b"baro BMP390");
        roundtrip(Frame {
            t_ms: u32::MAX,
            msg: Msg::Status(Status {
                level: Level::Error,
                text,
            }),
        });
    }

    #[test]
    fn decodes_concatenated_stream_across_chunk_split() {
        // Two frames encoded back-to-back, then fed in two arbitrary chunks: the decoder
        // must hold partial state across the split (the real serial-read case).
        let a = Frame {
            t_ms: 1,
            msg: Msg::Tick(1),
        };
        let b = Frame {
            t_ms: 2,
            msg: Msg::Fused(Fused {
                roll_deg: -10.0,
                pitch_deg: 70.0,
                yaw_deg: 359.9,
                altitude_m: 0.0,
                vertical_speed_mps: -1.5,
                baro_residual_m: 0.0,
            }),
        };
        let mut stream = Vec::new();
        let mut ba = [0u8; MAX_FRAME];
        let mut bb = [0u8; MAX_FRAME];
        stream.extend_from_slice(encode(&a, &mut ba).unwrap());
        stream.extend_from_slice(encode(&b, &mut bb).unwrap());

        let split = stream.len() / 2;
        let mut got = Vec::new();
        let mut dec = Decoder::new();
        dec.push(&stream[..split], |f| got.push(f));
        dec.push(&stream[split..], |f| got.push(f));
        assert_eq!(got, std::vec![a, b]);
    }

    #[test]
    fn resyncs_after_garbage() {
        // Leading garbage (a half-frame from connecting mid-stream) is discarded; the next
        // clean frame still decodes.
        let good = Frame {
            t_ms: 9,
            msg: Msg::Tick(7),
        };
        let mut gb = [0u8; MAX_FRAME];
        let wire = encode(&good, &mut gb).unwrap();

        let mut stream = std::vec![0x11u8, 0x22, 0x33]; // no delimiter yet — partial junk
        stream.push(0x00); // delimiter closes the junk -> DeserError, resync
        stream.extend_from_slice(wire);

        let mut got = Vec::new();
        let mut dec = Decoder::new();
        let dropped = dec.push(&stream, |f| got.push(f));
        assert_eq!(got, std::vec![good]);
        assert!(dropped >= 1, "the junk frame should be counted as dropped");
    }
}
