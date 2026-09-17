# 2026-09-17 レプリカ報告（kind 35981）と `swing replicas`

## 目的

plan §17 の「レプリカ数可視化」。agent が保存している CID を Nostr に報告し、サイトごとに何人が最新版を持っているかを表示する。

## 決めたこと

| 決定 | 理由 |
|---|---|
| 報告は `d = <作者>:<サイトd>` の addressable event（予約どおり）で、`a`・`p`・`cid`（複数）・`expiration`・`alt` を付ける | 報告者 × サイトで 1 系列にすれば、更新も取り下げも置き換えで済む。`p` は作者が自分のサイト分をまとめて取るため |
| `cid` は保存している版をすべて並べる | 集計側が「最新版を持つ」と「古い版だけ持つ」を区別できる |
| 取り下げは `cid` 無しの報告で置き換える | NIP-09 を処理しない relay があっても、addressable の置き換えは効く |
| NIP-40 の `expiration` を `created_at + report_ttl`（既定 `3d`）にし、`report_ttl / 2` 過ぎたら出し直す | 止まったまま戻らない agent の報告を自然に消す。集計側でも期限を判定するので、NIP-40 非対応の relay でも数えない |
| `report_ttl / 2 <= poll_interval` を設定エラーにする | 出し直しの判定は tick ごとなので、tick の間に期限が切れないようにする |
| kind は `site_event_kind` と同じく `replica_event_kind` で変えられるようにする | テストや別ネットワークでの試験のため。既定は 35981 |
| `swing publish` は報告を出さず、agent が `<mfs_root>/publish/<自分>/` も見て、自分のサイトの報告を出す | publish は常駐しないので出し直しができない。また同じ鍵で agent も同じ `d` の系列に書くと、互いの `cid` を上書きし、起動時の取り下げで publish の報告を消してしまう。agent がまとめて 1 つの報告にすれば両方解決する |
| 別の Kubo で publish したサイトは作者本人として数えない | agent から見えない。数が 1 減るだけで壊れるものはないので許容する（相談して決めた） |
| 送信済みの記録はメモリだけに持ち、起動後に relay から自分の報告を読んで補う | state.json の形を変えずに済み、後方互換の処置が要らない。relay にある古い報告（止まっている間に unfollow したサイトなど）もこれで取り下げられる |
| relay からの読み込みに失敗しても送信は続け、読めるまで同期のたびに再試行する | 報告の出し直しを relay の読み込みに依存させない。取り下げだけが遅れる |
| 同じ秒に出し直すときは `created_at` を前回 + 1 にする | 同じ `created_at` だと NIP-01 では `id` の小さい方が残り、古い内容が勝つことがある |
| 一覧に失敗したサイトは報告を送らない | 保存しているのに取り下げたり、publish 側の CID を落とした報告で上書きしたりしないため |
| 報告の送信は同時実行の枠を返してから、報告用のロックの中で行う。state のロックは CID を読む間だけ | relay への送信（最大 30 秒程度）で保存処理を止めない。ロックの順は報告用 → state に固定する |
| 送信処理は `ReportRelay` trait にし、テストでは偽物を使う | `KuboStore` / `Nip05Verify` と同じ形 |
| `swing replicas [<key>...]` を追加し、`swing sites` にも `replicas=N` を出す | 自分のサイトを誰が持っているかが一番知りたい情報なので既定は自分。`sites` は数だけ |
| 報告者の Follow Set に作者がいなければ `[not following]` を付け、数からは除かない | 報告は誰でも出せる。除くかどうかを決めるより、印を付けて見せる方が情報を失わない。Webring 表示でも Follow Set の取得を使い回せる |
| 集計側では署名を改めて確かめない | nostr-sdk が受信時に確かめている（`verify_and_cache`） |

## 作ったもの

- `nostr.rs`: `ReplicaReport`、`build_replica_report_builder`、`parse_replica_report`、`newest_by_address`、`site_coordinate`、`RelayClient::fetch_replica_reports` / `fetch_follow_sets`、`ReportRelay`（`RelayClient` と `Arc<T>` に実装）。`validate_d_tag` を公開。
- `agent.rs`: `sync_reports`（保存している CID の算出、relay からの読み込み、`reports_to_send` による送信内容の決定、送信）。tick の最後と、新版を記録したタスクの後に呼ぶ。`apply_site_event` は記録したかを返す。
- `mfs.rs`: `publish_account`、`site_from_name`（`site_name` の逆。別の綴りは拒否）。
- `replicas.rs`: `collect_reports`、`replicas_of`、`latest_count`、`fetch_for_sites`、`swing replicas`。
- `config.rs`: `replica_event_kind`（`SWING_REPLICA_EVENT_KIND`）、`report_ttl`（`SWING_REPORT_TTL`）。
- ドキュメント: `protocol.md` に第 8 節（以降の節番号を 1 つずらした）、`extensions.md` の 35981 を実装済みに、architecture・README・`swing.example.toml`・todo を更新。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`（ユニットテスト 174 件、ほかに `#[ignore]` 10 件）。追加したテスト: 報告の組み立てとパースの往復、`d` と `a` の不一致・不正な作者・不正な CID の拒否、`newest_by_address`、`site_from_name`、`report_ttl` の検証、agent（保存後の送信、変化が無ければ送らない、期限前の出し直し、同じ秒の `created_at`、unfollow の取り下げが 1 回だけ、起動時に relay の報告を読んで取り下げ・維持する、読み込み失敗後の取り下げ、受理されなかった報告の再送、publish ディレクトリの版を合わせた報告、一覧に失敗したサイトを送らない）、集計（最新・期限切れ・取り下げ・不正な報告の扱い、最新版での数え方、印の付け方）。
- ローカルの nostr-rs-relay（`scsibug/nostr-rs-relay`）で `nostr_relay_integration` の 2 件を通した。`#a` での取得、置き換えによる取り下げ、自分の報告の取得、Follow Set の取得を含む。
- ローカルの Kubo（v0.43.1、test プロファイル）で `kubo_integration` の 7 件と `agent_stores_and_removes_through_real_kubo` を通した。
- 同じ relay と Kubo で、使い捨ての鍵 A（作者）と B（ミラー）を使って通しで確かめた。
  1. A が publish し、B が A を Follow Set に入れ、両方の agent を動かす。A は publish ディレクトリの版を、B は保存した版を 1 回ずつ報告し、その後の tick では送り直さない。`swing replicas <A>` は `replicas=2`（B が `[latest]`、A が `[latest] [author]`）、`swing sites` は `replicas=2`。
  2. A が新しい版を publish し、A の agent だけを動かす。A の報告は `cid` 2 つになり、`replicas=1`、B は `[older version]`。
  3. B の agent を動かすと新版を保存して報告し、`replicas=2` に戻る。
  4. B が `mirror remove` した後に B の agent を起動し直すと、unfollow して `cid` 無しの報告を 1 回送り、`replicas=1 (reports=1)` になる。
