//! Trace-capture helpers for asserting on OpenTelemetry spans in tests.
//!
//! # Usage
//!
//! ```rust,no_run
//! # #[cfg(feature = "testing")]
//! # mod example {
//! use socle::testing::trace::{CaptureExporter, init_capture_tracing};
//! use socle::assert_span;
//!
//! #[tokio::test]
//! async fn span_is_recorded() {
//!     let exporter = init_capture_tracing();
//!
//!     tracing::info_span!("my_op").in_scope(|| {
//!         tracing::info!("doing work");
//!     });
//!
//!     let spans = exporter.spans();
//!     assert_span!(spans, name = "my_op");
//! }
//! # }
//! ```

use std::sync::{Arc, Mutex};

/// A single recorded span.
#[derive(Debug, Clone)]
pub struct SpanRecord {
    /// Span name.
    pub name: String,
    /// Key-value attributes recorded on the span.
    pub attributes: Vec<(String, String)>,
}

/// In-memory span collector.  Clone freely — all clones share the same buffer.
#[derive(Debug, Clone, Default)]
pub struct CaptureExporter {
    spans: Arc<Mutex<Vec<SpanRecord>>>,
}

impl CaptureExporter {
    /// Return all captured spans.
    pub fn spans(&self) -> Vec<SpanRecord> {
        self.spans.lock().unwrap().clone()
    }

    /// Drain and return all captured spans, leaving the buffer empty.
    pub fn drain(&self) -> Vec<SpanRecord> {
        std::mem::take(&mut self.spans.lock().unwrap())
    }

    pub(crate) fn push(&self, record: SpanRecord) {
        self.spans.lock().unwrap().push(record);
    }
}

use tracing::Subscriber;
use tracing::span::{Attributes, Id};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

struct CaptureLayer {
    exporter: CaptureExporter,
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("span must exist");
        let mut record = SpanRecord {
            name: span.name().to_owned(),
            attributes: vec![],
        };

        struct Visitor<'a>(&'a mut Vec<(String, String)>);
        impl tracing::field::Visit for Visitor<'_> {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0.push((field.name().to_owned(), format!("{value:?}")));
            }
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                self.0.push((field.name().to_owned(), value.to_owned()));
            }
        }
        attrs.record(&mut Visitor(&mut record.attributes));
        span.extensions_mut().insert(record);
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(&id)
            && let Some(record) = span.extensions().get::<SpanRecord>()
        {
            self.exporter.push(record.clone());
        }
    }
}

/// Install a `tracing` subscriber that captures spans into a shared
/// [`CaptureExporter`] and returns it.
///
/// The first call installs the subscriber; subsequent calls in the same
/// process return the same exporter already wired into the subscriber.
pub fn init_capture_tracing() -> CaptureExporter {
    use std::sync::OnceLock;
    use tracing_subscriber::prelude::*;

    static EXPORTER: OnceLock<CaptureExporter> = OnceLock::new();

    EXPORTER
        .get_or_init(|| {
            let exp = CaptureExporter::default();
            let layer = CaptureLayer {
                exporter: exp.clone(),
            };
            let _ = tracing_subscriber::registry().with(layer).try_init();
            exp
        })
        .clone()
}

/// Assert that at least one span in `$spans` has the given `name`.
///
/// ```rust,ignore
/// assert_span!(spans, name = "my_op");
/// ```
#[macro_export]
macro_rules! assert_span {
    ($spans:expr, name = $name:expr) => {{
        let found = $spans
            .iter()
            .any(|s: &$crate::testing::trace::SpanRecord| s.name == $name);
        assert!(
            found,
            "expected span {:?} not found in {:?}",
            $name,
            $spans.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }};
    ($spans:expr, name = $name:expr, attr = ($key:expr, $val:expr)) => {{
        let matching: Vec<_> = $spans
            .iter()
            .filter(|s: &&$crate::testing::trace::SpanRecord| s.name == $name)
            .collect();
        let found = matching
            .iter()
            .any(|s| s.attributes.iter().any(|(k, v)| k == $key && v == $val));
        assert!(
            found,
            "span {:?} with attribute {:?}={:?} not found. Spans: {:?}",
            $name, $key, $val, matching
        );
    }};
}
