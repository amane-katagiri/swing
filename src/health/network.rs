use crate::dashboard::dto::NetworkDto;
use crate::format::Sanitized;

pub(super) fn lines(network: Result<&NetworkDto, &str>) -> Vec<String> {
    let mut lines = reachability_lines(network);
    if let Ok(network) = network {
        lines.extend(anchor_lines(network));
    }
    lines
}

fn reachability_lines(network: Result<&NetworkDto, &str>) -> Vec<String> {
    let network = match network {
        Ok(network) => network,
        Err(reason) => return vec![format!("Reachability: unknown ({})", Sanitized(reason))],
    };
    let nat = match network.nat.as_deref() {
        Some("cgnat") => " (behind carrier-grade NAT)",
        Some("double-nat") => " (behind double NAT)",
        _ => "",
    };
    match network.reachability.as_str() {
        "public" => vec!["Reachability: public".to_string()],
        "offline" => vec![
            "Reachability: offline".to_string(),
            "  Kubo runs without network, so nothing is exchanged with other participants."
                .to_string(),
        ],
        "private" => {
            let mut lines = vec![
                format!("Reachability: private{nat}"),
                "  Other participants can fetch from this node only through hole punching.".to_string(),
                "  Accepting the swarm port (tcp and udp; 4001 unless [kubo].swarm_port is set) from outside helps.".to_string(),
            ];
            if !nat.is_empty() {
                lines.push(
                    "  Another NAT sits in front of your router, so forwarding the port on your router alone may not be enough."
                        .to_string(),
                );
            }
            lines.push(format!("  See {}", Sanitized(&network.guide)));
            lines
        }
        _ => vec![
            format!("Reachability: unknown{nat}"),
            "  Kubo can take a few minutes after it starts to find out.".to_string(),
        ],
    }
}

fn anchor_lines(network: &NetworkDto) -> Vec<String> {
    let anchor = &network.anchor;
    let mut lines = Vec::new();
    if anchor.advertising {
        lines.push("Anchor: advertising".to_string());
    }
    if !anchor.anchors.is_empty() {
        let total = anchor.anchors.len();
        if anchor.anchors.iter().all(|a| a.connected.is_none()) {
            lines.push(format!("Anchors: {total} selected"));
        } else {
            let connected = anchor
                .anchors
                .iter()
                .filter(|a| a.connected == Some(true))
                .count();
            lines.push(format!("Anchors: {connected} of {total} connected"));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::dto::AccountDto;
    use crate::dashboard::dto::network::{AnchorDto, AnchorPeerDto};

    fn network(reachability: &str, nat: Option<&str>) -> NetworkDto {
        NetworkDto {
            reachability: reachability.to_string(),
            nat: nat.map(str::to_string),
            reachable: vec![],
            unreachable: vec![],
            unknown: vec![],
            guide: "https://example.invalid/guide#x".to_string(),
            anchor: AnchorDto {
                advertise: true,
                advertising: false,
                max_peers: 4,
                anchors: vec![],
            },
        }
    }

    #[test]
    fn anchor_lines_show_only_what_is_happening() {
        let mut n = network("public", None);
        assert!(anchor_lines(&n).is_empty());
        n.anchor.advertising = true;
        let anchor = |connected| AnchorPeerDto {
            peer_id: "p".to_string(),
            account: AccountDto {
                pubkey: String::new(),
                npub: String::new(),
                fingerprint: vec![],
            },
            reason: "mutual".to_string(),
            sites: 0,
            connected,
        };
        n.anchor.anchors = vec![anchor(Some(true)), anchor(Some(false)), anchor(None)];
        assert_eq!(
            anchor_lines(&n),
            ["Anchor: advertising", "Anchors: 1 of 3 connected"]
        );
        n.anchor.advertising = false;
        n.anchor.anchors = vec![anchor(None), anchor(None)];
        assert_eq!(anchor_lines(&n), ["Anchors: 2 selected"]);
    }

    #[test]
    fn reachability_lines_explain_each_state() {
        assert_eq!(
            reachability_lines(Ok(&network("public", None))),
            ["Reachability: public"]
        );

        let private = reachability_lines(Ok(&network("private", None)));
        assert_eq!(private[0], "Reachability: private");
        assert!(private[1].contains("hole punching"), "{private:?}");
        assert!(private[2].contains("tcp and udp"), "{private:?}");
        assert_eq!(
            private.last().unwrap(),
            "  See https://example.invalid/guide#x"
        );
        assert_eq!(private.len(), 4);

        let cgnat = reachability_lines(Ok(&network("private", Some("cgnat"))));
        assert_eq!(cgnat[0], "Reachability: private (behind carrier-grade NAT)");
        assert!(cgnat[3].contains("Another NAT"), "{cgnat:?}");

        let offline = reachability_lines(Ok(&network("offline", None)));
        assert_eq!(offline[0], "Reachability: offline");
        assert!(offline[1].contains("without network"), "{offline:?}");
        assert_eq!(offline.len(), 2);

        let unknown = reachability_lines(Ok(&network("unknown", None)));
        assert_eq!(unknown[0], "Reachability: unknown");
        assert!(unknown[1].contains("few minutes"), "{unknown:?}");
    }

    #[test]
    fn reachability_lines_show_the_reason_when_the_call_fails() {
        assert_eq!(
            reachability_lines(Err("agent is not ready")),
            ["Reachability: unknown (agent is not ready)"]
        );
        let lines = reachability_lines(Err("bad\nline"));
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].contains('\n'), "{lines:?}");
    }
}
