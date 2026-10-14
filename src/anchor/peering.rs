use std::collections::BTreeSet;

use tracing::{info, warn};

use crate::ipfs::KuboNode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeeringPlan {
    pub add: Vec<String>,
    pub remove: Vec<String>,
    pub keep: BTreeSet<String>,
}

// Peerings already present but not added by SWING (Peering.Peers in the Kubo config) are neither re-added nor counted as ours, so they are never removed.
pub fn plan_peering(
    desired: &BTreeSet<String>,
    actual: &BTreeSet<String>,
    managed: &BTreeSet<String>,
) -> PeeringPlan {
    PeeringPlan {
        add: desired.difference(actual).cloned().collect(),
        remove: managed
            .difference(desired)
            .filter(|p| actual.contains(*p))
            .cloned()
            .collect(),
        keep: managed
            .intersection(desired)
            .filter(|p| actual.contains(*p))
            .cloned()
            .collect(),
    }
}

pub async fn apply_peering<K: KuboNode>(
    kubo: &K,
    desired: &BTreeSet<String>,
    managed: &BTreeSet<String>,
) -> anyhow::Result<BTreeSet<String>> {
    let actual = kubo.peering_list().await?;
    let plan = plan_peering(desired, &actual, managed);
    let mut now_managed = plan.keep;
    for peer in &plan.add {
        match kubo.peering_add(peer).await {
            Ok(()) => {
                info!(peer_id = %peer, "peering with an anchor");
                now_managed.insert(peer.clone());
            }
            Err(e) => {
                warn!(peer_id = %peer, error = %format!("{e:#}"), "adding an anchor peering failed; retrying on the next poll")
            }
        }
    }
    for peer in &plan.remove {
        match kubo.peering_remove(peer).await {
            Ok(()) => info!(peer_id = %peer, "stopped peering with an anchor"),
            Err(e) => {
                warn!(peer_id = %peer, error = %format!("{e:#}"), "removing an anchor peering failed; retrying on the next poll");
                now_managed.insert(peer.clone());
            }
        }
    }
    Ok(now_managed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::FakeKubo;

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn plan_adds_missing_and_removes_only_our_own_stale_peerings() {
        let plan = plan_peering(
            &set(&["keep", "new", "user", "lost"]),
            &set(&["keep", "user", "stale", "user-only"]),
            &set(&["keep", "stale", "lost", "gone"]),
        );
        assert_eq!(plan.add, ["lost", "new"]);
        assert_eq!(plan.remove, ["stale"]);
        assert_eq!(plan.keep, set(&["keep"]));
    }

    #[tokio::test]
    async fn apply_reconciles_against_kubo_and_leaves_user_peerings() {
        let kubo = FakeKubo::with(|s| {
            s.peering = set(&["user", "old"]);
        });
        let managed = apply_peering(&kubo, &set(&["a", "b"]), &set(&["old"]))
            .await
            .unwrap();
        assert_eq!(managed, set(&["a", "b"]));
        assert_eq!(kubo.s.lock().unwrap().peering, set(&["user", "a", "b"]));

        let managed = apply_peering(&kubo, &set(&["user"]), &managed)
            .await
            .unwrap();
        assert!(managed.is_empty());
        assert_eq!(kubo.s.lock().unwrap().peering, set(&["user"]));

        let managed = apply_peering(&kubo, &BTreeSet::new(), &managed)
            .await
            .unwrap();
        assert!(managed.is_empty());
        assert_eq!(kubo.s.lock().unwrap().peering, set(&["user"]));
    }

    #[tokio::test]
    async fn apply_re_adds_peerings_a_restarted_kubo_forgot() {
        let kubo = FakeKubo::default();
        let managed = apply_peering(&kubo, &set(&["a"]), &BTreeSet::new())
            .await
            .unwrap();
        kubo.s.lock().unwrap().peering.clear();
        let managed = apply_peering(&kubo, &set(&["a"]), &managed).await.unwrap();
        assert_eq!(managed, set(&["a"]));
        assert_eq!(kubo.s.lock().unwrap().peering, set(&["a"]));
    }

    #[tokio::test]
    async fn failed_calls_are_retried_and_keep_tracking() {
        let kubo = FakeKubo::with(|s| {
            s.peering = set(&["old"]);
            s.fail_peering = set(&["new", "old"]);
        });
        let managed = apply_peering(&kubo, &set(&["new"]), &set(&["old"]))
            .await
            .unwrap();
        assert_eq!(managed, set(&["old"]));
        kubo.s.lock().unwrap().fail_peering.clear();
        let managed = apply_peering(&kubo, &set(&["new"]), &managed)
            .await
            .unwrap();
        assert_eq!(managed, set(&["new"]));
        assert_eq!(kubo.s.lock().unwrap().peering, set(&["new"]));

        kubo.s.lock().unwrap().fail_peering_list = true;
        assert!(
            apply_peering(&kubo, &BTreeSet::new(), &managed)
                .await
                .is_err()
        );
    }
}
