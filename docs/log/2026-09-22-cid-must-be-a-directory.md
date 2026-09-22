# cid が UnixFS のディレクトリ root であることの確認

## きっかけ

`docs/todo.md` にあった監査項目。`protocol.md` 第 4 節は `cid` を「サイトのディレクトリ root を指す有効な CID」と定義しているが、実装はその意味を確かめていなかった。`parse_site_event`/`parse_replica_report` は `cid::Cid::try_from` で構文だけを検証し、`apply_site_event`（`src/agent/store.rs`）は `fetch_dag` で取得した後、中身を見ずに `mfs_put` していた。単一ファイルや raw block、dag-cbor ノードを `cid` に入れても構文上は妥当な CID なので、そのまま「サイト」として保存されてしまう。バイト数の上限は効くのでリソース枯渇ではないが、ゲートウェイでの閲覧（`/ipfs/<cid>/`）が壊れ、「版」の意味そのものが崩れる。

## やったこと

2 段構えにした。

1. **パース時のコーデック検証**: `src/nostr.rs::canonical_cid` に、CID のコーデックが dag-pb（`0x70`）でなければ拒否する判定を足した。UnixFS のディレクトリ root は必ず dag-pb（CIDv0 は常に dag-pb）なので、他のコーデック（raw `0x55`、dag-cbor `0x71` など）はこの時点で「ディレクトリではあり得ない」と分かる。別関数 `require_dag_pb` に分けることも考えたが、`canonical_cid` を呼ぶ場所（`parse_site_event`・`parse_replica_report`・`swing publish` の `add_and_measure`）は元々「CID として使える形にする」という 1 つの不変条件を期待している箇所なので、コーデック検証もそこに含めて「`canonical_cid` を通った文字列は dag-pb の CIDv1・base32」という 1 つの保証にまとめた。`add_and_measure` は Kubo の `add_dir`（既定で dag-pb・CIDv1）の結果にしか使わないので影響はない。

2. **取得後の実体確認**: `KuboStore` トレイト（`src/ipfs.rs`）に `is_directory(&self, cid: &str) -> Result<bool>` を足した。`IpfsClient` の実装は `files/stat?arg=/ipfs/{cid}` を呼び、`Type` フィールドが `"directory"` かどうかを見る（`FilesStatResponse` に `Type` フィールドを追加。既存の `mfs_stat_cid` も同じ構造体を使うが `Hash` しか読まないので影響しない）。ブロックは直前の `fetch_dag` でローカルに揃っているので、ルートブロック 1 つを読むだけの軽い呼び出しになる。タイムアウトは他の MFS 系呼び出しと同じ 60 秒にした。

   `apply_site_event`（`src/agent/store.rs`）で、`fetch_dag` が成功した直後・`mfs_put` の前にこの確認を挟んだ。`Ok(false)`（ディレクトリでない）なら `reason = "not_a_directory"` で warn を出して保存せずに終わる。MFS にはまだ何も置いていないので消すものが無く、取得済みのブロックは打ち切ったフェッチと同じく Kubo の GC に任せる。`Err`（`files/stat` 自体の失敗）は取得失敗と同じ扱いにし、warn を出して次の poll での再試行に委ねた。

   `KuboStore` の実装がもう 2 つある。`FakeKubo`（`src/agent/test_support.rs`）は `FakeKuboState` に `files: HashSet<String>` を足し、デフォルトはディレクトリ、`files` に入れた CID だけファイル扱いにする。`health.rs` のテスト専用 `FakeKubo` は `is_directory` を呼ぶ経路が無いので `unreachable!()` にした。

## 検証

- `src/nostr.rs`: `canonical_cid`・`parse_site_event`・`parse_replica_report` それぞれに、raw コーデック（`bafkrei...`）と dag-cbor コーデック（`bafyrei...`）の CID を拒否するテストを足した。テスト用の CID は `cid` クレートに対する使い捨てスクリプトで、既存の dag-pb フィクスチャ（`bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi`）と同じマルチハッシュのまま `Cid::new_v1(0x55, hash)` / `Cid::new_v1(0x71, hash)` を作って確認してから貼った。dag-pb の v0・v1 はどちらも既存のテスト（`accepts_cidv0_and_canonicalizes_it` など）でカバー済み。
- `src/agent/store.rs`: `FakeKubo` にファイル扱いの CID を渡すテストを足し、`fetch_dag` は呼ばれる（`fetched` に載る）が `mfs_put` は呼ばれず（MFS のパスが作られない）、state にも記録されないことを確認した。
- `tests/kubo_integration.rs`（`#[ignore]`、実 Kubo が要る）に `is_directory` の統合テストを足し、`add_dir` で作ったディレクトリ CID は `true`、その中の `index.html` のファイル CID は `false` になることを確かめた。
- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` を通した。

## 見送ったこと

- 完全な取得の前にタイプだけ先に見ること。ルートブロックだけでもタイプは分かるが、そのために別の RPC 往復を挟むより、どうせ全体をバイト数上限内で取得する設計なので、取得を最後まで走らせてから 1 回の `files/stat` で確認する方が単純だと判断した。
- HAMT でシャーディングされたディレクトリの特別扱い。HAMT シャードの root も UnixFS のディレクトリ（`Data` に `Type: HAMTShard` を持つ dag-pb ノード）であり、`files/stat` は `Type: "directory"` を返すので、今回の確認でそのまま通る。特別なコードは要らない。
