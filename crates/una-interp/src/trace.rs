//! The JSON form of a trace (`spec/11 §5`), shared by `una run --json`, `una test --json` and
//! the `--trace` file of a native executable (`spec/16 §3`), so that the interpreter and the
//! native backend write traces byte for byte alike.

use serde::Serialize;
use serde::ser::{SerializeMap, Serializer};

use crate::machine::{Event, EventOutcome};

/// Name and value pairs as a JSON object, in their order.
pub struct Pairs<'a>(pub &'a [(String, String)]);

impl Serialize for Pairs<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (name, value) in self.0 {
            map.serialize_entry(name, value)?;
        }
        map.end()
    }
}

/// An event of a trace, as `spec/11 §5` shows it.
#[derive(Serialize)]
pub struct EventDoc<'a> {
    seq: usize,
    /// The task's path, only inside a `together` task (`spec/19 §7` rule 1).
    #[serde(skip_serializing_if = "str::is_empty")]
    task: &'a str,
    instance: &'a str,
    capability: &'a str,
    op: &'a str,
    args: Pairs<'a>,
    outcome: OutcomeDoc<'a>,
}

impl<'a> From<&'a Event> for EventDoc<'a> {
    fn from(event: &'a Event) -> Self {
        EventDoc {
            seq: event.seq,
            task: &event.task,
            instance: &event.instance,
            capability: &event.capability,
            op: &event.op,
            args: Pairs(&event.args),
            outcome: match &event.outcome {
                EventOutcome::Ok(value) => OutcomeDoc::Ok { value },
                EventOutcome::Fail { variant, value } => OutcomeDoc::Fail { variant, value },
                EventOutcome::Aborted => OutcomeDoc::Aborted,
            },
        }
    }
}

/// How an operation, or the `do` line of an example, ended.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum OutcomeDoc<'a> {
    /// It returned this value.
    Ok {
        /// The value, in its Una form or redacted.
        value: &'a str,
    },
    /// It failed.
    Fail {
        /// The variant.
        variant: &'a str,
        /// The failure, in its Una form.
        value: &'a str,
    },
    /// The run aborted before it ended.
    Aborted,
}

/// A trace as a JSON array of events, compact, as `una run --json` writes its `trace`.
pub fn trace_json(events: &[Event]) -> String {
    let docs: Vec<EventDoc<'_>> = events.iter().map(EventDoc::from).collect();
    serde_json::to_string(&docs).unwrap_or_else(|_| "[]".to_owned())
}
