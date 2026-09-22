use nostr_sdk::prelude::PublicKey;
use serde::Serialize;

use crate::config;
use crate::health;
use crate::mirror;
use crate::nostr;
use crate::replicas;
use crate::webring;

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
pub struct OverviewDto {
    pub version: String,
    pub pubkey: String,
    pub npub: String,
    pub relays: Vec<String>,
    pub mirror_set: String,
    pub gateway: Option<String>,
    pub started_at: u64,
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
        replicas: site.replicas,
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

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
pub struct GarbageDto {
    pub path: String,
    pub list_failed: bool,
}

#[derive(Debug, Serialize)]
pub struct SiteSizeDto {
    pub pubkey: String,
    pub npub: String,
    pub d: String,
    pub path: String,
    pub actual: Option<u64>,
}

#[derive(Debug, Serialize)]
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
        })
        .collect();
    garbage.extend(report.garbage.unlisted.iter().map(|(p, _)| GarbageDto {
        path: p.clone(),
        list_failed: true,
    }));
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

#[derive(Debug, Serialize)]
pub struct MirrorChangeDto {
    pub changed: Vec<PubkeyDto>,
    pub unchanged: Vec<PubkeyDto>,
    pub published: bool,
    pub relays: Vec<RelayResultDto>,
    pub members: Vec<PubkeyDto>,
}

