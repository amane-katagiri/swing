use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use nostr_sdk::prelude::Keys;
use serde::{Deserialize, Serialize};
use toml_edit::{Array, DocumentMut, Item, Table, Value};

use crate::config::{self, Config, Source};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Size,
    Duration,
    Bool,
    Integer,
    String,
    List,
    Nip05,
    Path,
    SocketAddr,
    Port,
    Url,
    Secret,
    Listen,
}

#[derive(Debug, Clone, Copy)]
pub struct Text {
    pub en: &'static str,
    pub ja: &'static str,
}

/// How a setting's default appears in the generated `swing.example.toml`.
#[derive(Debug, Clone, Copy)]
pub enum Example {
    /// An active `field = value` line.
    Value(&'static str),
    /// A commented-out `#field = value` line (optional setting).
    Commented(&'static str),
    /// A commented-out `#field = value` line whose real default depends on
    /// another setting (the description explains the derivation).
    Derived(&'static str),
}

impl Example {
    fn literal(self) -> &'static str {
        match self {
            Example::Value(v) | Example::Commented(v) | Example::Derived(v) => v,
        }
    }

    fn is_active(self) -> bool {
        matches!(self, Example::Value(_))
    }
}

pub struct Setting {
    pub key: &'static str,
    pub section: &'static str,
    pub field: &'static str,
    pub env: &'static str,
    pub kind: Kind,
    pub example: Example,
    pub editable: bool,
    pub description: Text,
}

pub const SECTION_ORDER: [&str; 8] = [
    "nostr",
    "ipfs",
    "policy",
    "agent",
    "publish",
    "dashboard",
    "kubo",
    "gateway",
];

pub const SETTINGS: &[Setting] = &[
    Setting {
        key: "nostr.secret_key",
        section: "nostr",
        field: "secret_key",
        env: "SWING_NOSTR_SECRET_KEY",
        kind: Kind::Secret,
        example: Example::Commented("\"nsec1...\""),
        editable: false,
        description: Text {
            en: "Signing secret key (nsec or hex). Optional: without it, swing up starts in setup mode and a key can be generated and saved from the dashboard.",
            ja: "署名用の秘密鍵（nsec または hex）。省略可。無いと swing up はセットアップモードで起動し、ダッシュボードから生成・保存できる",
        },
    },
    Setting {
        key: "nostr.relays",
        section: "nostr",
        field: "relays",
        env: "SWING_NOSTR_RELAYS",
        kind: Kind::List,
        example: Example::Value(
            "[\"wss://relay.damus.io\", \"wss://nos.lol\", \"wss://relay.primal.net\", \"wss://yabu.me\", \"wss://relay-jp.nostr.wirednet.jp\"]",
        ),
        editable: true,
        description: Text {
            en: "Nostr relays to connect to (comma-separated as an env var).",
            ja: "接続する Nostr relay（カンマ区切り）",
        },
    },
    Setting {
        key: "nostr.mirror_set",
        section: "nostr",
        field: "mirror_set",
        env: "SWING_MIRROR_SET",
        kind: Kind::String,
        example: Example::Value("\"swing\""),
        editable: true,
        description: Text {
            en: "The d tag of the Follow Set (kind 30000) that defines the mirror set.",
            ja: "Follow Set（kind 30000）の d タグ",
        },
    },
    Setting {
        key: "nostr.site_event_kind",
        section: "nostr",
        field: "site_event_kind",
        env: "SWING_SITE_EVENT_KIND",
        kind: Kind::Integer,
        example: Example::Value("35980"),
        editable: false,
        description: Text {
            en: "Event kind used for site announcements.",
            ja: "サイトイベントの kind",
        },
    },
    Setting {
        key: "nostr.replica_event_kind",
        section: "nostr",
        field: "replica_event_kind",
        env: "SWING_REPLICA_EVENT_KIND",
        kind: Kind::Integer,
        example: Example::Value("35981"),
        editable: false,
        description: Text {
            en: "Event kind used for replica reports.",
            ja: "レプリカ報告イベントの kind",
        },
    },
    Setting {
        key: "ipfs.api",
        section: "ipfs",
        field: "api",
        env: "SWING_IPFS_API",
        kind: Kind::Url,
        example: Example::Commented("\"http://127.0.0.1:5001\""),
        editable: false,
        description: Text {
            en: "Kubo RPC endpoint, used only when [kubo].managed = false; setting it while managed = true is an error.",
            ja: "Kubo RPC のエンドポイント。[kubo].managed = false のときだけ使う。managed = true で指定するとエラーになる",
        },
    },
    Setting {
        key: "ipfs.mfs_root",
        section: "ipfs",
        field: "mfs_root",
        env: "SWING_MFS_ROOT",
        kind: Kind::String,
        example: Example::Value("\"/swing\""),
        editable: false,
        description: Text {
            en: "MFS directory that SWING stores sites under.",
            ja: "SWING が使う MFS のディレクトリ",
        },
    },
    Setting {
        key: "policy.max_total_storage",
        section: "policy",
        field: "max_total_storage",
        env: "SWING_MAX_TOTAL_STORAGE",
        kind: Kind::Size,
        example: Example::Value("\"100GB\""),
        editable: true,
        description: Text {
            en: "Total storage cap across all saved sites.",
            ja: "保存する全サイト合計の容量上限",
        },
    },
    Setting {
        key: "policy.max_per_site",
        section: "policy",
        field: "max_per_site",
        env: "SWING_MAX_PER_SITE",
        kind: Kind::Size,
        example: Example::Value("\"10GB\""),
        editable: true,
        description: Text {
            en: "Per-site storage cap; the oldest versions are dropped once it is exceeded, and an update that alone exceeds it is not saved.",
            ja: "1 サイトあたりの容量上限。超えた分は古い版から削除される。新版だけで超える更新は保存されない",
        },
    },
    Setting {
        key: "policy.max_per_account",
        section: "policy",
        field: "max_per_account",
        env: "SWING_MAX_PER_ACCOUNT",
        kind: Kind::Size,
        example: Example::Value("\"20GB\""),
        editable: true,
        description: Text {
            en: "Per-account storage cap across all of its sites; updates that would exceed it are not saved.",
            ja: "1 アカウントが持つ全サイトの合計容量上限。超える更新は保存されない",
        },
    },
    Setting {
        key: "policy.max_sites_per_account",
        section: "policy",
        field: "max_sites_per_account",
        env: "SWING_MAX_SITES_PER_ACCOUNT",
        kind: Kind::Integer,
        example: Example::Value("10"),
        editable: true,
        description: Text {
            en: "Maximum number of sites saved per account; already-saved sites keep receiving updates.",
            ja: "1 アカウントあたりに保存するサイト数の上限。既に保存しているサイトの更新は続く",
        },
    },
    Setting {
        key: "policy.max_update_size",
        section: "policy",
        field: "max_update_size",
        env: "SWING_MAX_UPDATE_SIZE",
        kind: Kind::Size,
        example: Example::Value("\"2GB\""),
        editable: true,
        description: Text {
            en: "Size cap for a single update (one version); larger updates are not saved.",
            ja: "1 回の更新（1 バージョン）あたりのサイズ上限。超えると保存されない",
        },
    },
    Setting {
        key: "policy.keep_versions",
        section: "policy",
        field: "keep_versions",
        env: "SWING_KEEP_VERSIONS",
        kind: Kind::Integer,
        example: Example::Value("5"),
        editable: true,
        description: Text {
            en: "Number of old versions kept per site; the oldest are removed once it is exceeded.",
            ja: "サイトごとに保持する旧バージョンの数。超えた分は古い順に削除される",
        },
    },
    Setting {
        key: "policy.keep_days",
        section: "policy",
        field: "keep_days",
        env: "SWING_KEEP_DAYS",
        kind: Kind::Integer,
        example: Example::Value("365"),
        editable: true,
        description: Text {
            en: "Days a version is kept; versions older than this (other than the latest) are removed.",
            ja: "バージョンを保持する日数。最新版を除き、これより古い版は削除される",
        },
    },
    Setting {
        key: "policy.min_update_interval",
        section: "policy",
        field: "min_update_interval",
        env: "SWING_MIN_UPDATE_INTERVAL",
        kind: Kind::Duration,
        example: Example::Value("\"1h\""),
        editable: true,
        description: Text {
            en: "Minimum interval between ingesting updates for the same site.",
            ja: "同じサイトを取り込む最短間隔",
        },
    },
    Setting {
        key: "policy.remove_on_unfollow",
        section: "policy",
        field: "remove_on_unfollow",
        env: "SWING_REMOVE_ON_UNFOLLOW",
        kind: Kind::Bool,
        example: Example::Value("true"),
        editable: true,
        description: Text {
            en: "Whether to automatically stop storing a site once its account is unfollowed.",
            ja: "ミラー対象から外したときに、自動でそのサイトの保存をやめるかどうか",
        },
    },
    Setting {
        key: "policy.nip05",
        section: "policy",
        field: "nip05",
        env: "SWING_NIP05",
        kind: Kind::Nip05,
        example: Example::Value("\"warn\""),
        editable: true,
        description: Text {
            en: "NIP-05 verification mode applied before storing a site (off / warn / require).",
            ja: "保存前に行う NIP-05 検証のモード（off / warn / require）",
        },
    },
    Setting {
        key: "policy.nip05_cache_ttl",
        section: "policy",
        field: "nip05_cache_ttl",
        env: "SWING_NIP05_CACHE_TTL",
        kind: Kind::Duration,
        example: Example::Value("\"1d\""),
        editable: true,
        description: Text {
            en: "How long a NIP-05 verification result is cached before re-checking.",
            ja: "NIP-05 の検証結果を再利用する期間",
        },
    },
    Setting {
        key: "agent.state_dir",
        section: "agent",
        field: "state_dir",
        env: "SWING_STATE_DIR",
        kind: Kind::Path,
        example: Example::Value("\"./data\""),
        editable: false,
        description: Text {
            en: "Directory where agent state files are kept.",
            ja: "状態ファイルを置くディレクトリ",
        },
    },
    Setting {
        key: "agent.poll_interval",
        section: "agent",
        field: "poll_interval",
        env: "SWING_POLL_INTERVAL",
        kind: Kind::Duration,
        example: Example::Value("\"5m\""),
        editable: true,
        description: Text {
            en: "How often the agent re-fetches the Follow Set.",
            ja: "Follow Set の再取得間隔",
        },
    },
    Setting {
        key: "agent.fetch_timeout",
        section: "agent",
        field: "fetch_timeout",
        env: "SWING_FETCH_TIMEOUT",
        kind: Kind::Duration,
        example: Example::Value("\"15m\""),
        editable: false,
        description: Text {
            en: "Overall timeout for fetching a single site.",
            ja: "1 サイト分の取得全体のタイムアウト",
        },
    },
    Setting {
        key: "agent.fetch_idle_timeout",
        section: "agent",
        field: "fetch_idle_timeout",
        env: "SWING_FETCH_IDLE_TIMEOUT",
        kind: Kind::Duration,
        example: Example::Value("\"2m\""),
        editable: false,
        description: Text {
            en: "Maximum time to wait without receiving data during a fetch.",
            ja: "取得中にデータが届かないまま待つ上限",
        },
    },
    Setting {
        key: "agent.concurrency",
        section: "agent",
        field: "concurrency",
        env: "SWING_CONCURRENCY",
        kind: Kind::Integer,
        example: Example::Value("4"),
        editable: true,
        description: Text {
            en: "Number of sites fetched and stored concurrently.",
            ja: "同時に取得・保存するサイト数",
        },
    },
    Setting {
        key: "agent.report_ttl",
        section: "agent",
        field: "report_ttl",
        env: "SWING_REPORT_TTL",
        kind: Kind::Duration,
        example: Example::Value("\"3d\""),
        editable: true,
        description: Text {
            en: "How long a replica report stays valid; it is re-sent after half this time and must be more than twice poll_interval.",
            ja: "レプリカ報告の有効期間。半分の期間を過ぎたら新しい報告を出す。poll_interval の 2 倍より長くする必要がある",
        },
    },
    Setting {
        key: "publish.nip05",
        section: "publish",
        field: "nip05",
        env: "SWING_PUBLISH_NIP05",
        kind: Kind::Nip05,
        example: Example::Value("\"warn\""),
        editable: true,
        description: Text {
            en: "NIP-05 verification mode for swing publish (off / warn / require); the --nip05 CLI flag takes precedence.",
            ja: "swing publish の NIP-05 検証モード（off / warn / require、--nip05 が優先）",
        },
    },
    Setting {
        key: "publish.keep_versions",
        section: "publish",
        field: "keep_versions",
        env: "SWING_PUBLISH_KEEP_VERSIONS",
        kind: Kind::Integer,
        example: Example::Value("5"),
        editable: true,
        description: Text {
            en: "Number of versions swing publish keeps on its own node.",
            ja: "swing publish が自分のノードに残す版の数",
        },
    },
    Setting {
        key: "dashboard.listen",
        section: "dashboard",
        field: "listen",
        env: "SWING_DASHBOARD_LISTEN",
        kind: Kind::SocketAddr,
        example: Example::Value("\"127.0.0.1:8082\""),
        editable: false,
        description: Text {
            en: "Address the dashboard listens on for as long as swing up runs.",
            ja: "ダッシュボードの待ち受けアドレス（swing up が動いている間ずっと待ち受ける）",
        },
    },
    Setting {
        key: "dashboard.ui",
        section: "dashboard",
        field: "ui",
        env: "SWING_DASHBOARD_UI",
        kind: Kind::Bool,
        example: Example::Value("true"),
        editable: false,
        description: Text {
            en: "When false, stops serving the static Web UI and keeps only the /api/* control API.",
            ja: "false で静的な Web UI を配信せず、/api/* の制御 API だけ残す",
        },
    },
    Setting {
        key: "dashboard.allowed_hosts",
        section: "dashboard",
        field: "allowed_hosts",
        env: "SWING_DASHBOARD_ALLOWED_HOSTS",
        kind: Kind::List,
        example: Example::Value("[]"),
        editable: false,
        description: Text {
            en: "Extra hostnames (without port, comma-separated as an env var) accepted in the Host header.",
            ja: "Host ヘッダで追加で許可するホスト名（ポート抜き、カンマ区切り）",
        },
    },
    Setting {
        key: "dashboard.public_url",
        section: "dashboard",
        field: "public_url",
        env: "SWING_DASHBOARD_PUBLIC_URL",
        kind: Kind::String,
        example: Example::Commented("\"http://127.0.0.1:8082\""),
        editable: false,
        description: Text {
            en: "Base URL a browser uses to reach the dashboard, for the login link printed by swing dashboard open; unset uses the listen address.",
            ja: "ブラウザからダッシュボードを開く URL（swing dashboard open が出すログインリンクの頭に使う）。未設定なら待ち受けアドレス",
        },
    },
    Setting {
        key: "dashboard.gateway",
        section: "dashboard",
        field: "gateway",
        env: "SWING_DASHBOARD_GATEWAY",
        kind: Kind::String,
        example: Example::Value("\"http://localhost:8080\""),
        editable: true,
        description: Text {
            en: "IPFS gateway used for links to saved sites; an empty value hides the links.",
            ja: "保存済みサイトを開くリンクの IPFS Gateway。空文字にするとリンクを表示しない",
        },
    },
    Setting {
        key: "dashboard.custom_css",
        section: "dashboard",
        field: "custom_css",
        env: "SWING_DASHBOARD_CUSTOM_CSS",
        kind: Kind::Path,
        example: Example::Commented("\"/path/to/custom.css\""),
        editable: false,
        description: Text {
            en: "Path to an extra CSS file loaded by the dashboard.",
            ja: "ダッシュボードに読み込ませる追加 CSS ファイルのパス",
        },
    },
    Setting {
        key: "dashboard.desktop_page",
        section: "dashboard",
        field: "desktop_page",
        env: "SWING_DASHBOARD_DESKTOP_PAGE",
        kind: Kind::Path,
        example: Example::Commented("\"/path/to/links.html\""),
        editable: false,
        description: Text {
            en: "Path to the Desktop view's link page (HTML); falls back to the bundled page when unset.",
            ja: "Desktop 画面のリンク集ページ（HTML）のパス。未設定なら同梱のページ",
        },
    },
    Setting {
        key: "dashboard.desktop_page_css",
        section: "dashboard",
        field: "desktop_page_css",
        env: "SWING_DASHBOARD_DESKTOP_PAGE_CSS",
        kind: Kind::Path,
        example: Example::Commented("\"/path/to/links.css\""),
        editable: false,
        description: Text {
            en: "Path to the CSS for that link page; falls back to the bundled CSS when unset.",
            ja: "そのリンク集ページ専用の CSS ファイルのパス。未設定なら同梱の CSS",
        },
    },
    Setting {
        key: "dashboard.desktop_banner",
        section: "dashboard",
        field: "desktop_banner",
        env: "SWING_DASHBOARD_DESKTOP_BANNER",
        kind: Kind::Path,
        example: Example::Commented("\"/path/to/banner.gif\""),
        editable: false,
        description: Text {
            en: "Path to the link page's 88x31 banner image (png/gif/jpeg/webp/svg); falls back to the bundled GIF when unset.",
            ja: "リンク集ページの 88×31 バナー画像のパス（png/gif/jpeg/webp/svg）。未設定なら同梱の GIF",
        },
    },
    Setting {
        key: "dashboard.max_upload",
        section: "dashboard",
        field: "max_upload",
        env: "SWING_DASHBOARD_MAX_UPLOAD",
        kind: Kind::Size,
        example: Example::Value("\"2GB\""),
        editable: false,
        description: Text {
            en: "Body size limit for POST /api/publish/upload; 0 is an error.",
            ja: "POST /api/publish/upload のボディ上限。0 はエラー",
        },
    },
    Setting {
        key: "kubo.managed",
        section: "kubo",
        field: "managed",
        env: "SWING_KUBO_MANAGED",
        kind: Kind::Bool,
        example: Example::Value("true"),
        editable: false,
        description: Text {
            en: "When true, swing up runs Kubo as a child process; when false, it uses an external Kubo via [ipfs].api.",
            ja: "true なら swing up が Kubo を子プロセスとして動かす。false なら外部の Kubo（[ipfs].api）を使う",
        },
    },
    Setting {
        key: "kubo.binary",
        section: "kubo",
        field: "binary",
        env: "SWING_KUBO_BINARY",
        kind: Kind::Path,
        example: Example::Commented("\"/usr/local/bin/ipfs\""),
        editable: false,
        description: Text {
            en: "Path to the managed Kubo binary; defaults to ipfs next to the swing executable, falling back to PATH.",
            ja: "管理する Kubo の実行ファイルのパス。既定: swing 実行ファイルと同じディレクトリの ipfs(.exe)、無ければ PATH の ipfs",
        },
    },
    Setting {
        key: "kubo.repo",
        section: "kubo",
        field: "repo",
        env: "SWING_KUBO_REPO",
        kind: Kind::Path,
        example: Example::Derived("\"./data/kubo\""),
        editable: false,
        description: Text {
            en: "Repository (IPFS_PATH) for the managed Kubo; defaults to kubo under [agent].state_dir.",
            ja: "管理する Kubo のリポジトリ（IPFS_PATH）。既定: [agent].state_dir/kubo",
        },
    },
    Setting {
        key: "kubo.storage_max",
        section: "kubo",
        field: "storage_max",
        env: "SWING_KUBO_STORAGE_MAX",
        kind: Kind::Size,
        example: Example::Derived("\"100GB\""),
        editable: true,
        description: Text {
            en: "Kubo's Datastore.StorageMax; defaults to the same value as [policy].max_total_storage.",
            ja: "Kubo の Datastore.StorageMax。既定: [policy].max_total_storage と同じ値",
        },
    },
    Setting {
        key: "kubo.provide_strategy",
        section: "kubo",
        field: "provide_strategy",
        env: "SWING_KUBO_PROVIDE_STRATEGY",
        kind: Kind::String,
        example: Example::Value("\"pinned+mfs\""),
        editable: false,
        description: Text {
            en: "Kubo's Provide.Strategy.",
            ja: "Kubo の Provide.Strategy",
        },
    },
    Setting {
        key: "kubo.gateway_listen",
        section: "kubo",
        field: "gateway_listen",
        env: "SWING_KUBO_GATEWAY_LISTEN",
        kind: Kind::SocketAddr,
        example: Example::Value("\"127.0.0.1:8080\""),
        editable: false,
        description: Text {
            en: "Addresses.Gateway of the managed Kubo.",
            ja: "管理する Kubo の Addresses.Gateway",
        },
    },
    Setting {
        key: "kubo.swarm_port",
        section: "kubo",
        field: "swarm_port",
        env: "SWING_KUBO_SWARM_PORT",
        kind: Kind::Port,
        example: Example::Commented("4001"),
        editable: false,
        description: Text {
            en: "Swarm port of the managed Kubo; left untouched at Kubo's default when unset.",
            ja: "管理する Kubo の swarm ポート。未設定なら Kubo の既定のまま触らない",
        },
    },
    Setting {
        key: "gateway.listen",
        section: "gateway",
        field: "listen",
        env: "SWING_GATEWAY_LISTEN",
        kind: Kind::Listen,
        example: Example::Value("\"off\""),
        editable: false,
        description: Text {
            en: "Address the built-in DNSLink gateway listens on (e.g. \"127.0.0.1:8081\"); \"off\" disables it.",
            ja: "SWING 内蔵 DNSLink ゲートウェイの待ち受けアドレス（例 \"127.0.0.1:8081\"）。\"off\" で無効",
        },
    },
    Setting {
        key: "gateway.hosts",
        section: "gateway",
        field: "hosts",
        env: "SWING_GATEWAY_HOSTS",
        kind: Kind::List,
        example: Example::Value("[]"),
        editable: false,
        description: Text {
            en: "Hostnames served over DNSLink (comma-separated as an env var); required when listen is not off, and added to Kubo's Gateway.PublicGateways when managed.",
            ja: "DNSLink で配信するホスト名（カンマ区切り）。listen が off でないなら必須。managed なら Kubo の Gateway.PublicGateways にも入れる",
        },
    },
    Setting {
        key: "gateway.upstream",
        section: "gateway",
        field: "upstream",
        env: "SWING_GATEWAY_UPSTREAM",
        kind: Kind::Url,
        example: Example::Derived("\"http://127.0.0.1:8080\""),
        editable: false,
        description: Text {
            en: "Kubo gateway the built-in gateway proxies to; defaults to http://<[kubo].gateway_listen> when managed, otherwise http://127.0.0.1:8080.",
            ja: "プロキシ先の Kubo gateway。既定: managed なら http://<[kubo].gateway_listen>、そうでなければ http://127.0.0.1:8080",
        },
    },
];

pub fn find(key: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|s| s.key == key)
}

/// The env var name for a catalog key, e.g. `env_of("policy.max_total_storage")`
/// = `"SWING_MAX_TOTAL_STORAGE"`. `build_config` uses this instead of its own
/// literals so each env name exists in exactly one place (the catalog).
pub fn env_of(key: &str) -> &'static str {
    find(key)
        .unwrap_or_else(|| panic!("settings::env_of: no such catalog key: {key}"))
        .env
}

