# 2026-09-16 アカウントあたりのサイト数上限、手動 pin の保護、Kubo の StorageMax 設定

## 目的

- 1 pubkey が `d` を変えて大量のサイトを出したとき、取得・NIP-05 検証・`state.verifications` の件数が増え続けないようにする。
- 取得後に reject した CID や evict・unfollow した CID を、運用者が SWING の外で pin していた場合にも unpin してしまう問題を直す。
- README で手動実行を勧めていた `Datastore.StorageMax` の設定を、compose の起動時に行う。

## 決めたこと

| 決定 | 理由 |
|---|---|
| `max_sites_per_account`（既定 10、0 は設定エラー）を追加し、記録済みの版が無いサイトにだけ適用する | 既存サイトの更新を止めると、上限を下げたときに更新が届かなくなる。上限を下げても既存サイトは消さない |
| 上限は 3 か所で効かせる。poll ごとの投入（pin 済み優先、残りは新しい順）、`submit` での同時実行数、`policy::decide` | `decide` だけでは、pin できない `d` を大量に出されたときに取得と NIP-05 検証が poll ごとに走る。`decide` では実行中のサイトを数えない。数えると、残り枠 1 に 2 サイトが同時に来たとき互いを数えて両方 skip し、以後も同じことを繰り返すため。`decide` は state のロック内で直列なので、確定済みの数だけで正確に判定できる |
| `submit` で上限を超えたイベントは捨てる | 次の poll で、新しい順の選択を通って拾い直される |
| pin されていないサイトの `verifications` は、記録のたびに pubkey ごとに新しい順で `max_sites_per_account` 件まで残す | `d` を poll ごとに変えられると、選択で件数を抑えても記録が溜まる |
| 手動 pin は、SWING が pin する直前に `pin/ls` で recursive と direct を確認し、`VersionRecord.preexisting_pin` に記録する | Kubo 0.43.1 で、direct pin のある CID を recursive で pin すると direct pin が recursive に置き換わり、SWING の `pin/rm` で消えることを確認した。`type=all` は pin されていない CID のとき全 pin の DAG をたどって indirect を探すので使わない |
| state に同じ CID の記録があれば、`pin/ls` を見ずにその記録のフラグを引き継ぐ | その CID は SWING も pin しているので、`pin/ls` では手動かどうか区別できない。引き継がないと、手動 pin のサイト A と同じ CID のサイト B が偽で記録され、A の後に B を解放したとき unpin してしまう |
| `pin/ls` の確認に失敗したら、その回は pin しない | 手動 pin かどうか分からないまま記録すると、後で消してしまうおそれがある。次の poll で再試行される |
| `preexisting_pin` は serde の属性を付けない必須キーにする。`State` の `sites` / `verifications` に付けていた `#[serde(default)]` も外す | どれも古い形式の state.json を読むためのものだが、運用中の state.json がまだ無く後方互換は不要。キーが無い state.json は読み込みエラーになる。state.json が無い、または空白だけのときに空の state として扱う処理は初回起動のためのものなので残す |
| `StorageMax` は Kubo イメージの `/container-init.d` に置いたスクリプトで毎回設定する | イメージの `start_ipfs` は repo 初期化の有無にかかわらず毎回このディレクトリのスクリプトを実行する。`IPFS_PROFILE` は初期化時だけで、既存 repo の設定変更には使えない |
| 値は `SWING_KUBO_STORAGE_MAX`、無ければ `SWING_MAX_TOTAL_STORAGE`、無ければ `100GB`（compose の入れ子の変数展開） | `max_total_storage` と同じにしておけば、pin の合計が上限に近づくまで GC は走らない。余裕を持たせたいときだけ別に指定する |

## 見送ったこと

- NAT64 / 6to4 のアドレス判定は todo のまま。IPv6 のみのネットワークで NAT64 ゲートウェイが内部の IPv4 に届く構成でしか効かず、既定の Docker ブリッジは IPv6 を持たない。
- 取得前（NIP-05 検証や `dag/export` の前）には手動 pin を確認しない。解放の判断に必要なのは pin 直前の状態だけで、取得の前後で変わる可能性もあるため。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`（ユニットテスト 133 件）。
- 追加したテスト: `decide` のサイト数上限、poll の選択（pin 済み優先、上限超過時は pin 済みのみ）、`submit` の同時実行上限、`verifications` の間引き、手動 pin の reject・evict・unfollow での保持、フラグの引き継ぎ、`pin/ls` 失敗時の中断、state の集計関数。
- `verifications` キーが無い state.json を読めることを確かめていたテストは、読み込みがエラーになることを確かめるテストに置き換えた。
- 解放時の `preexisting_pin` の確認を外すと、手動 pin のテスト 3 件が失敗することを確かめた。
- ローカルの Kubo 0.43.1（`IPFS_PROFILE=test`）で統合テスト 5 件（`is_pinned` の recursive / direct / 未 pin / 不正な引数を含む）を実行した。
- 同じイメージで `/container-init.d` のスクリプトが実行され `Datastore.StorageMax` が `25GB` になること、変数が無いとスクリプトが失敗してコンテナが止まることを確認した。`docker compose config` で、既定値の入れ子の展開（`100GB` / `SWING_MAX_TOTAL_STORAGE` / `SWING_KUBO_STORAGE_MAX`）を確認した。
