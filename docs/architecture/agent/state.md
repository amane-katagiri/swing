# state.json（`state.rs`）

[`../agent.md`](../agent.md) の子ページ。`[agent].state_dir` 直下。`auth::write_private_file` で書く（手順とパーミッションは [`../dashboard/security.md#トークン`](../dashboard/security.md#トークン)）。

```json
{
  "sites": {
    "<pubkey hex>:<d>": [
      { "cid": "bafy...", "size": 12345, "created_at": 1690000000, "stored_at": 1690000100 },
      { "cid": "bafy...", "size": 12345, "created_at": 1700000000, "stored_at": 1700000100, "event": { "id": "...", "pubkey": "...", "created_at": 1700000000, "kind": 35980, "tags": [["d", "example.com"], ["cid", "bafy..."]], "content": "", "sig": "..." } }
    ]
  },
  "verifications": {
    "<pubkey hex>:<d>": { "status": "verified", "detail": null, "checked_at": 1700000100 }
  },
  "follow_set": { "id": "...", "pubkey": "...", "created_at": 1700000000, "kind": 30000, "tags": [["d", "swing"], ["p", "..."]], "content": "", "sig": "..." },
  "withdrawn": {
    "<pubkey hex>:<d>": { "events": ["<event id hex>"], "through": 1700000500 }
  }
}
```

- キーは最初の `:` で `<pubkey hex>` と `<d>` に分ける。
- `sites` と `verifications` は必須キー。ファイルが無い、または空白だけなら空の state として扱う。
- `sites[].size` は `dag/stat` の `TotalSize`。
- `sites[].created_at` は版を保存したサイトイベントの `created_at` で、MFS のパス `<site>/<created_at>` の名前になるので、記録した後は変えない。サイトの「最新の版」は `created_at` が最大の版。
- `status` は `verified` / `mismatch` / `not_applicable` / `error`。`verifications` の扱いは [`../nip05.md`](../nip05.md)。
- `follow_set` は最後に保存した署名済みの Follow Set。無ければ `null`（キーが無くても `null`）。

## `sites[].event`

その版のサイトイベント（relay から受け取った署名済みのもの）。

- 版を保存したとき、署名が正しければ記録し、正しくなければ版だけを記録する。
- サイトごとに最新の版だけが持つ。新しい版を記録すると（`State::apply_store`）ほかの版からは消す。無い版ではキーごと書かない。読むときは無くてもよい（`#[serde(default)]`）。
- 後から入れる・置き換える（`State::fill_latest_event`）: 最新の版と同じ `cid` で署名の正しいイベントを受け取ったとき、`event` の無い版には `created_at` が版の `created_at` 以上のものを、`event` のある版にはそれより NIP-01 の置き換え規則で新しいもの（作者が同じ `cid` を後で署名し直したもの）を入れる。`withdrawn` に当たるものは入れない。置き換えたときは `withdrawn` の `events` を空にする（`through` は残す）。呼ぶのは過去分の取得（[`../agent.md`](../agent.md#全体の流れ)の 4 の 4）・保存の順序の 1（[`store.md`](store.md#保存の順序)）・送り直しの確認（[`resend.md`](resend.md#手順)の 4）。過去分の取得で選ぶのはサイトごとに relay にある最新の版なので、relay にその `cid` の版が無い、または別の `cid` の新しい版がある間は埋まらない。
- `event` の `created_at` は版の `created_at` より大きくなり得る。保存ポリシーの判定と保持期間（`agent::store::version_infos`）では、版の時刻として大きい方（`VersionRecord::announced_at`）を使う。
- 使い道は送り直し（[`resend.md`](resend.md)）と、一覧で relay の答えを補うこと（[`../nostr.md`](../nostr.md#検証) の `State::listed_site_events`、[`../mirror.md`](../mirror.md#unfollow-の一覧) の unfollow の一覧）。

## `withdrawn`

作者が取り下げたことを確かめたサイトの記録。送り直すかだけに使い、内容のミラーと一覧には使わない。版のイベント（`id`・`created_at`）は、`id` が `events` にあるか、`created_at` が `through` 以下なら取り下げに当たる（`Withdrawn::covers`）。

- 記録する契機（`State::record_withdrawal`。`events` は足し、`through` は大きい方を残す）:
  - 最新の版より新しい、`cid` の違う版を受け取ったとき（保存の順序の 1 の `State::note_newer_version`、送り直しの確認の 4）。記録の形は `Withdrawn::superseded`: 新しい版の `created_at` が保存している版より大きければ `through` を新しい版の `created_at` − 1 に、同じ秒なら保存している版の `id` を `events` に入れる（同じ秒を `through` にすると新しい版まで当たる）。保存している版に `event` が無ければ、新しい版の `created_at` が版の `created_at` より大きいときだけ `through` を新しい版の `created_at` − 1 にする。
  - 作者の NIP-09 の削除要求を見つけたとき（送り直しの確認の 7）。`e` タグなら版の `id` を `events` に、`a` タグなら削除要求の `created_at` を `through` に入れる。
- 記録するのは `sites` にあるサイトだけで、サイトが `sites` から消えると（unfollow、または保持期間・突き合わせですべての版が消えたとき）一緒に消す。
- そのサイトの最新の版より `created_at` が大きい版を記録すると（`State::apply_store`）、または最新の版の `event` を後の署名に置き換えると（`State::fill_latest_event`）、`events` を空にし、`through` は残す。両方無くなればキーごと消す。
- 空の `events`・無い `through` は書かない。取り下げの無い state では `withdrawn` 自体を書かない。読むときは無くてもよい（`#[serde(default)]`）。
