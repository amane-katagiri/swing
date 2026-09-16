# 2026-09-16 unfollow の判定を state 基準にし、`swing sites` にフォロー外のサイトを出す

## 問題

- `remove_on_unfollow = true` でも、agent の停止中に Follow Set から外した相手のサイトが消えなかった。外れた相手を「メモリ上の前回の Follow Set との差分」で求めていたが、起動直後はその集合が空なので差分も空になる。
- `remove_on_unfollow = false` で残したサイトは `swing sites` に表示されず、後から `true` に変えても（既に外れているので差分に出ず）消す手段が無かった。

## 決めたこと

| 決定 | 理由 |
|---|---|
| Follow Set を取得できるたびに、state（`sites` と `verifications`）にいる pubkey のうち今の Follow Set にいないものを消す | 起動直後でも、`false` から `true` に変えた後でも、残っている相手を消せる |
| Follow Set が見つからない、または取得に失敗した tick では何もしない | relay から一時的に取れないだけで全員を消さないため（以前と同じ） |
| 削除は state を先に保存してから、pubkey ごとの MFS ディレクトリを消す | 途中で失敗しても sweep が片付ける |
| `swing sites` の最後に、state に版があって Follow Set にいない pubkey を `[unfollowed]` で表示する | `false` で残しているサイトに気づけるようにする |
| `swing sites` は Follow Set が見つからなくても、この部分を表示する | Follow Set が無い状態こそ、残っているものを確認したい |
| フォロー外のサイトを消す専用のコマンドは作らず、`remove_on_unfollow = true` にして agent を再起動する手順を案内する | state.json は agent が持ち続けて書き換えるので、別プロセスの CLI が書き換えると上書きし合う |

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`（ユニットテスト 149 件、ほかに `#[ignore]` 1 件）。停止中に外した相手が最初の確認で消えるテスト、変化が無ければ state を書かないテスト、`State::accounts` / `remove_account`、`swing sites` のフォロー外一覧の組み立てを追加した。
- ローカルの Kubo と nostr-rs-relay で、publish → 自分をフォロー → agent が保存 → フォロー解除（`false`）→ agent は何も消さない → `swing sites` が `[unfollowed]` を表示 → `true` にして agent を再起動 → 最初の確認で state と MFS から消える、までを通した。
