# 2026-09-16 Follow Set の巻き戻りと消失への対策

## 問題

agent は relay から取れた Follow Set の中で最も新しいものを正しいとみなしていた。最新版を持つ relay が落ちていて、別の relay に古い版しか無いと、古い版の後に追加した相手が「外れた」と判定され、`remove_on_unfollow = true` ならサイトが消える。直前の変更（`2026-09-16-unfollow-from-state.md`）で外れた相手を state と比べるようにしたので、起動直後にも起きるようになっていた。`swing mirror add/remove` も relay の版を元に編集するので、古い版を元に新しい版を作って上書きしてしまう。

## 決めたこと

| 決定 | 理由 |
|---|---|
| agent は採用した Follow Set を `state.follow_set` に署名付きのまま保存し、relay の版と比べて新しい方を使う | 一度見た版より古い版には戻らない。relay から取れないときも、起動直後から対象者が決まる |
| 新しさは NIP-01 の置き換え可能イベントの規則（`created_at`、同じなら小さい `id`）で比べる。relay から取った候補の選択も同じ規則にする | relay が保持する版と判断を揃える |
| 保存済みの版は、kind・作者・`d`・署名を確かめてから使う | `mirror_set` を変えたときに古い `d` の版を使わない。state.json の改ざんで他人のリストを使わない |
| relay の版が古いか見つからないときは、保存済みの版を全 relay にそのまま再送する。取得自体に失敗したときは再送しない | 署名済みなので鍵を使わずに直せる。取得に失敗しているときは relay に届かない見込みが高い |
| 自分で Follow Set を NIP-09 で削除しても再送は止めない | 削除と一時的な消失を区別できない。ミラーをやめるには `mirror remove` を使う |
| `mirror list/add/remove` と `sites` も、state.json の保存済みの版が新しければそちらを使い、その旨を表示する。state.json は読むだけ | 古い版を元にした編集で新しい版を上書きしない。compose では agent と同じ `/data` を見る |
| 最初の案にあった「一定時間外れたままなら消す」猶予は入れない | 巻き戻りは上の対策で防げる。意図して外した相手はすぐ消えるほうが分かりやすい |

`follow_set` は `Option<Event>` なので、serde の既定の挙動でキーが無い state.json も `null` として読める。後方互換のために追加した処理ではない。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`（ユニットテスト 153 件、ほかに `#[ignore]` 1 件）。新しさの比較（同時刻の `id` の比較を含む）、Follow Set の検証（kind・作者・`d`・改ざん）、採用・保存・再送の判定表、`mirror` 側の新しい方の選択のテストを追加した。
- ローカルの Kubo と nostr-rs-relay 2 台で次を確認した。
  - relay1・relay2 に `{P}`、relay1 だけに `{P, Q}` を置き、agent が両方のサイトを保存して `{P, Q}` を state に保存する。
  - relay1 を止めると relay2 からは `{P}` しか取れないが、`mirror list` は保存済みの `{P, Q}` を表示し、agent は Q を消さずに `{P, Q}` を relay2 に再送する。state を持たない設定の `mirror list` でも relay2 から `{P, Q}` が取れるようになる。
  - relay2 を空の状態で作り直すと Follow Set が見つからなくなるが、agent が再送し、`mirror list` に 2 人が戻る。
