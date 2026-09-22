# CID の正規形での比較

## きっかけ

自己申告のイベント内容を信用しすぎている箇所の監査で見つかった。`parse_site_event` と `parse_replica_report` は `cid` タグを `cid::Cid::try_from` で妥当性だけ確かめ、タグの文字列をそのまま `SiteEvent::cid` / `ReplicaReport::cids` に入れていた。`policy::decide` の `duplicate_cid` 判定は文字列比較、`replicas::replicas_of` は報告の `cid` と現在の版の `cid` を文字列で突き合わせ、`state.json` と MFS のパスも文字列をそのまま使う。CIDv0（`Qm...`）と CIDv1 の base58btc（`z...`）・base32（`bafy...`）は同じコンテンツを指すが、これらはすべて別の文字列なので、同じ内容を別表記で再署名すれば `duplicate_cid` を回避でき、`keep_versions` を無意味に消費させたり fetch と MFS の書き込みを繰り返させたりできる。レプリカ報告も表記が食い違うと一致しなくなる。

## やったこと

`src/nostr.rs` に `pub fn canonical_cid(s: &str) -> Result<String>` を足した。`cid::Cid::try_from` でパースし、`Cid::into_v1()`（`cid` 0.11 の API、CIDv0 は dag-pb コーデックの CIDv1 に変換、CIDv1 はそのまま）を通してから `to_string()` する。`Cid` の `Display` は CIDv1 なら常に base32 lowercase の multibase 表記になる。パース失敗時のメッセージは既存のテストが意味を保つよう「invalid cid tag」のままにした。

`parse_site_event` と `parse_replica_report` の `cid`/`cid` タグの読み取りをこの関数に置き換えた。`ReplicaReport::cids` はもともと `BTreeSet<String>` なので、1 つの報告の中で同じ版を 2 つの表記で書いても正規化後は 1 エントリに畳まれる。

`swing publish`（および dashboard の publish API が共有する）`add_and_measure` は、Kubo の `add_dir` が返した CID もこの関数に通すようにした。Kubo は既定で CIDv1 base32 を返すのでほぼ無変換だが、「SWING の中を流れる CID 文字列はすべて正規形」という不変条件を発行側でも保つ。CLI やダッシュボードのどこにも、運用者が CID を直接タイプして渡す入力（`--cid` のような引数やアップロード API）は無かったので、他に適用箇所は無い。

`policy::decide` と `replicas::replicas_of` は文字列比較のままで変更していない。入力が正規化済みになった時点でこれらは自動的に「デコードした値で比較する」のと同じ結果になるため。

## 検証

- `src/nostr.rs`: `canonical_cid` の単体テスト（CIDv0 → base32、base58btc → base32、base32 → 無変換、不正な文字列 → `Err`）。
- `parse_site_event` の既存テスト（`accepts_cidv0_and_canonicalizes_it`）を、パース結果が正規形になることを確かめる形に更新した。
- `parse_replica_report` に、同じ CID を 2 つの表記で書いた報告が 1 エントリに畳まれることを確かめるテストを足した。
- `src/replicas.rs` と `src/agent/replicas.rs` の既存テストは、パース後に正規化された CID 文字列を期待するように 1 箇所修正した（値そのものの比較ロジックは変えていない）。
- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` を通した。

## 見送ったこと

- 既存の `state.json` に別表記で保存済みの版の書き換え。今回の変更は「これから解析するイベントの CID を正規化する」だけで、すでに保存済みのエントリはそのままの表記で残る。その結果、同じ内容が正規形で再度アナウンスされると、SWING はそれを 1 回だけ新しい版として扱う（`duplicate_cid` にならない）。これは許容範囲として見送った。
- 別表記を両方試す照合コード（正規形と旧表記の両方でルックアップする、など）。後方互換のための処置になるので入れていない。
