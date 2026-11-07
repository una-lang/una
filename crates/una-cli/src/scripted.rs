//! The scripted engine behind `--serve-script` (ADR 0123 rule 21, `spec/31 §8` rule 6): the
//! connections of a checked script in place of listeners, and `--serve-out`. It runs only in
//! the toolchain's own test builds, when `UNA_TEST_RUN_SERVER` is 1 as for the other runs that
//! serve there (`spec/31 §1` rule 10); every other run reports the script as not run yet.

use std::sync::Arc;

use una_core::http::serve::ServeGrant;
use una_http::serve::program::Port;
use una_http::serve::script_file::Event;
use una_interp::stopping::RunStop;

use crate::Environment;

/// The scripted engine of a run.
#[cfg(feature = "test-hooks")]
pub(crate) struct Engine(una_http::serve::script::ScriptPort);

/// No build but the toolchain's tests has the scripted engine.
#[cfg(not(feature = "test-hooks"))]
pub(crate) enum Engine {}

impl Engine {
    /// The engine of the grants `grants` fed by `events`, whose host stop is `stop`'s, when this
    /// build and `environment` run it; `None` otherwise.
    #[cfg(feature = "test-hooks")]
    pub(crate) fn start(
        environment: Environment<'_>,
        grants: &[ServeGrant],
        events: &[Event],
        stop: &Arc<RunStop>,
    ) -> Option<Engine> {
        if crate::variable(environment, "UNA_TEST_RUN_SERVER").as_deref() != Some("1") {
            return None;
        }
        let stop = Arc::clone(stop);
        Some(Engine(una_http::serve::script::ScriptPort::from_script(
            grants,
            events,
            Box::new(move || stop.request_local()),
        )))
    }

    #[cfg(not(feature = "test-hooks"))]
    pub(crate) fn start(
        _: Environment<'_>,
        _: &[ServeGrant],
        _: &[Event],
        _: &Arc<RunStop>,
    ) -> Option<Engine> {
        None
    }

    /// The engine as the program's operations meet it.
    pub(crate) fn port(&self) -> &dyn Port {
        #[cfg(feature = "test-hooks")]
        return &self.0;
        #[cfg(not(feature = "test-hooks"))]
        match *self {}
    }

    /// Ends the engine once the program has ended: the text of `--serve-out`, and what the
    /// engine left not at rest (counters, peers, timers, connections accepted before any server
    /// operation), which the run reports as warnings.
    pub(crate) fn end(&self) -> (String, Vec<String>) {
        #[cfg(feature = "test-hooks")]
        return {
            let ended = self.0.end();
            (
                una_http::serve::script::out_text(&ended.wire),
                ended.problems,
            )
        };
        #[cfg(not(feature = "test-hooks"))]
        match *self {}
    }
}