pub fn is_editable(config: &Config, key: &str) -> bool {
    find(key).is_some_and(|s| s.editable) && config.source_of(key) != Some(Source::Env)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum InputValue {
    Str(String),
    List(Vec<String>),
}

impl InputValue {
    fn as_str(&self, key: &str) -> Result<&str> {
        match self {
            InputValue::Str(s) => Ok(s.as_str()),
            InputValue::List(_) => bail!("{key} expects a single string value, not a list"),
        }
    }

    fn as_list(&self, key: &str) -> Result<&[String]> {
        match self {
            InputValue::List(v) => Ok(v.as_slice()),
            InputValue::Str(_) => bail!("{key} expects a list of strings"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum RawValue {
    Str(String),
    List(Vec<String>),
}

pub fn raw_value(config: &Config, key: &str) -> Option<RawValue> {
    let value = match key {
        "nostr.relays" => RawValue::List(config.nostr.relays.clone()),
        "nostr.mirror_set" => RawValue::Str(config.nostr.mirror_set.clone()),
        "policy.max_total_storage" => RawValue::Str(crate::dashboard::dto::format_bytes(
            config.policy.max_total_storage,
        )),
        "policy.max_per_site" => RawValue::Str(crate::dashboard::dto::format_bytes(
            config.policy.max_per_site,
        )),
        "policy.max_per_account" => RawValue::Str(crate::dashboard::dto::format_bytes(
            config.policy.max_per_account,
        )),
        "policy.max_update_size" => RawValue::Str(crate::dashboard::dto::format_bytes(
            config.policy.max_update_size,
        )),
        "policy.max_sites_per_account" => {
            RawValue::Str(config.policy.max_sites_per_account.to_string())
        }
        "policy.keep_versions" => RawValue::Str(config.policy.keep_versions.to_string()),
        "policy.keep_days" => RawValue::Str(config.policy.keep_days.to_string()),
        "policy.min_update_interval" => RawValue::Str(crate::dashboard::dto::format_duration_secs(
            config.policy.min_update_interval,
        )),
        "policy.nip05_cache_ttl" => RawValue::Str(crate::dashboard::dto::format_duration_secs(
            config.policy.nip05_cache_ttl,
        )),
        "agent.poll_interval" => RawValue::Str(crate::dashboard::dto::format_duration_secs(
            config.agent.poll_interval.as_secs(),
        )),
        "agent.report_ttl" => RawValue::Str(crate::dashboard::dto::format_duration_secs(
            config.agent.report_ttl.as_secs(),
        )),
        "policy.remove_on_unfollow" => RawValue::Str(config.policy.remove_on_unfollow.to_string()),
        "policy.nip05" => RawValue::Str(nip05_mode_name(config.policy.nip05).to_string()),
        "publish.nip05" => RawValue::Str(nip05_mode_name(config.publish.nip05).to_string()),
        "agent.concurrency" => RawValue::Str(config.agent.concurrency.to_string()),
        "publish.keep_versions" => RawValue::Str(config.publish.keep_versions.to_string()),
        "kubo.storage_max" => {
            RawValue::Str(crate::dashboard::dto::format_bytes(config.kubo.storage_max))
        }
        "dashboard.gateway" => RawValue::Str(config.dashboard.gateway.clone().unwrap_or_default()),
        _ => return None,
    };
    Some(value)
}

fn nip05_mode_name(mode: config::Nip05Mode) -> &'static str {
    match mode {
        config::Nip05Mode::Off => "off",
        config::Nip05Mode::Warn => "warn",
        config::Nip05Mode::Require => "require",
    }
}

const EXAMPLE_COMMENT_COL: usize = 37;

fn push_padded_comment(out: &mut String, assignment: &str, note: &str) {
    let len = assignment.chars().count();
    out.push_str(assignment);
    if len < EXAMPLE_COMMENT_COL {
        out.push_str(&" ".repeat(EXAMPLE_COMMENT_COL - len));
    } else {
        out.push('\n');
        out.push_str(&" ".repeat(EXAMPLE_COMMENT_COL));
    }
    out.push_str("# ");
    out.push_str(note);
    out.push('\n');
}

/// Prints `swing.example.toml`, regenerated from [`SETTINGS`]. Kept in sync by
/// a drift test comparing this against the checked-in file.
pub fn render_toml_example() -> String {
    let mut out = String::new();
    for section in SECTION_ORDER {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("[{section}]\n"));
        for s in SETTINGS.iter().filter(|s| s.section == section) {
            let literal = s.example.literal();
            let assignment = if s.example.is_active() {
                format!("{} = {}", s.field, literal)
            } else {
                format!("#{} = {}", s.field, literal)
            };
            let note = format!("{}: {}", s.env, s.description.ja);
            push_padded_comment(&mut out, &assignment, &note);
        }
    }
    out
}

fn strip_toml_string(literal: &str) -> String {
    literal.trim().trim_matches('"').to_string()
}

fn env_default_literal(kind: Kind, literal: &str) -> String {
    match kind {
        Kind::List => literal
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .map(|s| s.trim().trim_matches('"'))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(","),
        _ => strip_toml_string(literal),
    }
}

/// `.env` env vars that `compose.yaml` fixes directly for the `mirror` service
/// (a literal `environment:` value, not a `${VAR:-default}` substitution), so
/// setting them in `.env` has no effect there. This is docker-compose
/// wiring knowledge, not a per-setting fact, so it lives here rather than in
/// the catalog.
const ENV_NO_EFFECT_IN_COMPOSE: &[&str] = &[
    "ipfs.api",
    "agent.state_dir",
    "kubo.managed",
    "gateway.upstream",
];

const ENV_EXAMPLE_SECRET_KEY_HEADER: &str = "# nsec1... または hex 形式の秘密鍵。サイト公開専用の鍵を新しく作って使うことを推奨する。\n# 空のままでもコンテナは起動する（ダッシュボードだけが動くセットアップモードになり、ブラウザのセットアップ画面から鍵を生成・保存できる。docs/architecture/up.md 参照）\nSWING_NOSTR_SECRET_KEY=\n";

const ENV_EXAMPLE_COMPOSE_EPILOGUE: &str = "# --- Docker Compose 専用のホストバインド（上の SWING_* とは別物。config::env_var は読まない） ---\n# Kubo のゲートウェイをホストのどこに公開するか（ipfs コンテナ）\n#SWING_KUBO_GATEWAY_BIND=127.0.0.1:8080\n# swing 内蔵ゲートウェイをホストのどこに公開するか（mirror コンテナ）\n#SWING_GATEWAY_BIND=127.0.0.1:8081\n# ダッシュボードをホストのどこに公開するか（mirror コンテナ）\n#SWING_DASHBOARD_BIND=127.0.0.1:8082\n";

/// Prints `.env.example`, regenerated from [`SETTINGS`]. Every catalog env
/// var is listed, commented out, at its real default, so copying the file
/// changes nothing until a line is uncommented (an uncommented `.env` value
/// becomes `Source::Env` and locks that setting out of dashboard editing).
pub fn render_env_example() -> String {
    let mut out = String::new();
    out.push_str(ENV_EXAMPLE_SECRET_KEY_HEADER);
    for section in SECTION_ORDER {
        let mut settings = SETTINGS
            .iter()
            .filter(|s| s.section == section && s.key != "nostr.secret_key")
            .peekable();
        if settings.peek().is_none() {
            continue;
        }
        out.push_str(&format!("\n# [{section}]\n"));
        for s in settings {
            let default = env_default_literal(s.kind, s.example.literal());
            let note = if ENV_NO_EFFECT_IN_COMPOSE.contains(&s.key) {
                format!(
                    "{}（compose の mirror コンテナでは compose.yaml が固定で渡すため、ここに書いても効果が無い）",
                    s.description.ja
                )
            } else {
                s.description.ja.to_string()
            };
            out.push_str(&format!("# {note}\n"));
            out.push_str(&format!("#{}={default}\n", s.env));
        }
    }
    out.push('\n');
    out.push_str(ENV_EXAMPLE_COMPOSE_EPILOGUE);
    out
}

fn ensure_table<'a>(doc: &'a mut DocumentMut, section: &str) -> Result<&'a mut Table> {
    if doc.get(section).and_then(Item::as_table).is_none() {
        doc[section] = Item::Table(Table::new());
    }
    doc[section]
        .as_table_mut()
        .with_context(|| format!("[{section}] is not a table in the config file"))
}

fn set_item(table: &mut Table, desc: &Setting, value: &InputValue) -> Result<()> {
    match desc.kind {
        Kind::List => {
            let list = value.as_list(desc.key)?;
            if desc.key == "nostr.relays" && list.is_empty() {
                bail!("nostr.relays: at least one relay is required");
            }
            let mut arr = Array::new();
            for item in list {
                arr.push(item.as_str());
            }
            table.insert(desc.field, Item::Value(Value::Array(arr)));
        }
        Kind::Bool => {
            let s = value.as_str(desc.key)?;
            let parsed =
                config::parse_bool(s).with_context(|| format!("{}: invalid boolean", desc.key))?;
            table.insert(desc.field, Item::Value(Value::from(parsed)));
        }
        Kind::Integer => {
            let s = value.as_str(desc.key)?;
            let parsed: i64 = s
                .trim()
                .parse()
                .with_context(|| format!("{}: expected an integer", desc.key))?;
            table.insert(desc.field, Item::Value(Value::from(parsed)));
        }
        Kind::Size => {
            let s = value.as_str(desc.key)?;
            config::parse_size(s).with_context(|| format!("{}: invalid size", desc.key))?;
            table.insert(desc.field, Item::Value(Value::from(s.trim().to_string())));
        }
        Kind::Duration => {
            let s = value.as_str(desc.key)?;
            config::parse_duration_secs(s)
                .with_context(|| format!("{}: invalid duration", desc.key))?;
            table.insert(desc.field, Item::Value(Value::from(s.trim().to_string())));
        }
        Kind::Nip05 => {
            let s = value.as_str(desc.key)?;
            config::parse_nip05_mode(s)?;
            table.insert(desc.field, Item::Value(Value::from(s.trim().to_string())));
        }
        Kind::String => {
            let s = value.as_str(desc.key)?;
            table.insert(desc.field, Item::Value(Value::from(s.to_string())));
        }
        // Only the 20 editable kinds above ever reach set_item; the rest are catalog
        // entries for non-editable settings.
        other => bail!("{}: kind {other:?} is not editable", desc.key),
    }
    Ok(())
}

fn find_editable(key: &str) -> Option<&'static Setting> {
    find(key).filter(|s| s.editable)
}

