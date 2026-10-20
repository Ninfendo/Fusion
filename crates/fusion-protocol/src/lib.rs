//! The wire protocol between the view server and its clients.
//!
//! The messages are defined in `proto/fusion/v1/viewserver.proto` and
//! compiled to Rust when the crate builds. [`encode`] and [`decode`] turn a
//! message into the bytes of one WebSocket frame and back. The `From`
//! conversions turn cells into Fusion's own types.

mod convert;

/// The generated messages, `fusion.v1`.
#[allow(missing_docs, clippy::all, clippy::pedantic)]
pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/fusion.v1.rs"));
}

pub use convert::{Cell, DecimalOutOfRange};
pub use prost::DecodeError;

/// A message as the bytes of one WebSocket frame.
pub fn encode<M: prost::Message>(message: &M) -> Vec<u8> {
    message.encode_to_vec()
}

/// A message from the bytes of one WebSocket frame.
pub fn decode<M: prost::Message + Default>(bytes: &[u8]) -> Result<M, DecodeError> {
    M::decode(bytes)
}
