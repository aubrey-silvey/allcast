//! Shared gRPC types + service stubs for the receiver→sender telemetry channel.
//!
//! The media path is RTP/UDP; this crate is purely the control-plane schema,
//! generated from `proto/telemetry.proto` by tonic-build at compile time.

pub mod v1 {
    tonic::include_proto!("allcast.telemetry.v1");
}

pub use v1::{
    telemetry_client, telemetry_server, SubscribeRequest, TelemetrySample,
};

/// Default TCP port the receiver serves telemetry on (RTP media is UDP 5004).
pub const DEFAULT_PORT: u16 = 5005;
