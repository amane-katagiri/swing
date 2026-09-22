use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use nostr_sdk::prelude::*;
use tokio::task::JoinSet;
use tracing::warn;

use crate::ipfs::KuboStore;
use crate::nip05::Nip05Verify;
use crate::nostr::{self, RelayClient, ReportRelay, SiteEvent};
use crate::state::State;

use super::{Agent, now_secs};

pub(super) async fn refresh_follow_set<C, N, R>(
    relay: &RelayClient,
    agent: &Arc<Agent<C, N, R>>,
    tasks: &mut JoinSet<()>,
) where
    C: KuboStore + Send + Sync + 'static,
    N: Nip05Verify + Send + Sync + 'static,
    R: ReportRelay + Send + Sync + 'static,
{
    let config = &agent.config;
    let (fetched, fetch_succeeded) = match relay.fetch_follow_set(&config.nostr.mirror_set).await {
        Ok(event) => (event, true),
        Err(e) => {
            warn!(error = %e, "fetching follow set failed");
            (None, false)
        }
    };
    let own = relay.keys.public_key();
    let choice = {
        let mut state = agent.state.lock().await;
        let stored = state
            .follow_set
            .clone()
            .filter(|ev| nostr::is_follow_set_of(ev, &own, &config.nostr.mirror_set));
        let choice = nostr::choose_follow_set(fetched, fetch_succeeded, stored, now_secs());
        if let Some(choice) = &choice
            && choice.save
        {
            state.follow_set = Some(choice.event.clone());
            agent.save(&state, "follow set update").await;
        }
        choice
    };
    let Some(choice) = choice else {
        warn!(mirror_set = %config.nostr.mirror_set, "no follow set found yet; will retry");
        return;
    };
    if choice.republish {
        warn!(event_id = %choice.event.id, "relays returned an older follow set or none; republishing the saved one");
        match relay.publish_to_relays(&choice.event).await {
            Ok(output) if output.success.is_empty() => {
                warn!("no relay accepted the republished follow set")
            }
            Ok(_) => {}
            Err(e) => warn!(error = %e, "republishing the follow set failed"),
        }
    }
    let follow_event = choice.event;

    let (targets, truncated) = nostr::follow_set_pubkeys_capped(&follow_event);
    if truncated {
        warn!(
            event_id = %follow_event.id,
            cap = nostr::budget::MAX_FOLLOW_SET_ENTRIES,
            "follow set has more p tags than the cap; the rest are ignored"
        );
    }
    let new_targets: HashSet<PublicKey> = targets.into_iter().collect();
    agent.replace_targets(new_targets.clone());
    if config.policy.remove_on_unfollow {
        agent.remove_unfollowed().await;
    }

    let target_list: Vec<PublicKey> = new_targets.into_iter().collect();
    if let Err(e) = relay
        .subscribe_site_events(config.nostr.site_event_kind, &target_list)
        .await
    {
        warn!(error = %e, "subscribing to site events failed");
    }

    match relay
        .fetch_site_events(config.nostr.site_event_kind, &target_list)
        .await
    {
        Ok(events) => {
            let parsed: Vec<SiteEvent> = events
                .iter()
                .filter_map(
                    |e| match nostr::parse_site_event(e, config.nostr.site_event_kind) {
                        Ok(se) => Some(se),
                        Err(err) => {
                            warn!(error = %err, "skipping invalid historical site event");
                            None
                        }
                    },
                )
                .collect();
            let latest = nostr::select_latest(&parsed, now_secs())
                .into_values()
                .collect();
            let selected = {
                let state = agent.state.lock().await;
                limit_sites_per_account(latest, &state, config.policy.max_sites_per_account)
            };
            for ev in selected {
                agent.submit(ev, tasks);
            }
        }
        Err(e) => warn!(error = %e, "fetching historical site events failed"),
    }
}

pub(super) fn limit_sites_per_account(
    events: Vec<SiteEvent>,
    state: &State,
    max_sites: usize,
) -> Vec<SiteEvent> {
    let mut by_account: HashMap<PublicKey, Vec<(bool, SiteEvent)>> = HashMap::new();
    for ev in events {
        let stored = state
            .sites
            .contains_key(&crate::state::site_key(&ev.pubkey.to_hex(), &ev.d));
        by_account.entry(ev.pubkey).or_default().push((stored, ev));
    }
    let mut selected = Vec::new();
    for mut events in by_account.into_values() {
        events.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.created_at.cmp(&a.1.created_at)));
        let stored = events.iter().filter(|(s, _)| *s).count();
        selected.extend(
            events
                .into_iter()
                .take(stored.max(max_sites))
                .map(|(_, ev)| ev),
        );
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;
    use crate::state::VersionRecord;

    fn stored_state(fx: &Fixture, ds: &[&str]) -> State {
        let mut state = State::default();
        for d in ds {
            state.apply_store(
                &fx.key(d),
                VersionRecord {
                    cid: "c".into(),
                    size: 1,
                    created_at: 1,
                    stored_at: 1,
                },
            );
        }
        state
    }

    #[test]
    fn limit_sites_per_account_prefers_stored_then_newest() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        let state = stored_state(&fx, &["stored.example"]);
        let mut other_event = fx.event("x.example", "c", None, 1);
        other_event.pubkey = Keys::generate().public_key();
        let events = vec![
            fx.event("old.example", "c", None, 10),
            fx.event("stored.example", "c", None, 5),
            fx.event("new.example", "c", None, 30),
            fx.event("mid.example", "c", None, 20),
            other_event,
        ];

        let mut selected: Vec<String> = limit_sites_per_account(events, &state, 3)
            .into_iter()
            .map(|e| e.d)
            .collect();
        selected.sort();
        assert_eq!(
            selected,
            vec!["mid.example", "new.example", "stored.example", "x.example"]
        );
    }

    #[test]
    fn limit_sites_per_account_keeps_all_stored_sites_over_the_limit() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        let state = stored_state(&fx, &["a.example", "b.example"]);
        let events = vec![
            fx.event("a.example", "c", None, 1),
            fx.event("b.example", "c", None, 1),
            fx.event("new.example", "c", None, 99),
        ];

        let mut selected: Vec<String> = limit_sites_per_account(events, &state, 1)
            .into_iter()
            .map(|e| e.d)
            .collect();
        selected.sort();
        assert_eq!(selected, vec!["a.example", "b.example"]);
    }
}
