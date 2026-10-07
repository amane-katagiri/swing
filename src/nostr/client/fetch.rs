use std::collections::{BTreeSet, HashSet};
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

pub(super) struct Collected {
    pub(super) events: Vec<Event>,
    pub(super) completed: usize,
}

pub(super) async fn collect_newest(
    mut stream: impl Stream<Item = (RelayUrl, Streamed)> + Unpin,
    cap: FetchCap,
) -> Collected {
    let mut newest: BTreeSet<Event> = BTreeSet::new();
    let mut bytes = 0usize;
    let mut finished: HashSet<RelayUrl> = HashSet::new();
    let mut failed: HashSet<RelayUrl> = HashSet::new();
    let now = Timestamp::now().as_secs();
    while let Some((url, item)) = stream.next().await {
        match item {
            Streamed::Event(event) if !plausible_at(event.created_at.as_secs(), now) => {
                tracing::debug!(relay = %url, event_id = %event.id, "skipping an event dated too far ahead");
            }
            Streamed::Event(event) => {
                let size = event_bytes(&event);
                if newest.insert(event) {
                    bytes += size;
                }
                while newest.len() > cap.events || bytes > cap.bytes {
                    let Some(oldest) = newest.pop_last() else {
                        break;
                    };
                    bytes -= event_bytes(&oldest);
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
        events: newest.into_iter().collect(),
        completed: finished.difference(&failed).count(),
    }
}

pub(super) async fn gather(
    batches: impl Stream<Item = Result<Vec<Event>>>,
    cap: FetchCap,
    deadline: Duration,
    context: &'static str,
) -> Result<Vec<Event>> {
    let mut out = Vec::new();
    let mut bytes = 0usize;
    let collect = async {
        let mut batches = std::pin::pin!(batches);
        while let Some(batch) = batches.next().await {
            for event in batch? {
                let size = event_bytes(&event);
                if out.len() >= cap.events || bytes.saturating_add(size) > cap.bytes {
                    tracing::warn!(
                        context,
                        "relay answers exceed the fetch budget; keeping what fits"
                    );
                    return Ok(());
                }
                bytes += size;
                out.push(event);
            }
        }
        Ok::<(), anyhow::Error>(())
    };
    match tokio::time::timeout(deadline, collect).await {
        Ok(result) => result?,
        Err(_) => tracing::warn!(
            context,
            "relays did not finish within the fetch deadline; keeping what arrived"
        ),
    }
    Ok(out)
}
