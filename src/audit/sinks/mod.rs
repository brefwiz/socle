mod tracing_sink;

pub use tracing_sink::TracingAuditSink;

#[cfg(feature = "test-util")]
mod channel;

#[cfg(feature = "test-util")]
pub use channel::ChannelAuditSink;

#[cfg(feature = "nats")]
mod nats;

#[cfg(feature = "nats")]
pub use nats::{NatsJetStreamAuditSink, audit_dropped_total};
