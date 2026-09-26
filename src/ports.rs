use std::io;
use std::net::SocketAddr;

use tokio::net::TcpListener;

use crate::config::{Config, Source};

pub const PROBE_COUNT: u16 = 20;

pub fn may_shift(config: &Config, key: &str) -> bool {
    config.source_of(key) != Some(Source::Env)
}

fn candidates(addr: SocketAddr) -> impl Iterator<Item = SocketAddr> {
    let ip = addr.ip();
    (0..=PROBE_COUNT)
        .map_while(move |i| addr.port().checked_add(i))
        .chain(std::iter::once(0))
        .map(move |port| SocketAddr::new(ip, port))
}

fn is_taken(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::AddrInUse | io::ErrorKind::PermissionDenied
    )
}

pub async fn bind_shifting(addr: SocketAddr) -> io::Result<TcpListener> {
    let mut last = None;
    for candidate in candidates(addr) {
        match TcpListener::bind(candidate).await {
            Ok(listener) => return Ok(listener),
            Err(e) if is_taken(&e) => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.expect("candidates is never empty"))
}

pub async fn free_addr(addr: SocketAddr) -> io::Result<SocketAddr> {
    bind_shifting(addr).await?.local_addr()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn keeps_a_free_port() {
        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);
        let listener = bind_shifting(addr).await.unwrap();
        assert_eq!(listener.local_addr().unwrap(), addr);
    }

    #[tokio::test]
    async fn moves_off_a_taken_port() {
        let taken = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = taken.local_addr().unwrap();
        let listener = bind_shifting(addr).await.unwrap();
        let bound = listener.local_addr().unwrap();
        assert_ne!(bound.port(), addr.port());
        assert_eq!(bound.ip(), addr.ip());
    }

    #[test]
    fn candidates_stop_at_the_top_port_and_end_with_an_ephemeral_one() {
        let addr: SocketAddr = "127.0.0.1:65534".parse().unwrap();
        let ports: Vec<u16> = candidates(addr).map(|a| a.port()).collect();
        assert_eq!(ports, vec![65534, 65535, 0]);
    }

    #[test]
    fn env_sourced_addresses_never_shift() {
        let config = crate::config::build_config_from_str("", |k| {
            (k == "SWING_DASHBOARD_LISTEN").then(|| "127.0.0.1:9000".to_string())
        })
        .unwrap();
        assert!(!may_shift(&config, "dashboard.listen"));
        assert!(may_shift(&config, "kubo.gateway_listen"));
    }
}
