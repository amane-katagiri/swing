use std::fmt;
use std::time::{Duration, UNIX_EPOCH};

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
        0..120 => format!("{secs} seconds"),
        120..7200 => format!("about {} minutes", secs / 60),
        7200..172_800 => format!("about {} hours", secs / 3600),
        _ => format!("about {} days", secs / 86_400),
    }
}

fn refuse_future_version(site_path: &str, newest: u64, now: u64) -> Result<(), ClockError> {
    if nostr::plausible_at(newest.saturating_add(1), now) {
        return Ok(());
    }
    let path = format!("{site_path}/{newest}");
    Err(ClockError(format!(
        "the newest version {path} is dated {} after this machine's clock, and a site event dated after it would be dropped as from the future. \
         Either the clock on this machine is behind now (fix the system clock and publish again), \
         or that version was created while the clock was ahead (remove it with `ipfs files rm -r {path}` and publish again; \
         with Docker Compose, run it in the ipfs container: `docker compose exec ipfs ipfs files rm -r {path}`)",
        describe_secs(newest - now)
    )))
}

fn refuse_future_previous(previous: u64, now: u64) -> Result<(), ClockError> {
    if nostr::plausible_at(previous.saturating_add(1), now) {
        return Ok(());
    }
    Err(ClockError(format!(
        "your latest site event on the relays is dated {} after this machine's clock (created_at {previous}), \
         and the relays keep it over any event dated before it, so a new event would not replace it. \
         Either the clock on this machine is behind now (fix the system clock and publish again), \
         or that event was published while a clock was ahead (wait until that time has passed and publish again)",
        describe_secs(previous - now)
    )))
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
        refuse_future_previous(previous, now)?;
    }
    Ok([newest, previous]
        .into_iter()
        .flatten()
        .map(|t| t.saturating_add(1))
        .fold(now, u64::max))
}

pub(super) fn refuse_clock_ahead(
    now: u64,
    relay_times: &[(String, u64)],
) -> Result<(), ClockError> {
    let Some((relay, time)) = relay_times.iter().max_by_key(|(_, time)| *time) else {
        return Ok(());
    };
    if nostr::plausible_at(now, *time) {
        return Ok(());
    }
    Err(ClockError(format!(
        "the clock on this machine is {} ahead of every relay that answered (even {relay}, the one closest to it); fix the system clock and publish again",
        describe_secs(now - time)
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

async fn probe_relay_time(client: &reqwest::Client, relay: &str) -> Option<u64> {
    let url = relay_info_url(relay)?;
    let resp = client
        .get(url)
        .header(ACCEPT, "application/nostr+json")
        .send()
        .await
        .ok()?;
    parse_http_date(resp.headers().get(DATE)?.to_str().ok()?)
}

// The relay connections themselves bypass any HTTP proxy, so the probe does too to time the same servers.
pub(super) async fn probe_relay_times(relays: &[String]) -> Vec<(String, u64)> {
    let Ok(client) = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(PROBE_TIMEOUT)
        .build()
    else {
        return Vec::new();
    };
    let times = join_all(relays.iter().map(|relay| probe_relay_time(&client, relay))).await;
    relays
        .iter()
        .zip(times)
        .filter_map(|(relay, time)| time.map(|t| (relay.clone(), t)))
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
    fn the_version_time_never_goes_beyond_the_skew() {
        let edge = 1000 + MAX_FUTURE_SKEW - 1;
        assert_eq!(
            version_time(SITE, Some(edge), Some(edge), 1000),
            Ok(1000 + MAX_FUTURE_SKEW)
        );
        assert!(version_time(SITE, Some(edge + 1), None, 1000).is_err());
        assert!(version_time(SITE, None, Some(edge + 1), 1000).is_err());
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
    fn a_previous_event_beyond_the_skew_is_refused() {
        let err = version_time(SITE, Some(500), Some(1000 + 2 * 3600), 1000)
            .unwrap_err()
            .to_string();
        assert!(
            err.starts_with("your latest site event on the relays is dated about 2 hours after this machine's clock (created_at 8200)"),
            "{err}"
        );
        assert!(err.contains("wait until that time has passed"), "{err}");
    }

    #[test]
    fn the_clock_check_refuses_only_when_ahead_of_every_answering_relay() {
        let times = vec![
            ("wss://a".to_string(), 5000),
            ("wss://b".to_string(), 1000),
            ("wss://c".to_string(), 9000),
        ];
        assert!(refuse_clock_ahead(7000, &times).is_ok());
        assert!(refuse_clock_ahead(9000 + MAX_FUTURE_SKEW, &times).is_ok());
        assert!(refuse_clock_ahead(500, &times).is_ok());
        assert!(refuse_clock_ahead(1_000_000, &[]).is_ok());
        let err = refuse_clock_ahead(9000 + MAX_FUTURE_SKEW + 60, &times)
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "the clock on this machine is about 16 minutes ahead of every relay that answered (even wss://c, the one closest to it); fix the system clock and publish again"
        );
        let err = refuse_clock_ahead(7000, &times[1..2])
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
        assert_eq!(
            probe_relay_times(&relays).await,
            vec![(format!("ws://{dated}"), 784_111_777)]
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
        assert_eq!(
            probe_relay_times(&[format!("ws://{redirect}")]).await,
            vec![(format!("ws://{redirect}"), 784_111_777 + 86_400)]
        );
    }
}