fn apply_items(doc: &mut DocumentMut, items: &BTreeMap<String, InputValue>) -> Result<()> {
    for (key, value) in items {
        let Some(desc) = find_editable(key) else {
            bail!("unknown or non-editable key: {key}");
        };
        let table = ensure_table(doc, desc.section)?;
        set_item(table, desc, value)?;
    }
    Ok(())
}

fn check_not_env_sourced(current: &Config, items: &BTreeMap<String, InputValue>) -> Result<()> {
    for key in items.keys() {
        if find_editable(key).is_none() {
            bail!("unknown or non-editable key: {key}");
        }
        if current.source_of(key) == Some(Source::Env) {
            bail!("{key} is set via an environment variable and cannot be edited here");
        }
    }
    Ok(())
}

fn load_document(current: &Config) -> Result<DocumentMut> {
    let text = if current.config_exists {
        std::fs::read_to_string(&current.config_path)
            .with_context(|| format!("reading config file {}", current.config_path.display()))?
    } else {
        String::new()
    };
    text.parse::<DocumentMut>()
        .context("parsing existing config file")
}

#[cfg(unix)]
fn write_atomic(path: &Path, contents: &str, existed_before: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(dir) = dir {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating config directory {}", dir.display()))?;
    }
    let mode = if existed_before {
        std::fs::metadata(path)
            .map(|m| m.permissions().mode())
            .unwrap_or(0o600)
    } else {
        0o600
    };
    let tmp_path = path.with_extension(format!("toml.tmp-{}", std::process::id()));
    std::fs::write(&tmp_path, contents)
        .with_context(|| format!("writing {}", tmp_path.display()))?;
    std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("setting permissions on {}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, path)
        .with_context(|| format!("renaming {} to {}", tmp_path.display(), path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn write_atomic(path: &Path, contents: &str, _existed_before: bool) -> Result<()> {
    if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating config directory {}", dir.display()))?;
    }
    let tmp_path = path.with_extension(format!("toml.tmp-{}", std::process::id()));
    std::fs::write(&tmp_path, contents)
        .with_context(|| format!("writing {}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, path)
        .with_context(|| format!("renaming {} to {}", tmp_path.display(), path.display()))?;
    Ok(())
}

pub fn update(current: &Config, items: &BTreeMap<String, InputValue>) -> Result<Config> {
    check_not_env_sourced(current, items)?;
    let mut doc = load_document(current)?;
    apply_items(&mut doc, items)?;
    let rendered = doc.to_string();
    config::build_config_from_str(&rendered, config::env_var)
        .context("edited configuration is invalid")?;
    write_atomic(&current.config_path, &rendered, current.config_exists)?;
    Config::load(Some(&current.config_path))
}

pub fn setup_keys(secret_key_input: Option<&str>) -> Result<Keys> {
    match secret_key_input {
        Some(raw) if !raw.trim().is_empty() => {
            Keys::parse(raw.trim()).context("invalid secret key")
        }
        _ => Ok(Keys::generate()),
    }
}

pub fn setup(
    current: &Config,
    keys: Option<&Keys>,
    items: &BTreeMap<String, InputValue>,
) -> Result<Config> {
    if current.nostr.secret_key.is_some() {
        bail!("setup is only available before a Nostr key is configured");
    }
    check_not_env_sourced(current, items)?;

    let mut doc = load_document(current)?;
    apply_items(&mut doc, items)?;
    if let Some(keys) = keys {
        let nostr_table = ensure_table(&mut doc, "nostr")?;
        nostr_table.insert(
            "secret_key",
            Item::Value(Value::from(keys.secret_key().to_secret_hex())),
        );
    }
    let rendered = doc.to_string();
    config::build_config_from_str(&rendered, config::env_var)
        .context("edited configuration is invalid")?;
    write_atomic(&current.config_path, &rendered, current.config_exists)?;
    Config::load(Some(&current.config_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn base_config(dir: &Path) -> Config {
        let path = dir.join("swing.toml");
        std::fs::write(
            &path,
            "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n",
        )
        .unwrap();
        Config::load(Some(&path)).unwrap()
    }

    #[test]
    fn rejects_unknown_key() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = base_config(dir.path());
        let mut items = BTreeMap::new();
        items.insert(
            "kubo.binary".to_string(),
            InputValue::Str("/bin/ipfs".to_string()),
        );
        let err = update(&cfg, &items).unwrap_err();
        assert!(err.to_string().contains("kubo.binary"));
    }

    #[test]
    fn rejects_env_sourced_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swing.toml");
        std::fs::write(
            &path,
            "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n",
        )
        .unwrap();
        let mut cfg = config::build_config_from_str(
            "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n",
            |k| match k {
                "SWING_MAX_TOTAL_STORAGE" => Some("10GB".to_string()),
                _ => None,
            },
        )
        .unwrap();
        cfg.config_path = path;
        cfg.config_exists = true;
        let mut items = BTreeMap::new();
        items.insert(
            "policy.max_total_storage".to_string(),
            InputValue::Str("20GB".to_string()),
        );
        let err = update(&cfg, &items).unwrap_err();
        assert!(err.to_string().contains("environment variable"));
    }

    #[test]
    fn writer_preserves_comments_and_unrelated_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swing.toml");
        std::fs::write(
            &path,
            "# a comment\n[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\nmirror_set = \"keep-me\"\n",
        )
        .unwrap();
        let cfg = Config::load(Some(&path)).unwrap();
        let mut items = BTreeMap::new();
        items.insert(
            "policy.max_total_storage".to_string(),
            InputValue::Str("20GB".to_string()),
        );
        let updated = update(&cfg, &items).unwrap();
        assert_eq!(updated.policy.max_total_storage, 20 * (1u64 << 30));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# a comment"));
        assert!(text.contains("mirror_set = \"keep-me\""));
    }

    #[test]
    fn new_file_gets_owner_only_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("swing.toml");
        let mut cfg = config::build_config_from_str(
            "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n",
            |_| None,
        )
        .unwrap();
        cfg.config_path = path.clone();
        cfg.config_exists = false;
        let mut items = BTreeMap::new();
        items.insert(
            "nostr.mirror_set".to_string(),
            InputValue::Str("newset".to_string()),
        );
        update(&cfg, &items).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn validation_failure_leaves_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swing.toml");
        std::fs::write(
            &path,
            "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n",
        )
        .unwrap();
        let before = std::fs::read_to_string(&path).unwrap();
        let cfg = Config::load(Some(&path)).unwrap();
        let mut items = BTreeMap::new();
        items.insert(
            "policy.max_total_storage".to_string(),
            InputValue::Str("not-a-size".to_string()),
        );
        assert!(update(&cfg, &items).is_err());
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn empty_relay_list_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = base_config(dir.path());
        let mut items = BTreeMap::new();
        items.insert("nostr.relays".to_string(), InputValue::List(vec![]));
        let err = update(&cfg, &items).unwrap_err();
        assert!(err.to_string().contains("at least one relay"));
    }

    #[test]
    fn setup_writes_generated_key_and_leaves_setup_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swing.toml");
        let mut cfg = config::build_config_from_str("", |_| None).unwrap();
        cfg.config_path = path.clone();
        cfg.config_exists = false;
        assert!(cfg.nostr.secret_key.is_none());
        let items = BTreeMap::new();
        let keys = setup_keys(None).unwrap();
        let reloaded = setup(&cfg, Some(&keys), &items).unwrap();
        assert_eq!(
            reloaded
                .nostr
                .secret_key
                .as_ref()
                .map(|k| k.expose_secret()),
            Some(keys.secret_key().to_secret_hex().as_str())
        );
        assert!(path.exists());
    }

    #[test]
    fn setup_for_a_signer_app_writes_no_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swing.toml");
        let mut cfg = config::build_config_from_str("", |_| None).unwrap();
        cfg.config_path = path.clone();
        cfg.config_exists = false;
        let mut items = BTreeMap::new();
        items.insert(
            "nostr.relays".to_string(),
            InputValue::List(vec!["wss://relay.example".to_string()]),
        );
        let reloaded = setup(&cfg, None, &items).unwrap();
        assert!(reloaded.nostr.secret_key.is_none());
        assert_eq!(reloaded.nostr.relays, vec!["wss://relay.example"]);
    }

    #[test]
    fn catalog_covers_exactly_the_keys_build_config_resolves() {
        let cfg = config::build_config_from_str("", |_| None).unwrap();
        let catalog_keys: std::collections::BTreeSet<&str> =
            SETTINGS.iter().map(|s| s.key).collect();
        let source_keys: std::collections::BTreeSet<&str> =
            cfg.sources.keys().map(String::as_str).collect();
        assert_eq!(catalog_keys, source_keys);
    }

    #[test]
    fn catalog_has_exactly_20_editable_keys() {
        assert_eq!(SETTINGS.iter().filter(|s| s.editable).count(), 20);
    }

    #[test]
    fn swing_example_toml_matches_generator() {
        let generated = render_toml_example();
        let checked_in = include_str!("../swing.example.toml");
        assert_eq!(
            checked_in, generated,
            "swing.example.toml is out of date; run `swing config example > swing.example.toml` and commit the result"
        );
    }

    #[test]
    fn env_example_matches_generator() {
        let generated = render_env_example();
        let checked_in = include_str!("../.env.example");
        assert_eq!(
            checked_in, generated,
            ".env.example is out of date; run `swing config env-example > .env.example` and commit the result"
        );
    }

    #[test]
    fn env_no_effect_in_compose_matches_compose_yaml() {
        fn indent(line: &str) -> usize {
            line.len() - line.trim_start().len()
        }

        fn find_after(lines: &[&str], start: usize, end: usize, target: &str) -> Option<usize> {
            lines[start..end]
                .iter()
                .position(|l| l.trim() == target)
                .map(|i| start + i)
        }

        fn block_end(lines: &[&str], header: usize, limit: usize) -> usize {
            let header_indent = indent(lines[header]);
            lines[header + 1..limit]
                .iter()
                .position(|l| !l.trim().is_empty() && indent(l) <= header_indent)
                .map(|i| header + 1 + i)
                .unwrap_or(limit)
        }

        fn mirror_environment_literal_swing_vars(compose: &str) -> Vec<&str> {
            let lines: Vec<&str> = compose.lines().collect();
            let mirror = find_after(&lines, 0, lines.len(), "mirror:")
                .expect("compose.yaml: no `mirror:` service found");
            let mirror_end = block_end(&lines, mirror, lines.len());
            let environment = find_after(&lines, mirror, mirror_end, "environment:")
                .expect("compose.yaml: mirror service has no `environment:` block");
            let environment_end = block_end(&lines, environment, mirror_end);

            lines[environment + 1..environment_end]
                .iter()
                .filter_map(|line| {
                    let entry = line.trim().trim_start_matches("- ");
                    let (key, value) = entry.split_once([':', '='])?;
                    let key = key.trim();
                    let value = value.trim();
                    (key.starts_with("SWING_") && !value.contains("${")).then_some(key)
                })
                .collect()
        }

        let compose = include_str!("../compose.yaml");
        let found: std::collections::BTreeSet<&str> =
            mirror_environment_literal_swing_vars(compose)
                .into_iter()
                .collect();

        for env_name in &found {
            assert!(
                SETTINGS.iter().any(|s| s.env == *env_name),
                "{env_name}: found in compose.yaml's mirror environment block but is not a catalog env var"
            );
        }

        let listed: std::collections::BTreeSet<&str> = ENV_NO_EFFECT_IN_COMPOSE
            .iter()
            .map(|key| env_of(key))
            .collect();

        assert_eq!(
            found, listed,
            "ENV_NO_EFFECT_IN_COMPOSE is out of sync with compose.yaml's mirror service environment block"
        );
    }

    #[test]
    fn generated_toml_example_parses_to_defaults() {
        let generated = render_toml_example();
        let cfg = config::build_config_from_str(&generated, |_| None)
            .expect("generated swing.example.toml must parse");
        let defaults = config::build_config_from_str("", |_| None).unwrap();

        assert_eq!(cfg.policy, defaults.policy);
        assert_eq!(cfg.kubo, defaults.kubo);
        assert_eq!(cfg.publish, defaults.publish);
        assert_eq!(cfg.gateway, defaults.gateway);
        assert_eq!(cfg.dashboard, defaults.dashboard);
        assert_eq!(cfg.ipfs.api, defaults.ipfs.api);
        assert_eq!(cfg.ipfs.mfs_root, defaults.ipfs.mfs_root);

        assert_eq!(
            cfg.nostr.secret_key.is_none(),
            defaults.nostr.secret_key.is_none()
        );
        assert_eq!(cfg.nostr.relays, defaults.nostr.relays);
        assert_eq!(cfg.nostr.mirror_set, defaults.nostr.mirror_set);
        assert_eq!(cfg.nostr.site_event_kind, defaults.nostr.site_event_kind);
        assert_eq!(
            cfg.nostr.replica_event_kind,
            defaults.nostr.replica_event_kind
        );

        assert_eq!(cfg.agent.state_dir, defaults.agent.state_dir);
        assert_eq!(cfg.agent.poll_interval, defaults.agent.poll_interval);
        assert_eq!(cfg.agent.fetch_timeout, defaults.agent.fetch_timeout);
        assert_eq!(
            cfg.agent.fetch_idle_timeout,
            defaults.agent.fetch_idle_timeout
        );
        assert_eq!(cfg.agent.concurrency, defaults.agent.concurrency);
        assert_eq!(cfg.agent.report_ttl, defaults.agent.report_ttl);
    }
}
