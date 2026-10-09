use nostr_sdk::prelude::PublicKey;
use serde::{Deserialize, Serialize};

use crate::health;
use crate::mirror;
use crate::nostr;
use crate::replicas;
use crate::signer::{PairingState, SignFailure, Signer};
use crate::webring;

#[derive(Debug, Serialize, Deserialize)]
pub struct PubkeyDto {
    pub pubkey: String,
    pub npub: String,
}

impl PubkeyDto {
    pub fn new(pk: &PublicKey) -> Self {
        Self {
            pubkey: pk.to_hex(),
            npub: mirror::npub(pk),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RelayResultDto {
    pub relay: String,
    pub ok: bool,
    pub error: Option<String>,
}

const MAX_RELAY_ERROR_CHARS: usize = 500;

impl From<&nostr::RelaySendResult> for RelayResultDto {
    fn from(r: &nostr::RelaySendResult) -> Self {
        Self {
            relay: r.relay.clone(),
            ok: r.ok,
            error: r
                .error
                .as_deref()
                .map(|e| crate::format::sanitize_display_text(e, MAX_RELAY_ERROR_CHARS)),
        }
    }
}

pub(super) fn relay_results_dto(results: &[nostr::RelaySendResult]) -> Vec<RelayResultDto> {
    results.iter().map(RelayResultDto::from).collect()
}

fn strip_parens(note: &str) -> String {
    note.strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(note)
        .to_string()
}

pub fn gateway_url(gateway: Option<&str>, cid: &str, stored: bool) -> Option<String> {
    if !stored {
        return None;
    }
    let gateway = gateway?;
    if gateway.is_empty() {
        return None;
    }
    Some(format!("{}/ipfs/{cid}/", gateway.trim_end_matches('/')))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IdentityRequestDto {
    pub nonce: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IdentityDto {
    pub proof: String,
    pub instance: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginCodeDto {
    pub code: String,
    pub expires_in: u64,
}

#[derive(Debug, Serialize)]
pub struct ActivityDto {
    pub latest_stored_at: Option<u64>,
    pub latest_published_at: Option<u64>,
    pub latest_replica_report_at: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StatsDto {
    pub interval: u64,
    pub kubo_managed: bool,
    pub samples: Vec<crate::stats::Sample>,
}

#[derive(Debug, Serialize)]
pub struct OverviewDto {
    pub version: String,
    pub setup: bool,
    pub pubkey: Option<String>,
    pub npub: Option<String>,
    pub relays: Vec<String>,
    pub mirror_set: String,
    pub gateway: Option<String>,
    pub started_at: u64,
    pub instance: String,
    pub max_upload: u64,
    pub signer: Option<SignerDto>,
}

#[derive(Debug, Serialize)]
pub struct SignerDto {
    pub remote: bool,
    pub relays: Vec<String>,
    pub last_failure: Option<SignFailure>,
}

pub fn signer_dto(signer: &Signer) -> SignerDto {
    SignerDto {
        remote: signer.is_remote(),
        relays: signer.signer_relays().to_vec(),
        last_failure: signer.last_failure(),
    }
}

#[derive(Debug, Serialize)]
pub struct PairingStartDto {
    pub uri: String,
    pub qr_svg: String,
}

#[derive(Debug, Serialize)]
pub struct PairingStatusDto {
    pub state: &'static str,
    pub npub: Option<String>,
    pub probe_signed: Option<bool>,
    pub error: Option<String>,
}

pub fn pairing_status_dto(state: Option<&PairingState>) -> PairingStatusDto {
    let empty = PairingStatusDto {
        state: "idle",
        npub: None,
        probe_signed: None,
        error: None,
    };
    match state {
        None => empty,
        Some(PairingState::Waiting) => PairingStatusDto {
            state: "waiting",
            ..empty
        },
        Some(PairingState::Checking { user }) => PairingStatusDto {
            state: "checking",
            npub: Some(mirror::npub(user)),
            ..empty
        },
        Some(PairingState::Ready(paired)) => PairingStatusDto {
            state: "ready",
            npub: Some(mirror::npub(&paired.user)),
            probe_signed: Some(paired.probe_signed),
            error: paired.probe_error.clone(),
        },
        Some(PairingState::Failed(error)) => PairingStatusDto {
            state: "failed",
            error: Some(error.clone()),
            ..empty
        },
    }
}

#[derive(Debug, Serialize)]
pub struct FollowSetStatusDto {
    pub found: bool,
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SiteDto {
    pub d: String,
    pub cid: String,
    pub url: Option<String>,
    pub size: Option<u64>,
    pub stored_size: Option<u64>,
    pub stored_at: Option<u64>,
    pub created_at: u64,
    pub title: Option<String>,
    pub message: Option<String>,
    pub nip05: Option<String>,
    pub replicas: Option<usize>,
    pub unverified_replicas: Option<usize>,
    pub stored: bool,
    pub gateway_url: Option<String>,
    pub previous: Option<PreviousVersionDto>,
}

#[derive(Debug, Serialize)]
pub struct PreviousVersionDto {
    pub cid: String,
    pub created_at: u64,
    pub stored_at: u64,
    pub stored_size: u64,
    pub gateway_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AccountSitesDto {
    pub pubkey: String,
    pub npub: String,
    pub sites: Vec<SiteDto>,
}

fn site_dto(site: &mirror::SiteRow, gateway: Option<&str>) -> SiteDto {
    SiteDto {
        d: site.d.clone(),
        cid: site.cid.clone(),
        url: site.url.clone(),
        size: site.size,
        stored_size: site.stored_size,
        stored_at: site.stored_at,
        created_at: site.created_at,
        title: site.title.clone(),
        message: site.message.clone(),
        nip05: site.nip05.clone(),
        replicas: site.replicas.map(|c| c.trusted),
        unverified_replicas: site.replicas.map(|c| c.unverified),
        stored: site.stored,
        gateway_url: gateway_url(gateway, &site.cid, site.stored),
        previous: site.previous.as_ref().map(|v| PreviousVersionDto {
            cid: v.cid.clone(),
            created_at: v.created_at,
            stored_at: v.stored_at,
            stored_size: v.size,
            gateway_url: gateway_url(gateway, &v.cid, true),
        }),
    }
}

fn account_sites_dto(account: &mirror::AccountSites, gateway: Option<&str>) -> AccountSitesDto {
    AccountSitesDto {
        pubkey: account.pubkey.to_hex(),
        npub: mirror::npub(&account.pubkey),
        sites: account.sites.iter().map(|s| site_dto(s, gateway)).collect(),
    }
}

#[derive(Debug, Serialize)]
pub struct UnfollowedDto {
    pub remove_on_unfollow: bool,
    pub accounts: Vec<AccountSitesDto>,
}

#[derive(Debug, Serialize)]
pub struct SitesDto {
    pub follow_set: FollowSetStatusDto,
    pub accounts: Vec<AccountSitesDto>,
    pub replicas_error: Option<String>,
    pub unfollowed: UnfollowedDto,
}

pub fn sites_dto(view: &mirror::SitesView, gateway: Option<&str>) -> SitesDto {
    SitesDto {
        follow_set: FollowSetStatusDto {
            found: view.follow_set_found,
            note: view.follow_note.map(strip_parens),
        },
        accounts: view
            .accounts
            .iter()
            .map(|a| account_sites_dto(a, gateway))
            .collect(),
        replicas_error: view.replicas_error.clone(),
        unfollowed: UnfollowedDto {
            remove_on_unfollow: view.remove_on_unfollow,
            accounts: view
                .unfollowed
                .iter()
                .map(|a| account_sites_dto(a, gateway))
                .collect(),
        },
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VersionStatusDto {
    pub pubkey: Option<String>,
    pub npub: Option<String>,
    pub d: Option<String>,
    pub path: Option<String>,
    pub cid: Option<String>,
    pub size: Option<u64>,
    pub created_at: Option<u64>,
    pub health: String,
    pub detail: Option<String>,
}

fn health_token(health: &health::VersionHealth) -> &'static str {
    match health {
        health::VersionHealth::Ok => "ok",
        health::VersionHealth::Missing => "missing",
        health::VersionHealth::Mismatch(_) => "cid_mismatch",
        health::VersionHealth::Incomplete(_) => "incomplete",
        health::VersionHealth::CheckFailed(_) => "check_failed",
    }
}

fn version_status_dto(v: &health::VersionStatus) -> VersionStatusDto {
    let detail = match &v.health {
        health::VersionHealth::Ok => None,
        other => Some(format!("{other}")),
    };
    VersionStatusDto {
        pubkey: Some(v.pubkey.to_hex()),
        npub: Some(mirror::npub(&v.pubkey)),
        d: Some(v.d.clone()),
        path: Some(v.path.clone()),
        cid: Some(v.cid.clone()),
        size: Some(v.size),
        created_at: Some(v.created_at),
        health: health_token(&v.health).to_string(),
        detail,
    }
}

fn invalid_key_status_dto(key: &str, cid: &str) -> VersionStatusDto {
    VersionStatusDto {
        pubkey: None,
        npub: None,
        d: None,
        path: None,
        cid: Some(cid.to_string()),
        size: None,
        created_at: None,
        health: "invalid_key".to_string(),
        detail: Some(key.to_string()),
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GarbageDto {
    pub path: String,
    pub list_failed: bool,
    pub list_failed_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SiteSizeDto {
    pub pubkey: String,
    pub npub: String,
    pub d: String,
    pub path: String,
    pub actual: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StatusDto {
    pub versions: Vec<VersionStatusDto>,
    pub sites: Vec<SiteSizeDto>,
    pub actual_bytes: Option<u64>,
    pub garbage: Vec<GarbageDto>,
    pub problems: usize,
}

pub fn status_dto(report: &health::StatusReport) -> StatusDto {
    let mut garbage: Vec<GarbageDto> = report
        .garbage
        .paths
        .iter()
        .map(|p| GarbageDto {
            path: p.clone(),
            list_failed: false,
            list_failed_reason: None,
        })
        .collect();
    garbage.extend(
        report
            .garbage
            .unlisted
            .iter()
            .map(|(p, reason)| GarbageDto {
                path: p.clone(),
                list_failed: true,
                list_failed_reason: Some(reason.clone()),
            }),
    );
    let versions = report
        .lines
        .iter()
        .map(|line| match line {
            health::StatusLine::Version(v) => version_status_dto(v),
            health::StatusLine::InvalidKey { key, cid } => invalid_key_status_dto(key, cid),
        })
        .collect();
    let sites = report
        .sites
        .iter()
        .map(|s| SiteSizeDto {
            pubkey: s.pubkey.to_hex(),
            npub: mirror::npub(&s.pubkey),
            d: s.d.clone(),
            path: s.path.clone(),
            actual: s.actual,
        })
        .collect();
    StatusDto {
        versions,
        sites,
        actual_bytes: report.actual_bytes(),
        garbage,
        problems: report.problems,
    }
}

#[derive(Debug, Serialize)]
pub struct MirrorListDto {
    pub title: Option<String>,
    pub note: Option<String>,
    pub members: Vec<PubkeyDto>,
}

pub fn mirror_list_dto(view: &mirror::MirrorListView) -> MirrorListDto {
    MirrorListDto {
        title: view
            .set
            .as_ref()
            .and_then(|s| s.title().map(str::to_string)),
        note: view.note.map(strip_parens),
        members: view
            .set
            .as_ref()
            .map(|s| s.pubkeys().iter().map(PubkeyDto::new).collect())
            .unwrap_or_default(),
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MirrorChangeDto {
    pub changed: Vec<PubkeyDto>,
    pub unchanged: Vec<PubkeyDto>,
    pub published: bool,
    pub relays: Vec<RelayResultDto>,
    pub members: Vec<PubkeyDto>,
    pub note: Option<String>,
    pub follow_set_found: bool,
}

pub fn mirror_change_dto(change: &mirror::MirrorChange) -> MirrorChangeDto {
    MirrorChangeDto {
        changed: change.changed.iter().map(PubkeyDto::new).collect(),
        unchanged: change.unchanged.iter().map(PubkeyDto::new).collect(),
        published: change.published,
        relays: relay_results_dto(&change.relay_results),
        members: change.set.pubkeys().iter().map(PubkeyDto::new).collect(),
        note: change.note.map(str::to_string),
        follow_set_found: change.follow_set_found,
    }
}

#[derive(Debug, Serialize)]
pub struct WebringNodeDto {
    pub pubkey: String,
    pub npub: String,
    pub short_npub: String,
    pub names: Vec<String>,
    pub label: String,
    pub depth: usize,
    pub root: bool,
    pub has_follow_set: bool,
}

#[derive(Debug, Serialize)]
pub struct WebringEdgeDto {
    pub from: String,
    pub to: String,
    pub mutual: bool,
}

#[derive(Debug, Serialize)]
pub struct ReferencingDto {
    pub accounts: Vec<PubkeyDto>,
    pub more: usize,
}

#[derive(Debug, Serialize)]
pub struct WebringDto {
    pub depth: usize,
    pub nodes: Vec<WebringNodeDto>,
    pub edges: Vec<WebringEdgeDto>,
    pub beyond: usize,
    pub over_budget: usize,
    pub referencing: ReferencingDto,
    pub text: String,
    pub dot: String,
    pub mermaid: String,
}

pub fn webring_dto(view: &webring::WebringView) -> WebringDto {
    let labels = webring::text_labels(&view.graph.nodes, &view.names);
    let mut nodes: Vec<(&PublicKey, &usize)> = view.graph.nodes.iter().collect();
    nodes.sort_by_key(|(pk, depth)| (**depth, labels[*pk].clone()));
    let node_dtos = nodes
        .into_iter()
        .map(|(pk, depth)| WebringNodeDto {
            pubkey: pk.to_hex(),
            npub: mirror::npub(pk),
            short_npub: webring::short_npub(pk),
            names: view.name_lists.get(pk).cloned().unwrap_or_default(),
            label: labels[pk].clone(),
            depth: *depth,
            root: *depth == 0,
            has_follow_set: !view.graph.without_follow_set.contains(pk),
        })
        .collect();

    let links = webring::split_links(&view.graph, |pk| *pk);
    let mut edges: Vec<WebringEdgeDto> = links
        .mutual
        .iter()
        .map(|(a, b)| WebringEdgeDto {
            from: a.to_hex(),
            to: b.to_hex(),
            mutual: true,
        })
        .collect();
    edges.extend(links.one_way.iter().map(|(a, b)| WebringEdgeDto {
        from: a.to_hex(),
        to: b.to_hex(),
        mutual: false,
    }));

    WebringDto {
        depth: view.depth,
        nodes: node_dtos,
        edges,
        beyond: view.graph.beyond,
        over_budget: view.graph.over_budget,
        referencing: ReferencingDto {
            accounts: view.referencing.iter().map(PubkeyDto::new).collect(),
            more: view.referencing_dropped,
        },
        text: webring::render_text(
            &view.graph,
            &view.names,
            &view.mirror_set,
            view.depth,
            &view.referencing,
            view.referencing_dropped,
        ),
        dot: webring::render_dot(&view.graph, &view.names),
        mermaid: webring::render_mermaid(&view.graph, &view.names),
    }
}

#[derive(Debug, Serialize)]
pub struct ReporterDto {
    pub pubkey: String,
    pub npub: String,
    pub latest: bool,
    pub tier: String,
}

#[derive(Debug, Serialize)]
pub struct SiteReplicasDto {
    pub d: String,
    pub cid: String,
    pub replicas: usize,
    pub unverified: usize,
    pub reports: usize,
    pub dropped: usize,
    pub reporters: Vec<ReporterDto>,
}

#[derive(Debug, Serialize)]
pub struct AuthorReplicasDto {
    pub pubkey: String,
    pub npub: String,
    pub sites: Vec<SiteReplicasDto>,
}

#[derive(Debug, Serialize)]
pub struct ReplicasDto {
    pub authors: Vec<AuthorReplicasDto>,
}

pub fn replicas_dto(authors: &[replicas::AuthorReplicas]) -> ReplicasDto {
    ReplicasDto {
        authors: authors
            .iter()
            .map(|a| AuthorReplicasDto {
                pubkey: a.pubkey.to_hex(),
                npub: mirror::npub(&a.pubkey),
                sites: a
                    .sites
                    .iter()
                    .map(|s| SiteReplicasDto {
                        d: s.d.clone(),
                        cid: s.cid.clone(),
                        replicas: s.replicas,
                        unverified: s.unverified,
                        reports: s.reports,
                        dropped: s.dropped,
                        reporters: s
                            .reporters
                            .iter()
                            .map(|r| ReporterDto {
                                pubkey: r.pubkey.to_hex(),
                                npub: mirror::npub(&r.pubkey),
                                latest: r.latest,
                                tier: r.tier.as_str().to_string(),
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Nip05ResultDto {
    pub status: String,
    pub detail: Option<String>,
}

pub fn nip05_off_dto() -> Nip05ResultDto {
    Nip05ResultDto {
        status: "off".to_string(),
        detail: None,
    }
}

pub fn nip05_result_dto(result: &crate::nip05::VerificationResult) -> Nip05ResultDto {
    if let Some(raw) = result.detail() {
        tracing::warn!(detail = %raw, "nip05 verification error (dashboard)");
    }
    Nip05ResultDto {
        status: result.as_state_str().to_string(),
        detail: result.coarse_detail().map(str::to_string),
    }
}

#[derive(Debug, Serialize)]
pub struct DotfilesCheckDto {
    pub status: &'static str,
    pub mode: &'static str,
    pub count: usize,
    pub paths: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SizeCheckDto {
    pub status: &'static str,
    pub mode: &'static str,
    pub bytes: Option<u64>,
    pub threshold: u64,
}

#[derive(Debug, Serialize)]
pub struct LinkItemDto {
    pub file: String,
    pub reference: String,
    pub files: usize,
}

#[derive(Debug, Serialize)]
pub struct LinkKindDto {
    pub kind: &'static str,
    pub blocks: bool,
    pub count: usize,
    pub items: Vec<LinkItemDto>,
}

#[derive(Debug, Serialize)]
pub struct LinksCheckDto {
    pub status: &'static str,
    pub mode: &'static str,
    pub count: usize,
    pub blocking: usize,
    pub redirects: bool,
    pub skipped: usize,
    pub kinds: Vec<LinkKindDto>,
    pub guide: Option<&'static str>,
}

fn links_check_dto(local: &crate::publish::LocalChecks) -> LinksCheckDto {
    let mode = local.links_mode.name();
    let Some(report) = &local.links else {
        return LinksCheckDto {
            status: "off",
            mode,
            count: 0,
            blocking: 0,
            redirects: false,
            skipped: 0,
            kinds: Vec::new(),
            guide: None,
        };
    };
    let count = report.total();
    LinksCheckDto {
        status: if count == 0 { "ok" } else { "found" },
        mode,
        count,
        blocking: report.blocking(),
        redirects: report.redirects,
        skipped: report.skipped.len(),
        kinds: report
            .kinds()
            .map(|(kind, findings)| LinkKindDto {
                kind: kind.name(),
                blocks: report.blocks(kind),
                count: findings.len(),
                items: findings
                    .iter()
                    .take(crate::publish::links::LISTED_LINKS)
                    .map(|f| LinkItemDto {
                        file: f.file.clone(),
                        reference: f.reference.clone(),
                        files: f.files,
                    })
                    .collect(),
            })
            .collect(),
        guide: (count > 0).then_some(crate::publish::links::SITE_GUIDE_URL),
    }
}

#[derive(Debug, Serialize)]
pub struct UnchangedCheckDto {
    pub status: &'static str,
    pub mode: &'static str,
    pub previous_cid: Option<String>,
    pub previous_created_at: Option<u64>,
    pub detail: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PublishChecksDto {
    pub dotfiles: DotfilesCheckDto,
    pub size: SizeCheckDto,
    pub links: LinksCheckDto,
    pub unchanged: Option<UnchangedCheckDto>,
}

pub fn publish_checks_dto(
    local: &crate::publish::LocalChecks,
    unchanged: Option<&crate::publish::UnchangedOutcome>,
) -> PublishChecksDto {
    let dotfiles = match &local.dotfiles {
        None => DotfilesCheckDto {
            status: "off",
            mode: local.dotfiles_mode.name(),
            count: 0,
            paths: Vec::new(),
        },
        Some(found) => DotfilesCheckDto {
            status: if found.is_empty() { "ok" } else { "found" },
            mode: local.dotfiles_mode.name(),
            count: found.len(),
            paths: found
                .iter()
                .take(crate::publish::LISTED_DOTFILES)
                .cloned()
                .collect(),
        },
    };
    let size = SizeCheckDto {
        status: match local.bytes {
            None => "off",
            Some(_) if local.size_over() => "over",
            Some(_) => "ok",
        },
        mode: local.size_mode.name(),
        bytes: local.bytes,
        threshold: crate::publish::SIZE_GUIDELINE,
    };
    let unchanged = unchanged.map(|u| UnchangedCheckDto {
        status: u.status.as_str(),
        mode: u.mode.name(),
        previous_cid: u.previous_cid.clone(),
        previous_created_at: u.previous_created_at,
        detail: u.detail.clone(),
    });
    PublishChecksDto {
        dotfiles,
        size,
        links: links_check_dto(local),
        unchanged,
    }
}

#[derive(Debug, Serialize)]
pub struct PublishResultDto {
    pub published: bool,
    pub site: String,
    pub url: Option<String>,
    pub title: Option<String>,
    pub message: Option<String>,
    pub nip05: Nip05ResultDto,
    pub checks: PublishChecksDto,
    pub cid: String,
    pub size: u64,
    pub created_at: Option<u64>,
    pub mfs_path: Option<String>,
    pub relays: Vec<RelayResultDto>,
    pub pruned: Vec<String>,
    pub prune_error: Option<String>,
    pub gateway_url: Option<String>,
    pub note: Option<NoteResultDto>,
}

#[derive(Debug, Serialize)]
pub struct NoteResultDto {
    pub relays: Vec<RelayResultDto>,
    pub error: Option<String>,
}

pub(super) fn note_result_dto(sent: anyhow::Result<Vec<nostr::RelaySendResult>>) -> NoteResultDto {
    match sent {
        Ok(results) => NoteResultDto {
            error: (!results.iter().any(|r| r.ok))
                .then(|| crate::publish::NO_RELAY_ACCEPTED_NOTE.to_string()),
            relays: relay_results_dto(&results),
        },
        Err(e) => NoteResultDto {
            relays: Vec::new(),
            error: Some(format!("{e:#}")),
        },
    }
}

#[derive(Debug, Serialize)]
pub struct PublishUploadResultDto {
    #[serde(flatten)]
    pub result: PublishResultDto,
    pub files: usize,
}

#[derive(Debug, Serialize)]
pub struct PublishSiteDto {
    pub d: String,
    pub url: Option<String>,
    pub cid: String,
    pub size: Option<u64>,
    pub created_at: u64,
    pub title: Option<String>,
    pub message: Option<String>,
    pub gateway_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PublishSitesDto {
    pub sites: Vec<PublishSiteDto>,
}

pub fn publish_sites_dto(
    events: &[&crate::nostr::SiteEvent],
    gateway: Option<&str>,
) -> PublishSitesDto {
    PublishSitesDto {
        sites: events
            .iter()
            .map(|ev| PublishSiteDto {
                d: ev.d.clone(),
                url: ev.url.clone(),
                cid: ev.cid.clone(),
                size: ev.size,
                created_at: ev.created_at,
                title: ev.title.clone(),
                message: ev.message.clone(),
                gateway_url: gateway_url(gateway, &ev.cid, true),
            })
            .collect(),
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PreviousFilesDto {
    pub status: &'static str,
    pub previous_cid: Option<String>,
    pub previous_created_at: Option<u64>,
    pub detail: Option<String>,
    pub files: Vec<String>,
}

pub fn previous_files_dto(previous: crate::publish::PreviousFiles) -> PreviousFilesDto {
    use crate::publish::PreviousFiles;
    let empty = |status, detail| PreviousFilesDto {
        status,
        previous_cid: None,
        previous_created_at: None,
        detail,
        files: Vec::new(),
    };
    match previous {
        PreviousFiles::NoPrevious => empty("no_previous", None),
        PreviousFiles::Unknown(reason) => empty("unknown", Some(reason)),
        PreviousFiles::Listed {
            cid,
            created_at,
            paths,
        } => PreviousFilesDto {
            status: "listed",
            previous_cid: Some(cid),
            previous_created_at: Some(created_at),
            detail: None,
            files: paths,
        },
    }
}

pub(crate) use super::config_dto::{ConfigDto, config_dto};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::{Garbage, StatusLine, StatusReport, VersionHealth, VersionStatus};
    use crate::nip05::{ErrorCategory, VerificationResult};
    use nostr_sdk::prelude::Keys;
    use std::path::PathBuf;

    #[test]
    fn nip05_result_dto_coarsens_error_detail_and_keeps_other_states_untouched() {
        let err = VerificationResult::Error(
            "tcp connect error: connection refused (os error 111) to 10.0.0.5:443".to_string(),
            ErrorCategory::Unreachable,
        );
        let dto = nip05_result_dto(&err);
        assert_eq!(dto.status, "error");
        assert_eq!(dto.detail.as_deref(), Some("unreachable"));
        assert!(!dto.detail.unwrap().contains("10.0.0.5"));

        assert_eq!(nip05_result_dto(&VerificationResult::Verified).detail, None);
        assert_eq!(nip05_result_dto(&VerificationResult::Mismatch).detail, None);
        assert_eq!(
            nip05_result_dto(&VerificationResult::NotApplicable).detail,
            None
        );
    }

    #[test]
    fn status_dto_reports_invalid_keys_with_null_fields() {
        let pk = Keys::generate().public_key();
        let report = StatusReport {
            state_path: PathBuf::from("/data/state.json"),
            lines: vec![
                StatusLine::Version(VersionStatus {
                    pubkey: pk,
                    d: "example.com".to_string(),
                    path: "/swing/agent/ab/example.com/1".to_string(),
                    cid: "bafy-ok".to_string(),
                    size: 10,
                    created_at: 1,
                    health: VersionHealth::Ok,
                }),
                StatusLine::InvalidKey {
                    key: "not-a-valid-key".to_string(),
                    cid: "bafy-bad".to_string(),
                },
            ],
            sites: vec![health::SiteSize {
                pubkey: pk,
                d: "example.com".to_string(),
                path: "/swing/agent/ab/example.com".to_string(),
                actual: Some(7),
            }],
            garbage: Garbage::default(),
            problems: 1,
        };

        let dto = status_dto(&report);

        assert_eq!(dto.versions.len(), 2);
        assert_eq!(dto.actual_bytes, Some(7));
        assert_eq!(dto.sites[0].actual, Some(7));
        let invalid = &dto.versions[1];
        assert_eq!(invalid.health, "invalid_key");
        assert_eq!(invalid.detail.as_deref(), Some("not-a-valid-key"));
        assert_eq!(invalid.cid.as_deref(), Some("bafy-bad"));
        assert!(invalid.pubkey.is_none());
        assert!(invalid.npub.is_none());
        assert!(invalid.d.is_none());
        assert!(invalid.path.is_none());
        assert!(invalid.size.is_none());
        assert!(invalid.created_at.is_none());
    }

    #[test]
    fn relay_errors_are_sanitized_and_bounded() {
        let result = nostr::RelaySendResult {
            relay: "wss://r".into(),
            ok: false,
            error: Some(format!("bad\x1b[31m{}", "x".repeat(2000))),
        };
        let error = RelayResultDto::from(&result).error.unwrap();
        assert!(!error.contains('\x1b'));
        assert_eq!(error.chars().count(), MAX_RELAY_ERROR_CHARS + 1);
    }
}
