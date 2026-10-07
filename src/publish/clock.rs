use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::future::join_all;
use reqwest::header::{ACCEPT, DATE};

use crate::nostr;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockError(String);

impl fmt::Display for ClockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ClockError {}

fn describe_secs(secs: u64) -> String {
    match secs {
        1 => "1 second".to_string(),
        0..120 => format!("{secs} seconds"),
        120..7200 => format!("about {} minutes", secs / 60),
        7200..172_800 => format!("about {} hours", secs / 3600),
        _ => format!("about {} days", secs / 86_400),
    }
}

// Signing right at the skew edge would have peers whose clocks lag ours by a second drop the event.
const SIGN_MARGIN: u64 = 60;

fn sign_limit(now: u64) -> u64 {
    now.saturating_add(nostr::MAX_FUTURE_SKEW - SIGN_MARGIN)
}

fn wait_until_signable(what: &str, at: u64, now: u64) -> ClockError {
    ClockError(format!(
        "{what} is dated {} after this machine's clock; a new version has to be dated after it, \
         and that would be too close to the future limit for relays and mirrors to accept. Wait {} and publish again",
        describe_secs(at - now),
        describe_secs(at - sign_limit(now) + 1)
    ))
}

fn refuse_future_version(site_path: &str, newest: u64, now: u64) -> Result<(), ClockError> {
    if newest < sign_limit(now) {
        return Ok(());
    }
    let path = format!("{site_path}/{newest}");
    if newest <= now.saturating_add(nostr::MAX_FUTURE_SKEW) {
        return Err(wait_until_signable(
            &format!("the newest version {path}"),
            newest,
            now,
        ));
    }
    Err(ClockError(format!(
        "the newest version {path} is dated {} after this machine's clock, and a site event dated after it would be dropped as from the future. \
         Either the clock on this machine is behind now (fix the system clock and publish again), \
         or that version was created while the clock was ahead (remove it with `ipfs files rm -r {path}` and publish again; \
         with Docker Compose, run it in the ipfs container: `docker compose exec ipfs ipfs files rm -r {path}`)",
        describe_secs(newest - now)
    )))
}

fn refuse_previous_ahead(previous: u64, now: u64) -> Result<(), ClockError> {
    if previous < sign_limit(now) {
        return Ok(());
    }
    Err(wait_until_signable(
        "your latest version on the relays",
        previous,
        now,
    ))
}

// Reusing a same-second path would let add_site or a cancelled publish's deferred removal delete the other version.
pub(super) fn version_time(
    site_path: &str,
    newest: Option<u64>,
    previous: Option<u64>,
    now: u64,
) -> Result<u64, ClockError> {
    if let Some(newest) = newest {
        refuse_future_version(site_path, newest, now)?;
    }
    if let Some(previous) = previous {
        refuse_previous_ahead(previous, now)?;
    }
    Ok([newest, previous]
        .into_iter()
        .flatten()
        .map(|t| t + 1)
        .fold(now, u64::max))
}

pub(super) fn refuse_clock_ahead(relay_offsets: &[(String, i64)]) -> Result<(), ClockError> {
    let Some((relay, offset)) = relay_offsets.iter().min_by_key(|(_, offset)| *offset) else {
        return Ok(());
    };
    if *offset <= nostr::MAX_FUTURE_SKEW as i64 {
        return Ok(());
    }
    Err(ClockError(format!(
        "the clock on this machine is {} ahead of every relay that answered (even {relay}, the one closest to it); fix the system clock and publish again",
        describe_secs(offset.unsigned_abs())
    )))
}

fn relay_info_url(relay: &str) -> Option<String> {
    let (scheme, rest) = relay.split_once("://")?;
    let http = match scheme.to_ascii_lowercase().as_str() {
        "ws" => "http",
        "wss" => "https",
        _ => return None,
    };
    Some(format!("{http}://{rest}"))
}

