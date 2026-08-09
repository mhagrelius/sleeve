//! The only file in the tree that performs a request.
//!
//! Everything under `model/source/` builds a [`Request`] and parses a body;
//! this drives them. Keeping that boundary in one file is what lets the whole
//! source layer be tested from recorded fixtures.
//!
//! No threads. libsoup's async calls complete on the GLib main loop, so eight
//! sources answering at eight different speeds need no worker, no channel and no
//! lock — each one's callback fires when its answer lands and updates the view
//! then. A slow source cannot hold up a fast one, and a source that never
//! answers just leaves its row in the state it was already in.
//!
//! Rate limiting is a per-source token bucket driven by [`policy`], because the
//! budgets differ by two orders of magnitude and MusicBrainz blocks rather than
//! throttles. A request that arrives too early is not dropped, it is scheduled —
//! a `glib::timeout` on the main loop, never a sleeping thread.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gio;
use gtk::glib;
use soup::prelude::*;

use crate::model::cache::{Cache, Kind};
use crate::model::source::{policy, Outcome, Reason, Request, SourceId};

/// When a source may next be asked, and until when it is in disgrace.
#[derive(Debug, Clone)]
struct Gate {
    next_allowed: Instant,
    session: soup::Session,
}

#[derive(Clone)]
pub struct Http {
    gates: Rc<RefCell<HashMap<SourceId, Gate>>>,
    cache: Rc<Cache>,
}

impl Http {
    pub fn new(cache: Rc<Cache>) -> Self {
        Http {
            gates: Rc::new(RefCell::new(HashMap::new())),
            cache,
        }
    }

    /// Fetch a request, from the cache if it is there and current.
    ///
    /// `deliver` is called exactly once, on the main loop, with the body or with
    /// the reason there is not one. It is never called with an exception and
    /// there is nothing to catch: a source failing is an expected outcome of a
    /// lookup, not an error in it.
    pub fn fetch<F>(&self, request: Request, kind: Kind, deliver: F)
    where
        F: FnOnce(Outcome<Vec<u8>>) + 'static,
    {
        if let Ok(Some(body)) = self.cache.body(request.cache_key()) {
            deliver(Outcome::Found(body));
            return;
        }

        // No dedupe on identical in-flight requests, deliberately. An earlier
        // version answered the second caller with `Unusable("already being
        // fetched")`, which is a lie with consequences: the question really was
        // being answered, and reporting it as a failed source put "Not checked:
        // MusicBrainz" under a ranking whose MusicBrainz data had arrived and
        // been cached by the other caller. Cover art — the only place duplicate
        // requests actually pile up — dedupes in `Covers::load` instead, where a
        // dropped duplicate costs nothing because nobody reports on it.
        let wait = self.reserve(request.source);
        let http = self.clone();

        if wait.is_zero() {
            http.send(request, kind, deliver);
        } else {
            glib::timeout_add_local_once(wait, move || http.send(request, kind, deliver));
        }
    }

    /// Claim the next slot for a source, and say how long until it opens.
    fn reserve(&self, source: SourceId) -> Duration {
        let policy = policy::for_source(source);
        let mut gates = self.gates.borrow_mut();
        let gate = gates.entry(source).or_insert_with(|| Gate {
            next_allowed: Instant::now(),
            session: session_for(source),
        });

        let now = Instant::now();
        let wait = gate.next_allowed.saturating_duration_since(now);
        gate.next_allowed = gate.next_allowed.max(now) + policy.min_interval;
        wait
    }

    /// Push a source's next slot out after it has told us to slow down.
    fn penalise(&self, source: SourceId) {
        let policy = policy::for_source(source);
        if let Some(gate) = self.gates.borrow_mut().get_mut(&source) {
            gate.next_allowed = Instant::now() + policy.backoff;
        }
    }

    fn session(&self, source: SourceId) -> soup::Session {
        self.gates
            .borrow()
            .get(&source)
            .map(|gate| gate.session.clone())
            .unwrap_or_else(|| session_for(source))
    }

    fn send<F>(&self, request: Request, kind: Kind, deliver: F)
    where
        F: FnOnce(Outcome<Vec<u8>>) + 'static,
    {
        let source = request.source;
        let key = request.cache_key().to_string();

        let Ok(message) = soup::Message::new("GET", &request.url) else {
            deliver(Outcome::Unusable(Reason::Network(format!(
                "{} is not a URL",
                request.url
            ))));
            return;
        };

        let headers = message.request_headers();
        if let Some(headers) = headers {
            for (name, value) in &request.headers {
                headers.replace(name, value);
            }
        }

        let http = self.clone();
        let sent = message.clone();
        self.session(source).send_and_read_async(
            &message,
            glib::Priority::DEFAULT,
            gio::Cancellable::NONE,
            move |result| {
                let status = sent.status_code() as u16;

                let outcome = match result {
                    Err(error) => {
                        // libsoup reports a timeout as an ordinary I/O error, so
                        // the distinction is made on the message rather than
                        // lost. A timed-out source and an unreachable one need
                        // different backoffs.
                        let text = error.to_string();
                        if text.contains("Timeout") || text.contains("timed out") {
                            Outcome::Unusable(Reason::Timeout)
                        } else {
                            Outcome::Unusable(Reason::Network(text))
                        }
                    }
                    Ok(_) if status == 429 || status == 503 => {
                        http.penalise(source);
                        Outcome::Unusable(Reason::RateLimited)
                    }
                    Ok(_) if !(200..300).contains(&status) => {
                        // A 404 from the Cover Art Archive is routine and means
                        // "no picture here", not a fault. Everything else in the
                        // 4xx and 5xx ranges is reported.
                        if status == 404 {
                            Outcome::Empty
                        } else {
                            Outcome::Unusable(Reason::Http(status))
                        }
                    }
                    Ok(bytes) if bytes.is_empty() => Outcome::Empty,
                    Ok(bytes) => {
                        let body = bytes.to_vec();
                        let _ = http.cache.store_body(&key, kind, &body);
                        Outcome::Found(body)
                    }
                };
                deliver(outcome);
            },
        );
    }
}

/// A session per source, so each carries its own timeout.
///
/// libsoup's timeout is a property of the session rather than the message, and
/// the Dynamic Range Database needs fifteen seconds where iTunes needs eight.
fn session_for(source: SourceId) -> soup::Session {
    let policy = policy::for_source(source);
    soup::Session::builder()
        .timeout(policy.timeout.as_secs() as u32)
        // Follow the Cover Art Archive's redirect to archive.org.
        .max_conns_per_host(2)
        .build()
}
