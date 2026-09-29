use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::{Config, Source};

mod edit;
mod example;

pub use edit::{EditError, pin_addrs, setup, setup_keys, update};
pub use example::{render_env_example, render_toml_example};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Size,
    Duration,
    Bool,
    Integer,
    String,
    List,
    Mode,
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

#[derive(Debug, Clone, Copy)]
pub enum Example {
    Value(&'static str),
    Commented(&'static str),
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
        example: Example::Value("\"100GiB\""),
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
        example: Example::Value("\"10GiB\""),
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
        example: Example::Value("\"20GiB\""),
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
        example: Example::Value("\"2GiB\""),
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
        kind: Kind::Mode,
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
            en: "How long a replica report stays valid; it is re-sent after half this time, must be more than twice poll_interval, and at most 7d (receivers stop counting older reports).",
            ja: "レプリカ報告の有効期間。半分の期間を過ぎたら新しい報告を出す。poll_interval の 2 倍より長く、7d 以下にする必要がある（受信側はそれより古い報告を数えない）",
        },
    },
    Setting {
        key: "publish.nip05",
        section: "publish",
        field: "nip05",
        env: "SWING_PUBLISH_NIP05",
        kind: Kind::Mode,
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
        key: "publish.check_dotfiles",
        section: "publish",
        field: "check_dotfiles",
        env: "SWING_PUBLISH_CHECK_DOTFILES",
        kind: Kind::Mode,
        example: Example::Value("\"require\""),
        editable: true,
        description: Text {
            en: "What swing publish does when the site contains files or directories whose names start with a dot, other than dotfiles_allow (off / warn / require); the --check-dotfiles CLI flag takes precedence.",
            ja: "サイトに dotfiles_allow 以外のドットで始まる名前のファイル・ディレクトリがあったときの swing publish の扱い（off / warn / require、--check-dotfiles が優先）",
        },
    },
    Setting {
        key: "publish.dotfiles_allow",
        section: "publish",
        field: "dotfiles_allow",
        env: "SWING_PUBLISH_DOTFILES_ALLOW",
        kind: Kind::List,
        example: Example::Value(
            "[\".well-known\", \".nojekyll\", \".gitkeep\", \".keep\", \".domains\"]",
        ),
        editable: true,
        description: Text {
            en: "Dotfile names that check_dotfiles lets through, along with everything beneath them (comma-separated as an env var); setting it replaces the default list.",
            ja: "check_dotfiles が通すドットで始まる名前（その下も含めて通す、カンマ区切り）。指定すると既定の一覧を置き換える",
        },
    },
    Setting {
        key: "publish.check_size",
        section: "publish",
        field: "check_size",
        env: "SWING_PUBLISH_CHECK_SIZE",
        kind: Kind::Mode,
        example: Example::Value("\"warn\""),
        editable: true,
        description: Text {
            en: "What swing publish does when the files of the site add up to more than 512 MiB (off / warn / require); the --check-size CLI flag takes precedence.",
            ja: "サイトのファイルの合計が 512 MiB を超えたときの swing publish の扱い（off / warn / require、--check-size が優先）",
        },
    },
    Setting {
        key: "publish.check_unchanged",
        section: "publish",
        field: "check_unchanged",
        env: "SWING_PUBLISH_CHECK_UNCHANGED",
        kind: Kind::Mode,
        example: Example::Value("\"require\""),
        editable: true,
        description: Text {
            en: "What swing publish does when the new CID equals that of your latest site event on the relays (off / warn / require; require stops without publishing); the --check-unchanged CLI flag takes precedence.",
            ja: "新しい CID が relay 上の自分の最新版と同じだったときの swing publish の扱い（off / warn / require。require は publish せずに終える、--check-unchanged が優先）",
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
        example: Example::Derived("\"http://localhost:8080\""),
        editable: true,
        description: Text {
            en: "IPFS gateway used for links to saved sites; an empty value hides the links. Defaults to http://localhost:<port of [kubo].gateway_listen> when managed, otherwise http://localhost:8080.",
            ja: "保存済みサイトを開くリンクの IPFS Gateway。空文字にするとリンクを表示しない。既定: managed なら http://localhost:<[kubo].gateway_listen のポート>、そうでなければ http://localhost:8080",
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
        key: "dashboard.mascots_dir",
        section: "dashboard",
        field: "mascots_dir",
        env: "SWING_DASHBOARD_MASCOTS_DIR",
        kind: Kind::Path,
        example: Example::Commented("\"/path/to/mascots\""),
        editable: false,
        description: Text {
            en: "Directory of user-defined Desktop mascot packs (one subdirectory per pack); read once at startup in addition to the two bundled packs.",
            ja: "ユーザー定義の Desktop マスコットパックを置くディレクトリ（1 サブディレクトリ = 1 パック）。同梱の 2 パックに加えて起動時に 1 回読み込む",
        },
    },
    Setting {
        key: "dashboard.max_upload",
        section: "dashboard",
        field: "max_upload",
        env: "SWING_DASHBOARD_MAX_UPLOAD",
        kind: Kind::Size,
        example: Example::Value("\"2GiB\""),
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
        example: Example::Derived("\"100GiB\""),
        editable: true,
        description: Text {
            en: "Kubo's Datastore.StorageMax; defaults to the same value as [policy].max_total_storage. Write it in GiB-style units so the compose Kubo reads the same value (it reads GB as decimal).",
            ja: "Kubo の Datastore.StorageMax。既定: [policy].max_total_storage と同じ値。GiB 系で書くと compose の Kubo でも同じ値になる（Kubo は GB を 10 進で解釈する）",
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

/// Exists so each env var name lives in exactly one place (the catalog).
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
        "policy.max_total_storage" => {
            RawValue::Str(crate::format::format_bytes(config.policy.max_total_storage))
        }
        "policy.max_per_site" => {
            RawValue::Str(crate::format::format_bytes(config.policy.max_per_site))
        }
        "policy.max_per_account" => {
            RawValue::Str(crate::format::format_bytes(config.policy.max_per_account))
        }
        "policy.max_update_size" => {
            RawValue::Str(crate::format::format_bytes(config.policy.max_update_size))
        }
        "policy.max_sites_per_account" => {
            RawValue::Str(config.policy.max_sites_per_account.to_string())
        }
        "policy.keep_versions" => RawValue::Str(config.policy.keep_versions.to_string()),
        "policy.keep_days" => RawValue::Str(config.policy.keep_days.to_string()),
        "policy.min_update_interval" => RawValue::Str(crate::format::format_duration_secs(
            config.policy.min_update_interval,
        )),
        "policy.nip05_cache_ttl" => RawValue::Str(crate::format::format_duration_secs(
            config.policy.nip05_cache_ttl,
        )),
        "agent.poll_interval" => RawValue::Str(crate::format::format_duration_secs(
            config.agent.poll_interval.as_secs(),
        )),
        "agent.report_ttl" => RawValue::Str(crate::format::format_duration_secs(
            config.agent.report_ttl.as_secs(),
        )),
        "policy.remove_on_unfollow" => RawValue::Str(config.policy.remove_on_unfollow.to_string()),
        "policy.nip05" => RawValue::Str(config.policy.nip05.name().to_string()),
        "publish.nip05" => RawValue::Str(config.publish.nip05.name().to_string()),
        "agent.concurrency" => RawValue::Str(config.agent.concurrency.to_string()),
        "publish.keep_versions" => RawValue::Str(config.publish.keep_versions.to_string()),
        "publish.check_dotfiles" => RawValue::Str(config.publish.check_dotfiles.name().to_string()),
        "publish.dotfiles_allow" => RawValue::List(config.publish.dotfiles_allow.clone()),
        "publish.check_size" => RawValue::Str(config.publish.check_size.name().to_string()),
        "publish.check_unchanged" => {
            RawValue::Str(config.publish.check_unchanged.name().to_string())
        }
        "kubo.storage_max" => RawValue::Str(crate::format::format_bytes(config.kubo.storage_max)),
        "dashboard.gateway" => RawValue::Str(config.dashboard.gateway.clone().unwrap_or_default()),
        _ => return None,
    };
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_exactly_the_keys_build_config_resolves() {
        let cfg = crate::config::build_config_from_str("", |_| None).unwrap();
        let catalog_keys: std::collections::BTreeSet<&str> =
            SETTINGS.iter().map(|s| s.key).collect();
        let source_keys: std::collections::BTreeSet<&str> =
            cfg.sources.keys().map(String::as_str).collect();
        assert_eq!(catalog_keys, source_keys);
    }

    #[test]
    fn catalog_has_exactly_24_editable_keys() {
        assert_eq!(SETTINGS.iter().filter(|s| s.editable).count(), 24);
    }

    // raw_value() hand-enumerates editable keys separately from the catalog; a key added to
    // SETTINGS without a matching raw_value() arm would silently return None instead of failing.
    #[test]
    fn raw_value_covers_every_editable_key() {
        let cfg = crate::config::build_config_from_str("", |_| None).unwrap();
        for setting in SETTINGS.iter().filter(|s| s.editable) {
            assert!(
                raw_value(&cfg, setting.key).is_some(),
                "raw_value() has no arm for editable key {}",
                setting.key
            );
        }
    }
}
