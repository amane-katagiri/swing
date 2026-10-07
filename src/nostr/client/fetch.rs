use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Duration;

use anyhow::Result;
use futures_util::{Stream, StreamExt};
use nostr_sdk::prelude::*;

use super::super::{budget, plausible_at};

pub(super) const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const FETCH_DEADLINE: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy)]
pub(super) struct FetchCap {
    pub(super) events: usize,
    pub(super) bytes: usize,
}

impl FetchCap {
    pub(super) const TOTAL: Self = Self {
        events: budget::MAX_FETCH_TOTAL_EVENTS,
        bytes: budget::MAX_FETCH_TOTAL_BYTES,
    };

    pub(super) const PER_REQ: Self = Self {
        events: budget::MAX_RELAY_FETCH_LIMIT,
        bytes: budget::MAX_FETCH_TOTAL_BYTES / budget::FETCH_CONCURRENCY,
    };
}

pub(super) fn event_bytes(event: &Event) -> usize {
    event.as_json().len()
}

pub(super) fn capped_limit(count: usize, per: usize) -> usize {
    count.saturating_mul(per).min(budget::MAX_RELAY_FETCH_LIMIT)
}

pub(super) enum Streamed {
    Event(Event),
    Failed(String),
    Finished,
}

pub(super) fn relay_stream(
    relay: Relay,
    filters: Vec<Filter>,
) -> std::pin::Pin<Box<dyn Stream<Item = (RelayUrl, Streamed)> + Send>> {
    let url = relay.url().clone();
    let opened = async move {
        match relay.stream_events(filters).await {
            Ok(events) => {
                // The SDK ends the stream the same way on EOSE and on a dropped connection.
                let end = futures_util::stream::once(async move {
                    if relay.status().is_connected() {
                        Streamed::Finished
                    } else {
                        Streamed::Failed("disconnected before the end of stored events".into())
                    }
                });
                events
                    .map(|item| match item {
                        Ok(event) => Streamed::Event(event),
                        Err(e) => Streamed::Failed(e.to_string()),
                    })
                    .chain(end)
                    .left_stream()
            }
            Err(e) => futures_util::stream::iter([Streamed::Failed(e.to_string())]).right_stream(),
        }
    };
    Box::pin(
        futures_util::stream::once(opened)
            .flatten()
            .map(move |item| (url.clone(), item)),
    )
}

pub(super) struct Measured {
    pub(super) event: Event,
    pub(super) bytes: usize,
}

pub(super) struct Collected {
    pub(super) events: Vec<Measured>,
    pub(super) completed: usize,
}

pub(super) async fn collect_newest(
    mut stream: impl Stream<Item = (RelayUrl, Streamed)> + Unpin,
    cap: FetchCap,
) -> Collected {
    let mut newest: BTreeSet<Event> = BTreeSet::new();
    let mut sizes: HashMap<EventId, usize> = HashMap::new();
    let mut bytes = 0usize;
    let mut finished: HashSet<RelayUrl> = HashSet::new();
    let mut failed: HashSet<RelayUrl> = HashSet::new();
    let now = Timestamp::now().as_secs();
    while let Some((url, item)) = stream.next().await {
        match item {
            Streamed::Event(event) if !plausible_at(event.created_at.as_secs(), now) => {
                tracing::debug!(relay = %url, event_id = %event.id, "skipping an event dated too far ahead");
            }
            Streamed::Event(event) if sizes.contains_key(&event.id) => {}
            Streamed::Event(event) => {
                let size = event_bytes(&event);
                sizes.insert(event.id, size);
                newest.insert(event);
                bytes += size;
                while newest.len() > cap.events || bytes > cap.bytes {
                    let Some(oldest) = newest.pop_last() else {
                        break;
                    };
                    bytes -= sizes.remove(&oldest.id).unwrap_or(0);
                }
            }
            Streamed::Failed(e) => {
                tracing::debug!(relay = %url, error = %e, "relay did not answer the request");
                failed.insert(url);
            }
            Streamed::Finished => {
                finished.insert(url);
            }
        }
    }
    Collected {
        events: newest
            .into_iter()
            .map(|event| Measured {
                bytes: sizes.get(&event.id).copied().unwrap_or(0),
                event,
            })
            .collect(),
        completed: finished.difference(&failed).count(),
    }
}

pub(super) async fn gather(
    batches: impl Stream<Item = Result<Vec<Measured>>>,
    cap: FetchCap,
    deadline: Duration,
    context: &'static str,
) -> Result<Vec<Event>> {
    let mut out = Vec::new();
    let mut bytes = 0usize;
    let mut answered = 0usize;
    let mut failures: Vec<anyhow::Error> = Vec::new();
    let collect = async {
        let mut batches = std::pin::pin!(batches);
        while let Some(batch) = batches.next().await {
            let batch = match batch {
                Ok(batch) => batch,
                Err(e) => {
                    failures.push(e);
                    continue;
                }
            };
            answered += 1;
            for Measured { event, bytes: size } in batch {
                if out.len() >= cap.events || bytes.saturating_add(size) > cap.bytes {
                    tracing::warn!(
                        context,
                        "relay answers exceed the fetch budget; keeping what fits"
                    );
                    return;
                }
                bytes += size;
                out.push(event);
            }
        }
    };
    if tokio::time::timeout(deadline, collect).await.is_err() {
        tracing::warn!(
            context,
            "relays did not finish within the fetch deadline; keeping what arrived"
        );
    }
    if answered == 0
        && let Some(e) = failures.pop()
    {
        return Err(e);
    }
    if !failures.is_empty() {
        tracing::warn!(
            context,
            failed = failures.len(),
            answered,
            error = format!("{:#}", failures[0]),
            "some requests got no answer from any relay; keeping the others"
        );
    }
    Ok(out)
}

pub(super) async fn walk_pages(
    relay: Relay,
    filter: Filter,
    page: usize,
    cap: FetchCap,
    deadline: tokio::time::Instant,
) -> Option<Vec<Event>> {
    let mut seen: HashSet<EventId> = HashSet::new();
    let mut out = Vec::new();
    let mut bytes = 0usize;
    let mut until: Option<Timestamp> = None;
    let mut answered = false;
    while tokio::time::Instant::now() < deadline {
        let mut request = filter.clone().limit(page);
        if let Some(until) = until {
            request = request.until(until);
        }
        let stream = relay_stream(relay.clone(), vec![request])
            .take_until(tokio::time::sleep(FETCH_TIMEOUT))
            .take_until(tokio::time::sleep_until(deadline));
        let collected = collect_newest(std::pin::pin!(stream), FetchCap::PER_REQ).await;
        if collected.completed == 0 {
            break;
        }
        answered = true;
        let oldest = collected.events.iter().map(|m| m.event.created_at).min();
        let before = out.len();
        for Measured { event, bytes: size } in collected.events {
            if !seen.insert(event.id) {
                continue;
            }
            if out.len() >= cap.events || bytes.saturating_add(size) > cap.bytes {
                tracing::warn!(relay = %relay.url(), "relay answers exceed the fetch budget; keeping what fits");
                return Some(out);
            }
            bytes += size;
            out.push(event);
        }
        match oldest {
            Some(oldest) if out.len() > before => until = Some(oldest),
            _ => break,
        }
    }
    answered.then_some(out)
}
