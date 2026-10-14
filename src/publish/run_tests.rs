use std::path::Path;

use nostr_sdk::prelude::*;

use super::*;
use crate::test_support::{
    KuboApi, events_on_relay, read_only_relay, relay_holding, unanswered_signer,
};

const ADDED: &str = "bafybeicwrird3ditgfyu4snq6i3x6i564gfcywffrwcgumnjhyiva3p2ra";
const SITE: &str = "example.com";

fn config_for(state: &Path, keys: &Keys, kubo: &KuboApi, relay: &str, keep: usize) -> Config {
    let toml = format!(
        "[nostr]\nsecret_key = \"{}\"\n[ipfs]\napi = \"{}\"\n[kubo]\nmanaged = false\n",
        keys.secret_key().to_secret_hex(),
        kubo.url
    );
    let mut config = config::build_config_from_str(&toml, |_| None).unwrap();
    config.agent.state_dir = state.join("data");
    config.kubo.repo = state.join("kubo");
    config.config_path = state.join("swing.toml");
    config.nostr.relays = vec![relay.to_string()];
    config.publish.keep_versions = keep;
    config
}

fn request() -> Request {
    let off = || Some("off".to_string());
    Request {
        d: SITE.into(),
        url: None,
        title: None,
        message: None,
        note: false,
        modes: ModeOverrides {
            nip05: off(),
            check_dotfiles: off(),
            check_size: off(),
            check_links: off(),
            check_unchanged: Some("require".into()),
        },
        yes: true,
    }
}

fn site_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "hi").unwrap();
    dir
}

struct Published {
    kubo: KuboApi,
    keys: Keys,
    site_path: String,
    old: Vec<String>,
}

async fn with_old_versions(keep_old: &[u64]) -> Published {
    let kubo = KuboApi::serve(ADDED).await;
    let keys = Keys::generate();
    let site_path = MfsLayout::new("/swing").publish_site(&keys.public_key().to_hex(), SITE);
    let old: Vec<String> = keep_old
        .iter()
        .map(|t| format!("{site_path}/{t}"))
        .collect();
    let mut s = kubo.s.lock().unwrap();
    for path in &old {
        s.mfs
            .insert(path.clone(), crate::test_support::CID_B.into());
    }
    drop(s);
    Published {
        kubo,
        keys,
        site_path,
        old,
    }
}

async fn site_events(url: &str, keys: &Keys) -> Vec<Event> {
    events_on_relay(
        url,
        Filter::new()
            .author(keys.public_key())
            .kind(Kind::Custom(35980)),
    )
    .await
}

#[tokio::test]
async fn a_publish_announces_the_site_keeps_the_version_and_prunes_the_oldest() {
    let now = Timestamp::now().as_secs();
    let p = with_old_versions(&[now - 300, now - 200, now - 100]).await;
    let (_relay, url) = relay_holding(&[]).await;
    let state = tempfile::tempdir().unwrap();
    let config = config_for(state.path(), &p.keys, &p.kubo, &url, 2);
    let site = site_dir();

    run(config, site.path(), request()).await.unwrap();

    let events = site_events(&url, &p.keys).await;
    let [event] = events.as_slice() else {
        panic!("{events:?}");
    };
    let parsed = nostr::parse_site_event(event, 35980).unwrap();
    assert_eq!(parsed.cid, ADDED);
    assert_eq!(parsed.d, SITE);
    assert_eq!(parsed.size, Some(42));
    let current = format!("{}/{}", p.site_path, parsed.created_at);
    assert_eq!(p.kubo.paths(), vec![p.old[2].clone(), current]);
    assert_eq!(
        p.kubo.s.lock().unwrap().removed,
        vec![p.old[1].clone(), p.old[0].clone()]
    );
}

#[tokio::test]
async fn when_no_relay_accepts_the_event_the_old_versions_are_kept() {
    let now = Timestamp::now().as_secs();
    let p = with_old_versions(&[now - 300, now - 200, now - 100]).await;
    let (_relay, url) = read_only_relay().await;
    let state = tempfile::tempdir().unwrap();
    let config = config_for(state.path(), &p.keys, &p.kubo, &url, 1);
    let site = site_dir();

    let err = run(config, site.path(), request()).await.unwrap_err();

    assert_eq!(err.to_string(), NO_RELAY_ACCEPTED);
    let paths = p.kubo.paths();
    assert_eq!(paths.len(), 4, "{paths:?}");
    assert!(p.old.iter().all(|old| paths.contains(old)), "{paths:?}");
    assert!(p.kubo.s.lock().unwrap().removed.is_empty());
}

#[tokio::test]
async fn an_unsigned_site_event_withdraws_the_staged_version() {
    let p = with_old_versions(&[]).await;
    let staged = format!("{}/{}", p.site_path, Timestamp::now().as_secs());
    p.kubo
        .s
        .lock()
        .unwrap()
        .mfs
        .insert(staged.clone(), ADDED.into());
    let (_relay, url) = relay_holding(&[]).await;
    let relay = RelayClient::connect(
        unanswered_signer(&url, p.keys.public_key()),
        std::slice::from_ref(&url),
    )
    .await
    .unwrap();
    let version = StagedVersion::new(&p.kubo.client(), staged.clone());

    let err = announce(
        relay,
        true,
        version,
        &SiteAnnouncement {
            site_event_kind: 35980,
            d: SITE,
            cid: ADDED,
            url: None,
            size: 42,
            title: None,
            message: None,
            created_at: Timestamp::now(),
        },
        false,
    )
    .await
    .unwrap_err();

    assert!(format!("{err:#}").contains("signing site event"), "{err:#}");
    assert_eq!(p.kubo.s.lock().unwrap().removed, vec![staged]);
    assert!(p.kubo.paths().is_empty());
    assert!(site_events(&url, &p.keys).await.is_empty());
}

#[tokio::test]
async fn pruning_reports_each_failure_and_keeps_going() {
    let kubo = KuboApi::serve(ADDED).await;
    {
        let mut s = kubo.s.lock().unwrap();
        for t in [100, 200, 300, 400] {
            s.mfs.insert(format!("/s/{t}"), ADDED.into());
        }
        s.fail_remove.insert("/s/200".into());
    }

    let outcome = prune_old_versions_collect(&kubo.client(), "/s", 400, 2).await;

    let [
        PruneAttempt::Failed(failed, reason),
        PruneAttempt::Removed(removed),
    ] = outcome.attempts.as_slice()
    else {
        panic!("{outcome:?}");
    };
    assert_eq!((failed.as_str(), removed.as_str()), ("200", "100"));
    assert!(reason.contains("simulated files/rm failure"), "{reason}");
    assert_eq!(outcome.list_error, None);
    assert_eq!(kubo.paths(), vec!["/s/200", "/s/300", "/s/400"]);

    kubo.s.lock().unwrap().fail_list = true;
    let outcome = prune_old_versions_collect(&kubo.client(), "/s", 400, 1).await;
    assert!(outcome.attempts.is_empty());
    assert!(
        outcome
            .list_error
            .as_deref()
            .is_some_and(|e| e.contains("simulated files/ls failure")),
        "{outcome:?}"
    );
    assert_eq!(kubo.paths().len(), 3);
}
