use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use nostr_sdk::prelude::*;
use tokio::task::JoinSet;
use tracing::warn;

use crate::ipfs::{KuboNode, KuboStore};
use crate::nip05::Nip05Verify;
use crate::nostr::{self, AnchorRelay, RelayClient, ReportRelay, SignedSite};
use crate::state::State;

use super::{Agent, now_secs};

pub(super) async fn refresh_follow_set<C, N, R>(
    relay: &RelayClient,
    agent: &Arc<Agent<C, N, R>>,
    tasks: &mut JoinSet<()>,
) -> Option<Vec<PublicKey>>
where
    C: KuboStore + KuboNode + Send + Sync + 'static,
    N: Nip05Verify + Send + Sync + 'static,
    R: ReportRelay + AnchorRelay + Send + Sync + 'static,
{
    let follow_event = choose_and_apply_follow_set(relay, agent).await?;
    Some(resubscribe_and_backfill(relay, agent, tasks, follow_event).await)
}

async fn choose_and_apply_follow_set<C, N, R>(
    relay: &RelayClient,
    agent: &Arc<Agent<C, N, R>>,
) -> Option<Event>
where
    C: KuboStore + KuboNode + Send + Sync + 'static,
    N: Nip05Verify + Send + Sync + 'static,
    R: ReportRelay + AnchorRelay + Send + Sync + 'static,
{
    let config = &agent.config;
    let (fetched, fetch_succeeded) = match relay.fetch_follow_set(&config.nostr.mirror_set).await {
        Ok(event) => (event, true),
        Err(e) => {
            warn!(error = %e, "fetching follow set failed");
            (None, false)
        }
    };
    let own = relay.public_key();
    let choice = {
        let mut state = agent.state.lock().await;
        let stored = state
            .follow_set
            .clone()
            .filter(|ev| nostr::is_saved_follow_set_of(ev, &own, &config.nostr.mirror_set));
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
        let stored_accounts = agent.state.lock().await.accounts().len();
        if stored_accounts > 0 {
            warn!(
                mirror_set = %config.nostr.mirror_set,
                stored_accounts,
                "no follow set found yet; keeping stored sites until one is found (did the key or mirror_set change?)"
            );
        } else {
            warn!(mirror_set = %config.nostr.mirror_set, "no follow set found yet; will retry");
        }
        return None;
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
    Some(choice.event)
}

async fn resubscribe_and_backfill<C, N, R>(
    relay: &RelayClient,
    agent: &Arc<Agent<C, N, R>>,
    tasks: &mut JoinSet<()>,
    follow_event: Event,
) -> Vec<PublicKey>
where
    C: KuboStore + KuboNode + Send + Sync + 'static,
    N: Nip05Verify + Send + Sync + 'static,
    R: ReportRelay + AnchorRelay + Send + Sync + 'static,
{
    let config = &agent.config;
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
        .fetch_latest_signed_sites(config.nostr.site_event_kind, &target_list, Vec::new())
        .await
    {
        Ok(latest) => {
            let selected = {
                let mut state = agent.state.lock().await;
                if fill_saved_events(&mut state, &latest) {
                    agent.save(&state, "keeping site events").await;
                }
                limit_sites_per_account(latest, &state, config.policy.max_sites_per_account)
            };
            agent.set_followed_sites(
                selected
                    .iter()
                    .map(|s| (s.site.pubkey, s.site.d.clone()))
                    .collect(),
            );
            for ev in selected {
                agent.submit(ev, tasks);
            }
        }
        Err(e) => warn!(error = %e, "fetching historical site events failed"),
    }
    target_list
}

fn fill_saved_events(state: &mut State, latest: &[SignedSite]) -> bool {
    let mut filled = false;
    for signed in latest {
        let key = crate::state::site_key(&signed.site.pubkey.to_hex(), &signed.site.d);
        filled |= state.fill_latest_event(&key, &signed.site.cid, &signed.event);
    }
    filled
}

pub(super) fn limit_sites_per_account(
    events: Vec<SignedSite>,
    state: &State,
    max_sites: usize,
) -> Vec<SignedSite> {
    let mut by_account: HashMap<PublicKey, Vec<(bool, SignedSite)>> = HashMap::new();
    for ev in events {
        let stored = state.sites.contains_key(&crate::state::site_key(
            &ev.site.pubkey.to_hex(),
            &ev.site.d,
        ));
        by_account
            .entry(ev.site.pubkey)
            .or_default()
            .push((stored, ev));
    }
    let mut selected = Vec::new();
    for mut events in by_account.into_values() {
        events.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then(b.1.site.created_at.cmp(&a.1.site.created_at))
        });
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
                    event: None,
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
        other_event.site.pubkey = Keys::generate().public_key();
        let events = vec![
            fx.event("old.example", "c", None, 10),
            fx.event("stored.example", "c", None, 5),
            fx.event("new.example", "c", None, 30),
            fx.event("mid.example", "c", None, 20),
            other_event,
        ];

        let mut selected: Vec<String> = limit_sites_per_account(events, &state, 3)
            .into_iter()
            .map(|e| e.site.d)
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
            .map(|e| e.site.d)
            .collect();
        selected.sort();
        assert_eq!(selected, vec!["a.example", "b.example"]);
    }

    #[test]
    fn site_events_fill_old_records_that_lack_them() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        let mut state = State::default();
        let fetched = fx.event("a.example", "bafy-a", None, 100);
        state.apply_store(
            &fx.key("a.example"),
            VersionRecord {
                cid: "bafy-a".into(),
                size: 1,
                created_at: 100,
                stored_at: 1,
                event: None,
            },
        );
        let other = fx.event("b.example", "bafy-b", None, 100);

        assert!(fill_saved_events(&mut state, &[fetched.clone(), other]));
        assert_eq!(
            state.latest_version(&fx.key("a.example")).unwrap().event,
            Some(fetched.event.clone())
        );
        assert!(!fill_saved_events(&mut state, &[fetched]));
    }

    #[test]
    fn a_later_signing_of_the_stored_cid_replaces_the_kept_event() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        let mut state = State::default();
        let stored = fx.event("a.example", "bafy-a", None, 100);
        let resigned = fx.event("a.example", "bafy-a", None, 200);
        state.apply_store(
            &fx.key("a.example"),
            VersionRecord {
                cid: "bafy-a".into(),
                size: 1,
                created_at: 100,
                stored_at: 1,
                event: Some(stored.event),
            },
        );

        assert!(fill_saved_events(
            &mut state,
            std::slice::from_ref(&resigned)
        ));
        let latest = state.latest_version(&fx.key("a.example")).unwrap();
        assert_eq!(latest.event, Some(resigned.event.clone()));
        assert_eq!(latest.created_at, 100);
        assert!(!fill_saved_events(&mut state, &[resigned]));
    }
}