pub fn mirror_change_dto(change: &mirror::MirrorChange) -> MirrorChangeDto {
    MirrorChangeDto {
        changed: change.changed.iter().map(PubkeyDto::new).collect(),
        unchanged: change.unchanged.iter().map(PubkeyDto::new).collect(),
        published: change.published,
        relays: relay_results_dto(&change.relay_results),
        members: change.set.pubkeys().iter().map(PubkeyDto::new).collect(),
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
pub struct WebringDto {
    pub depth: usize,
    pub nodes: Vec<WebringNodeDto>,
    pub edges: Vec<WebringEdgeDto>,
    pub beyond: usize,
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
        text: webring::render_text(&view.graph, &view.names, &view.mirror_set, view.depth),
        dot: webring::render_dot(&view.graph, &view.names),
        mermaid: webring::render_mermaid(&view.graph, &view.names),
    }
}

#[derive(Debug, Serialize)]
pub struct ReporterDto {
    pub pubkey: String,
    pub npub: String,
    pub latest: bool,
    pub is_author: bool,
    pub following: bool,
}

#[derive(Debug, Serialize)]
pub struct SiteReplicasDto {
    pub d: String,
    pub cid: String,
    pub replicas: usize,
    pub reports: usize,
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
                        reports: s.reports,
                        reporters: s
                            .reporters
                            .iter()
                            .map(|r| ReporterDto {
                                pubkey: r.pubkey.to_hex(),
                                npub: mirror::npub(&r.pubkey),
                                latest: r.latest,
                                is_author: r.is_author,
                                following: r.following,
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
pub struct ConfigItemDto {
    pub key: Option<String>,
    pub env: Option<String>,
    pub value: ConfigValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
}

impl ConfigItemDto {
    fn new(key: &str, env: &str, value: ConfigValue) -> Self {
        Self {
            key: Some(key.to_string()),
            env: Some(env.to_string()),
            value,
            display: None,
        }
    }

    fn env_only(env: &str, value: ConfigValue) -> Self {
        Self {
            key: None,
            env: Some(env.to_string()),
            value,
            display: None,
        }
    }

    fn with_display(mut self, display: String) -> Self {
        self.display = Some(display);
        self
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

fn dashboard_listen_str(listen: &config::DashboardListen) -> String {
    match listen {
        config::DashboardListen::Off => "off".to_string(),
        config::DashboardListen::Addr(addr) => addr.to_string(),
    }
}

#[derive(Debug, Serialize)]
pub struct ConfigDto {
    pub config_path: Option<String>,
    pub sections: Vec<ConfigSectionDto>,
}

pub fn config_dto(config: &config::Config) -> ConfigDto {
    let nostr = ConfigSectionDto {
        name: "nostr".to_string(),
        items: vec![
            ConfigItemDto::new(
                "secret_key",
                "SWING_NOSTR_SECRET_KEY",
                ConfigValue::Str("(set, hidden)".to_string()),
            ),
            ConfigItemDto::new(
                "relays",
                "SWING_NOSTR_RELAYS",
                ConfigValue::List(config.nostr.relays.clone()),
            ),
            ConfigItemDto::new(
                "mirror_set",
                "SWING_MIRROR_SET",
                ConfigValue::Str(config.nostr.mirror_set.clone()),
            ),
            ConfigItemDto::new(
                "site_event_kind",
                "SWING_SITE_EVENT_KIND",
                ConfigValue::Num(config.nostr.site_event_kind as u64),
            ),
            ConfigItemDto::new(
                "replica_event_kind",
                "SWING_REPLICA_EVENT_KIND",
                ConfigValue::Num(config.nostr.replica_event_kind as u64),
            ),
        ],
    };

    let ipfs = ConfigSectionDto {
        name: "ipfs".to_string(),
        items: vec![
            ConfigItemDto::new(
                "api",
                "SWING_IPFS_API",
                ConfigValue::Str(config.ipfs.api.clone()),
            ),
            ConfigItemDto::new(
                "mfs_root",
                "SWING_MFS_ROOT",
                ConfigValue::Str(config.ipfs.mfs_root.clone()),
            ),
        ],
    };

    let policy = ConfigSectionDto {
        name: "policy".to_string(),
        items: vec![
            ConfigItemDto::new(
                "max_total_storage",
                "SWING_MAX_TOTAL_STORAGE",
                ConfigValue::Num(config.policy.max_total_storage),
            )
            .with_display(format_bytes(config.policy.max_total_storage)),
            ConfigItemDto::new(
                "max_per_site",
                "SWING_MAX_PER_SITE",
                ConfigValue::Num(config.policy.max_per_site),
            )
            .with_display(format_bytes(config.policy.max_per_site)),
            ConfigItemDto::new(
                "max_per_account",
                "SWING_MAX_PER_ACCOUNT",
                ConfigValue::Num(config.policy.max_per_account),
            )
            .with_display(format_bytes(config.policy.max_per_account)),
            ConfigItemDto::new(
                "max_sites_per_account",
                "SWING_MAX_SITES_PER_ACCOUNT",
                ConfigValue::Num(config.policy.max_sites_per_account as u64),
            ),
            ConfigItemDto::new(
                "max_update_size",
                "SWING_MAX_UPDATE_SIZE",
                ConfigValue::Num(config.policy.max_update_size),
            )
            .with_display(format_bytes(config.policy.max_update_size)),
            ConfigItemDto::new(
                "keep_versions",
                "SWING_KEEP_VERSIONS",
                ConfigValue::Num(config.policy.keep_versions as u64),
            ),
            ConfigItemDto::new(
                "keep_days",
                "SWING_KEEP_DAYS",
                ConfigValue::Num(config.policy.keep_days),
            ),
            ConfigItemDto::new(
                "min_update_interval",
                "SWING_MIN_UPDATE_INTERVAL",
                ConfigValue::Num(config.policy.min_update_interval),
            )
            .with_display(format_duration_secs(config.policy.min_update_interval)),
            ConfigItemDto::new(
                "remove_on_unfollow",
                "SWING_REMOVE_ON_UNFOLLOW",
                ConfigValue::Bool(config.policy.remove_on_unfollow),
            ),
            ConfigItemDto::new(
                "nip05",
                "SWING_NIP05",
                ConfigValue::Str(nip05_mode_str(config.policy.nip05).to_string()),
            ),
            ConfigItemDto::new(
                "nip05_cache_ttl",
                "SWING_NIP05_CACHE_TTL",
                ConfigValue::Num(config.policy.nip05_cache_ttl),
            )
            .with_display(format_duration_secs(config.policy.nip05_cache_ttl)),
        ],
    };

    let agent = ConfigSectionDto {
        name: "agent".to_string(),
        items: vec![
            ConfigItemDto::new(
                "state_dir",
                "SWING_STATE_DIR",
                ConfigValue::Str(config.agent.state_dir.display().to_string()),
            ),
            ConfigItemDto::new(
                "poll_interval",
                "SWING_POLL_INTERVAL",
                ConfigValue::Num(config.agent.poll_interval.as_secs()),
            )
            .with_display(format_duration_secs(config.agent.poll_interval.as_secs())),
            ConfigItemDto::new(
                "concurrency",
                "SWING_CONCURRENCY",
                ConfigValue::Num(config.agent.concurrency as u64),
            ),
            ConfigItemDto::new(
                "report_ttl",
                "SWING_REPORT_TTL",
                ConfigValue::Num(config.agent.report_ttl.as_secs()),
            )
            .with_display(format_duration_secs(config.agent.report_ttl.as_secs())),
            ConfigItemDto::env_only(
                "SWING_FETCH_TIMEOUT",
                ConfigValue::Num(config.agent.fetch_timeout.as_secs()),
            )
            .with_display(format_duration_secs(config.agent.fetch_timeout.as_secs())),
            ConfigItemDto::env_only(
                "SWING_FETCH_IDLE_TIMEOUT",
                ConfigValue::Num(config.agent.fetch_idle_timeout.as_secs()),
            )
            .with_display(format_duration_secs(
                config.agent.fetch_idle_timeout.as_secs(),
            )),
        ],
    };

    let publish = ConfigSectionDto {
        name: "publish".to_string(),
        items: vec![
            ConfigItemDto::new(
                "nip05",
                "SWING_PUBLISH_NIP05",
                ConfigValue::Str(nip05_mode_str(config.publish.nip05).to_string()),
            ),
            ConfigItemDto::new(
                "keep_versions",
                "SWING_PUBLISH_KEEP_VERSIONS",
                ConfigValue::Num(config.publish.keep_versions as u64),
            ),
        ],
    };

    let dashboard = ConfigSectionDto {
        name: "dashboard".to_string(),
        items: vec![
            ConfigItemDto::new(
                "listen",
                "SWING_DASHBOARD_LISTEN",
                ConfigValue::Str(dashboard_listen_str(&config.dashboard.listen)),
            ),
            ConfigItemDto::new(
                "allowed_hosts",
                "SWING_DASHBOARD_ALLOWED_HOSTS",
                ConfigValue::List(config.dashboard.allowed_hosts.clone()),
            ),
            ConfigItemDto::new(
                "gateway",
                "SWING_DASHBOARD_GATEWAY",
                ConfigValue::Str(config.dashboard.gateway.clone().unwrap_or_default()),
            ),
            ConfigItemDto::new(
                "custom_css",
                "SWING_DASHBOARD_CUSTOM_CSS",
                ConfigValue::Str(
                    config
                        .dashboard
                        .custom_css
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                ),
            ),
            ConfigItemDto::new(
                "desktop_page",
                "SWING_DASHBOARD_DESKTOP_PAGE",
                ConfigValue::Str(
                    config
                        .dashboard
                        .desktop_page
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                ),
            ),
            ConfigItemDto::new(
                "desktop_page_css",
                "SWING_DASHBOARD_DESKTOP_PAGE_CSS",
                ConfigValue::Str(
                    config
                        .dashboard
                        .desktop_page_css
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                ),
            ),
            ConfigItemDto::new(
                "desktop_banner",
                "SWING_DASHBOARD_DESKTOP_BANNER",
                ConfigValue::Str(
                    config
                        .dashboard
                        .desktop_banner
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                ),
            ),
            ConfigItemDto::new(
                "max_upload",
                "SWING_DASHBOARD_MAX_UPLOAD",
                ConfigValue::Num(config.dashboard.max_upload),
            )
            .with_display(format_bytes(config.dashboard.max_upload)),
        ],
    };

    ConfigDto {
        config_path: config.config_path.as_ref().map(|p| p.display().to_string()),
        sections: vec![nostr, ipfs, policy, agent, publish, dashboard],
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
}