fn parse_http_date(value: &str) -> Option<u64> {
    let time = httpdate::parse_http_date(value).ok()?;
    time.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

async fn probe_relay_offset(client: &reqwest::Client, relay: &str) -> Option<i64> {
    let url = relay_info_url(relay)?;
    let resp = client
        .get(url)
        .header(ACCEPT, "application/nostr+json")
        .send()
        .await
        .ok()?;
    let local = unix_now();
    let date = parse_http_date(resp.headers().get(DATE)?.to_str().ok()?)?;
    Some(local.saturating_sub(i64::try_from(date).ok()?))
}

// The relay connections themselves bypass any HTTP proxy, so the probe does too to time the same servers.
pub(super) async fn probe_relay_offsets(relays: &[String]) -> Vec<(String, i64)> {
    let Ok(client) = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(PROBE_TIMEOUT)
        .build()
    else {
        return Vec::new();
    };
    let offsets = join_all(
        relays
            .iter()
            .map(|relay| probe_relay_offset(&client, relay)),
    )
    .await;
    relays
        .iter()
        .zip(offsets)
        .filter_map(|(relay, offset)| offset.map(|o| (relay.clone(), o)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nostr::MAX_FUTURE_SKEW;

    const SITE: &str = "/swing/publish/k/s";

    #[test]
    fn the_version_time_is_after_every_known_version() {
        assert_eq!(version_time(SITE, None, None, 1000), Ok(1000));
        assert_eq!(version_time(SITE, Some(500), Some(400), 1000), Ok(1000));
        assert_eq!(version_time(SITE, Some(1000), None, 1000), Ok(1001));
        assert_eq!(version_time(SITE, Some(1000), Some(1200), 1000), Ok(1201));
        assert_eq!(version_time(SITE, Some(1300), Some(1200), 1000), Ok(1301));
        assert_eq!(version_time(SITE, None, Some(1000), 1000), Ok(1001));
    }

    #[test]
    fn the_version_time_keeps_a_margin_below_the_skew() {
        let limit = 1000 + MAX_FUTURE_SKEW - SIGN_MARGIN;
        assert_eq!(version_time(SITE, Some(limit - 1), None, 1000), Ok(limit));
        assert!(version_time(SITE, Some(limit), None, 1000).is_err());
        assert_eq!(version_time(SITE, None, Some(limit - 1), 1000), Ok(limit));
    }

    #[test]
    fn a_previous_event_at_the_limit_asks_to_wait_instead_of_signing_an_older_one() {
        let limit = 1000 + MAX_FUTURE_SKEW - SIGN_MARGIN;
        let err = version_time(SITE, None, Some(limit), 1000)
            .unwrap_err()
            .to_string();
        assert!(
            err.starts_with(&format!(
                "your latest version on the relays is dated {} after",
                describe_secs(limit - 1000)
            )),
            "{err}"
        );
        assert!(err.ends_with("Wait 1 second and publish again"), "{err}");
        let err = version_time(SITE, Some(500), Some(1000 + MAX_FUTURE_SKEW), 1000)
            .unwrap_err()
            .to_string();
        assert!(
            err.ends_with(&format!(
                "Wait {} seconds and publish again",
                SIGN_MARGIN + 1
            )),
            "{err}"
        );
        assert!(version_time(SITE, None, Some(u64::MAX), 1000).is_err());
    }

    #[test]
    fn a_version_beyond_the_skew_is_refused_with_its_path() {
        let err = version_time(SITE, Some(1000 + 3 * 3600), Some(1000 + 7200), 1000)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("the newest version /swing/publish/k/s/11800"),
            "{err}"
        );
        assert!(err.contains("about 3 hours"), "{err}");
        assert!(err.contains("clock on this machine is behind"), "{err}");
        assert!(
            err.contains("`ipfs files rm -r /swing/publish/k/s/11800`"),
            "{err}"
        );
        assert!(
            err.contains("`docker compose exec ipfs ipfs files rm -r /swing/publish/k/s/11800`"),
            "{err}"
        );
    }

    #[test]
    fn a_version_within_the_skew_asks_to_wait_instead_of_removing_it() {
        let limit = sign_limit(1000);
        let err = version_time(SITE, Some(limit), None, 1000)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains(&format!("the newest version /swing/publish/k/s/{limit}")),
            "{err}"
        );
        assert!(err.contains("Wait 1 second"), "{err}");
        assert!(!err.contains("files rm"), "{err}");
        let edge = 1000 + MAX_FUTURE_SKEW;
        let err = version_time(SITE, Some(edge), None, 1000)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains(&format!("Wait {} seconds", SIGN_MARGIN + 1)),
            "{err}"
        );
        let beyond = version_time(SITE, Some(edge + 1), None, 1000)
            .unwrap_err()
            .to_string();
        assert!(beyond.contains("files rm"), "{beyond}");
    }

    #[test]
    fn the_clock_check_refuses_only_when_ahead_of_every_answering_relay() {
        let skew = MAX_FUTURE_SKEW as i64;
        let offsets = |a: i64, b: i64, c: i64| {
            vec![
                ("wss://a".to_string(), a),
                ("wss://b".to_string(), b),
                ("wss://c".to_string(), c),
            ]
        };
        assert!(refuse_clock_ahead(&offsets(2000, 6000, -2000)).is_ok());
        assert!(refuse_clock_ahead(&offsets(skew + 4000, skew + 8000, skew)).is_ok());
        assert!(refuse_clock_ahead(&offsets(-4500, -500, -8500)).is_ok());
        assert!(refuse_clock_ahead(&[]).is_ok());
        let err = refuse_clock_ahead(&offsets(skew + 4060, skew + 8060, skew + 60))
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "the clock on this machine is about 16 minutes ahead of every relay that answered (even wss://c, the one closest to it); fix the system clock and publish again"
        );
        let err = refuse_clock_ahead(&[("wss://b".to_string(), 6000)])
            .unwrap_err()
            .to_string();
        assert!(
            err.starts_with("the clock on this machine is about 100 minutes ahead of every relay that answered (even wss://b"),
            "{err}"
        );
    }

    #[test]
    fn skews_are_described_in_a_readable_unit() {
        assert_eq!(describe_secs(119), "119 seconds");
        assert_eq!(describe_secs(120), "about 2 minutes");
        assert_eq!(describe_secs(7200), "about 2 hours");
        assert_eq!(describe_secs(3 * 86_400 + 5), "about 3 days");
    }

    #[test]
    fn relay_urls_map_to_their_nip11_urls() {
        assert_eq!(
            relay_info_url("wss://relay.example/").as_deref(),
            Some("https://relay.example/")
        );
        assert_eq!(
            relay_info_url("ws://relay:8080").as_deref(),
            Some("http://relay:8080")
        );
        assert_eq!(
            relay_info_url("WSS://relay.example").as_deref(),
            Some("https://relay.example")
        );
        assert_eq!(relay_info_url("https://relay.example"), None);
        assert_eq!(relay_info_url("relay.example"), None);
    }

    #[test]
    fn http_dates_parse_to_unix_seconds() {
        assert_eq!(
            parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(784_111_777)
        );
        assert_eq!(parse_http_date("not a date"), None);
        assert_eq!(parse_http_date(""), None);
    }

    async fn serve_raw(response: &'static str) -> std::net::SocketAddr {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        addr
    }

    #[tokio::test]
    async fn the_probe_reads_the_date_header_and_skips_relays_without_one() {
        let with_date = axum::Router::new().route(
            "/",
            axum::routing::get(|headers: axum::http::HeaderMap| async move {
                let nip11 = headers
                    .get(ACCEPT)
                    .is_some_and(|v| v == "application/nostr+json");
                let status = if nip11 {
                    axum::http::StatusCode::OK
                } else {
                    axum::http::StatusCode::BAD_REQUEST
                };
                (status, [(DATE, "Sun, 06 Nov 1994 08:49:37 GMT")], "{}")
            }),
        );
        let dated = crate::test_support::serve_router(with_date).await;
        let undated = serve_raw(
            "HTTP/1.1 200 OK\r\ncontent-type: application/nostr+json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
        )
        .await;
        let relays = vec![
            format!("ws://{dated}"),
            format!("ws://{undated}"),
            "ws://127.0.0.1:1".to_string(),
            "https://not-a-relay.example".to_string(),
        ];
        let before = unix_now();
        let offsets = probe_relay_offsets(&relays).await;
        let after = unix_now();
        let [(relay, offset)] = offsets.as_slice() else {
            panic!("{offsets:?}");
        };
        assert_eq!(relay, &format!("ws://{dated}"));
        assert!(
            (before - 784_111_777..=after - 784_111_777).contains(offset),
            "{offset}"
        );
    }

    #[tokio::test]
    async fn the_probe_does_not_follow_redirects() {
        let target = crate::test_support::serve_router(axum::Router::new().route(
            "/",
            axum::routing::get(|| async { ([(DATE, "Sun, 06 Nov 1994 08:49:37 GMT")], "{}") }),
        ))
        .await;
        let redirect = crate::test_support::serve_router(axum::Router::new().route(
            "/",
            axum::routing::get(move || async move {
                (
                    axum::http::StatusCode::FOUND,
                    [
                        (axum::http::header::LOCATION, format!("http://{target}/")),
                        (DATE, "Mon, 07 Nov 1994 08:49:37 GMT".to_string()),
                    ],
                )
            }),
        ))
        .await;
        let before = unix_now();
        let offsets = probe_relay_offsets(&[format!("ws://{redirect}")]).await;
        let after = unix_now();
        let [(_, offset)] = offsets.as_slice() else {
            panic!("{offsets:?}");
        };
        let date = 784_111_777 + 86_400;
        assert!((before - date..=after - date).contains(offset), "{offset}");
    }
}
