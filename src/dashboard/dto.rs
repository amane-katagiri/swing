use nostr_sdk::prelude::PublicKey;
use serde::{Deserialize, Serialize};

use crate::config;
use crate::health;
use crate::mirror;
use crate::nostr;
use crate::replicas;
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

impl From<&nostr::RelaySendResult> for RelayResultDto {
    fn from(r: &nostr::RelaySendResult) -> Self {
        Self {
            relay: r.relay.clone(),
            ok: r.ok,
            error: r.error.clone(),
        }
    }
}

fn relay_results_dto(results: &[nostr::RelaySendResult]) -> Vec<RelayResultDto> {
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
pub struct LoginCodeDto {
    pub code: String,
    pub expires_in: u64,
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
    pub created_at: u64,
    pub title: Option<String>,
    pub message: Option<String>,
    pub nip05: Option<String>,
    pub replicas: Option<usize>,
    pub unverified_replicas: Option<usize>,
    pub stored: bool,
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
        created_at: site.created_at,
        title: site.title.clone(),
        message: site.message.clone(),
        nip05: site.nip05.clone(),
        replicas: site.replicas.map(|c| c.trusted),
        unverified_replicas: site.replicas.map(|c| c.unverified),
        stored: site.stored,
        gateway_url: gateway_url(gateway, &site.cid, site.stored),
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
pub struct PublishResultDto {
    pub site: String,
    pub url: Option<String>,
    pub title: Option<String>,
    pub message: Option<String>,
    pub nip05: Nip05ResultDto,
    pub cid: String,
    pub size: u64,
    pub created_at: u64,
    pub mfs_path: String,
    pub relays: Vec<RelayResultDto>,
    pub pruned: Vec<String>,
    pub prune_error: Option<String>,
    pub gateway_url: Option<String>,
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

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ConfigValue {
    Str(String),
    Bool(bool),
    Num(u64),
    List(Vec<String>),
}

#[derive(Debug, Serialize)]
pub struct DescriptionDto {
    pub en: &'static str,
    pub ja: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ConfigItemDto {
    pub key: String,
    pub env: String,
    pub value: ConfigValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    pub source: &'static str,
    pub editable: bool,
    pub kind: crate::settings::Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<crate::settings::RawValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<&'static str>>,
    pub description: DescriptionDto,
}

fn source_str(source: config::Source) -> &'static str {
    match source {
        config::Source::Env => "env",
        config::Source::File => "file",
        config::Source::Default => "default",
    }
}

fn config_item_dto(
    config: &config::Config,
    desc: &'static crate::settings::Setting,
) -> ConfigItemDto {
    let (value, display) = config_value(config, desc);
    let source = config
        .source_of(desc.key)
        .unwrap_or(config::Source::Default);
    let options = if desc.kind == crate::settings::Kind::Nip05 {
        Some(config::NIP05_MODE_NAMES.to_vec())
    } else {
        None
    };
    ConfigItemDto {
        key: desc.field.to_string(),
        env: desc.env.to_string(),
        value,
        display,
        source: source_str(source),
        editable: crate::settings::is_editable(config, desc.key),
        kind: desc.kind,
        raw: crate::settings::raw_value(config, desc.key),
        options,
        description: DescriptionDto {
            en: desc.description.en,
            ja: desc.description.ja,
        },
    }
}

fn opt_path_str(p: &Option<std::path::PathBuf>) -> String {
    p.as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

/// The current value (and, for size/duration keys, a human-readable
/// `display` string) for one catalog key. One arm per `settings::SETTINGS`
/// entry; `settings::SETTINGS.len()` keys must all be handled here (enforced
/// indirectly by the `_ => unreachable!` arm and the dashboard config tests).
fn config_value(
    config: &config::Config,
    desc: &crate::settings::Setting,
) -> (ConfigValue, Option<String>) {
    match desc.key {
        "nostr.secret_key" => (
            ConfigValue::Str(
                if config.nostr.secret_key.is_some() {
                    "(set, hidden)"
                } else {
                    "(not set)"
                }
                .to_string(),
            ),
            None,
        ),
        "nostr.relays" => (ConfigValue::List(config.nostr.relays.clone()), None),
        "nostr.mirror_set" => (ConfigValue::Str(config.nostr.mirror_set.clone()), None),
        "nostr.site_event_kind" => (ConfigValue::Num(config.nostr.site_event_kind as u64), None),
        "nostr.replica_event_kind" => (
            ConfigValue::Num(config.nostr.replica_event_kind as u64),
            None,
        ),
        "ipfs.api" => (ConfigValue::Str(ipfs_api_str(&config.ipfs.api)), None),
        "ipfs.mfs_root" => (ConfigValue::Str(config.ipfs.mfs_root.clone()), None),
        "policy.max_total_storage" => (
            ConfigValue::Num(config.policy.max_total_storage),
            Some(format_bytes(config.policy.max_total_storage)),
        ),
        "policy.max_per_site" => (
            ConfigValue::Num(config.policy.max_per_site),
            Some(format_bytes(config.policy.max_per_site)),
        ),
        "policy.max_per_account" => (
            ConfigValue::Num(config.policy.max_per_account),
            Some(format_bytes(config.policy.max_per_account)),
        ),
        "policy.max_sites_per_account" => (
            ConfigValue::Num(config.policy.max_sites_per_account as u64),
            None,
        ),
        "policy.max_update_size" => (
            ConfigValue::Num(config.policy.max_update_size),
            Some(format_bytes(config.policy.max_update_size)),
        ),
        "policy.keep_versions" => (ConfigValue::Num(config.policy.keep_versions as u64), None),
        "policy.keep_days" => (ConfigValue::Num(config.policy.keep_days), None),
        "policy.min_update_interval" => (
            ConfigValue::Num(config.policy.min_update_interval),
            Some(format_duration_secs(config.policy.min_update_interval)),
        ),
        "policy.remove_on_unfollow" => (ConfigValue::Bool(config.policy.remove_on_unfollow), None),
        "policy.nip05" => (
            ConfigValue::Str(nip05_mode_str(config.policy.nip05).to_string()),
            None,
        ),
        "policy.nip05_cache_ttl" => (
            ConfigValue::Num(config.policy.nip05_cache_ttl),
            Some(format_duration_secs(config.policy.nip05_cache_ttl)),
        ),
        "agent.state_dir" => (
            ConfigValue::Str(config.agent.state_dir.display().to_string()),
            None,
        ),
        "agent.poll_interval" => (
            ConfigValue::Num(config.agent.poll_interval.as_secs()),
            Some(format_duration_secs(config.agent.poll_interval.as_secs())),
        ),
        "agent.fetch_timeout" => (
            ConfigValue::Num(config.agent.fetch_timeout.as_secs()),
            Some(format_duration_secs(config.agent.fetch_timeout.as_secs())),
        ),
        "agent.fetch_idle_timeout" => (
            ConfigValue::Num(config.agent.fetch_idle_timeout.as_secs()),
            Some(format_duration_secs(
                config.agent.fetch_idle_timeout.as_secs(),
            )),
        ),
        "agent.concurrency" => (ConfigValue::Num(config.agent.concurrency as u64), None),
        "agent.report_ttl" => (
            ConfigValue::Num(config.agent.report_ttl.as_secs()),
            Some(format_duration_secs(config.agent.report_ttl.as_secs())),
        ),
        "publish.nip05" => (
            ConfigValue::Str(nip05_mode_str(config.publish.nip05).to_string()),
            None,
        ),
        "publish.keep_versions" => (ConfigValue::Num(config.publish.keep_versions as u64), None),
        "dashboard.listen" => (ConfigValue::Str(config.dashboard.listen.to_string()), None),
        "dashboard.ui" => (ConfigValue::Bool(config.dashboard.ui), None),
        "dashboard.allowed_hosts" => (
            ConfigValue::List(config.dashboard.allowed_hosts.clone()),
            None,
        ),
        "dashboard.public_url" => (
            ConfigValue::Str(config.dashboard.public_url.clone().unwrap_or_default()),
            None,
        ),
        "dashboard.gateway" => (
            ConfigValue::Str(config.dashboard.gateway.clone().unwrap_or_default()),
            None,
        ),
        "dashboard.custom_css" => (
            ConfigValue::Str(opt_path_str(&config.dashboard.custom_css)),
            None,
        ),
        "dashboard.desktop_page" => (
            ConfigValue::Str(opt_path_str(&config.dashboard.desktop_page)),
            None,
        ),
        "dashboard.desktop_page_css" => (
            ConfigValue::Str(opt_path_str(&config.dashboard.desktop_page_css)),
            None,
        ),
        "dashboard.desktop_banner" => (
            ConfigValue::Str(opt_path_str(&config.dashboard.desktop_banner)),
            None,
        ),
        "dashboard.max_upload" => (
            ConfigValue::Num(config.dashboard.max_upload),
            Some(format_bytes(config.dashboard.max_upload)),
        ),
        "kubo.managed" => (ConfigValue::Bool(config.kubo.managed), None),
        "kubo.binary" => (ConfigValue::Str(opt_path_str(&config.kubo.binary)), None),
        "kubo.repo" => (
            ConfigValue::Str(config.kubo.repo.display().to_string()),
            None,
        ),
        "kubo.storage_max" => (
            ConfigValue::Num(config.kubo.storage_max),
            Some(format_bytes(config.kubo.storage_max)),
        ),
        "kubo.provide_strategy" => (ConfigValue::Str(config.kubo.provide_strategy.clone()), None),
        "kubo.gateway_listen" => (
            ConfigValue::Str(config.kubo.gateway_listen.to_string()),
            None,
        ),
        "kubo.swarm_port" => (
            ConfigValue::Str(
                config
                    .kubo
                    .swarm_port
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            ),
            None,
        ),
        "gateway.listen" => (ConfigValue::Str(listen_str(&config.gateway.listen)), None),
        "gateway.hosts" => (ConfigValue::List(config.gateway.hosts.clone()), None),
        "gateway.upstream" => (ConfigValue::Str(config.gateway.upstream.clone()), None),
        other => unreachable!("settings::SETTINGS has no dto mapping for {other}"),
    }
}

const BYTE_UNITS: [(u64, &str); 4] = [
    (1u64 << 40, "TB"),
    (1u64 << 30, "GB"),
    (1u64 << 20, "MB"),
    (1u64 << 10, "KB"),
];

pub fn format_bytes(n: u64) -> String {
    for &(factor, unit) in &BYTE_UNITS {
        if n < factor {
            continue;
        }
        if n.is_multiple_of(factor) {
            return format!("{} {unit}", n / factor);
        }
        if (u128::from(n) * 10).is_multiple_of(u128::from(factor)) {
            return format!("{:.1} {unit}", n as f64 / factor as f64);
        }
    }
    format!("{n} B")
}

const DURATION_UNITS: [(u64, &str); 3] = [(86_400, "d"), (3_600, "h"), (60, "m")];

pub fn format_duration_secs(secs: u64) -> String {
    for &(factor, unit) in &DURATION_UNITS {
        if secs != 0 && secs.is_multiple_of(factor) {
            return format!("{}{unit}", secs / factor);
        }
    }
    format!("{secs}s")
}

#[derive(Debug, Serialize)]
pub struct ConfigSectionDto {
    pub name: String,
    pub items: Vec<ConfigItemDto>,
}

fn nip05_mode_str(mode: config::Nip05Mode) -> &'static str {
    match mode {
        config::Nip05Mode::Off => "off",
        config::Nip05Mode::Warn => "warn",
        config::Nip05Mode::Require => "require",
    }
}

fn listen_str(listen: &config::Listen) -> String {
    match listen {
        config::Listen::Off => "off".to_string(),
        config::Listen::Addr(addr) => addr.to_string(),
    }
}

fn ipfs_api_str(api: &config::IpfsApi) -> String {
    match api {
        config::IpfsApi::Url(url) => url.clone(),
        config::IpfsApi::Managed => "managed".to_string(),
    }
}

fn is_config_writable(config: &config::Config) -> bool {
    if config.config_exists {
        return std::fs::OpenOptions::new()
            .append(true)
            .open(&config.config_path)
            .is_ok();
    }
    let Some(parent) = config
        .config_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    else {
        return true;
    };
    match std::fs::metadata(parent) {
        Ok(meta) => !meta.permissions().readonly(),
        Err(_) => false,
    }
}

#[derive(Debug, Serialize)]
pub struct ConfigDto {
    pub config_path: Option<String>,
    pub config_exists: bool,
    pub writable: bool,
    pub restart_required: bool,
    pub sections: Vec<ConfigSectionDto>,
}

pub fn config_dto(config: &config::Config, restart_required: bool) -> ConfigDto {
    let sections = crate::settings::SECTION_ORDER
        .iter()
        .map(|&name| {
            let items = crate::settings::SETTINGS
                .iter()
                .filter(|s| s.section == name)
                .map(|s| config_item_dto(config, s))
                .collect();
            ConfigSectionDto {
                name: name.to_string(),
                items,
            }
        })
        .collect();

    ConfigDto {
        config_path: Some(config.config_path.display().to_string()),
        config_exists: config.config_exists,
        writable: is_config_writable(config),
        restart_required,
        sections,
    }
}

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
    fn format_bytes_prefers_the_largest_exact_unit() {
        assert_eq!(format_bytes(107_374_182_400), "100 GB");
        assert_eq!(format_bytes(512 * (1u64 << 20)), "512 MB");
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(300), "300 B");
    }

    #[test]
    fn format_duration_secs_prefers_the_largest_exact_unit() {
        assert_eq!(format_duration_secs(300), "5m");
        assert_eq!(format_duration_secs(86_400), "1d");
        assert_eq!(format_duration_secs(7_200), "2h");
        assert_eq!(format_duration_secs(600), "10m");
        assert_eq!(format_duration_secs(30), "30s");
        assert_eq!(format_duration_secs(0), "0s");
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

    fn test_config_at(path: PathBuf, exists: bool) -> config::Config {
        let mut cfg = config::build_config_from_str(
            "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n",
            |_| None,
        )
        .unwrap();
        cfg.config_path = path;
        cfg.config_exists = exists;
        cfg
    }

    #[test]
    fn missing_config_with_a_missing_parent_directory_is_not_writable() {
        let cfg = test_config_at(
            PathBuf::from("/nonexistent-swing-test-dir-xyz/swing.toml"),
            false,
        );
        assert!(!is_config_writable(&cfg));
    }

    #[test]
    fn missing_config_in_a_writable_directory_is_writable() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = test_config_at(dir.path().join("swing.toml"), false);
        assert!(is_config_writable(&cfg));
    }

    #[test]
    fn existing_writable_config_is_writable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swing.toml");
        std::fs::write(&path, "").unwrap();
        let cfg = test_config_at(path, true);
        assert!(is_config_writable(&cfg));
    }

    #[cfg(unix)]
    #[test]
    fn existing_readonly_config_is_not_writable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swing.toml");
        std::fs::write(&path, "").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
        let cfg = test_config_at(path.clone(), true);
        if std::fs::OpenOptions::new().append(true).open(&path).is_ok() {
            // Running as root (or on a filesystem that ignores permission bits):
            // the read-only bit doesn't block writes, so there's nothing to assert.
            return;
        }
        assert!(!is_config_writable(&cfg));
    }
}
