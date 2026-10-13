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
- `sites[].size` は `dag/stat` の `TotalSize`。
- `sites[].event` は、その版を保存したときの署名済みのサイトイベント（relay から受け取ったもの）。署名が正しいときだけ記録し、正しくなければ版だけを記録して `event` は書かない。サイトごとに最新の版（`created_at` が最大の版）だけが持ち、新しい版を記録すると（`State::apply_store`）ほかの版からは消す。無い版ではキーごと書かない。読むときは無くてもよく（`#[serde(default)]`）。最新の版の `event` は、過去分の取得（[全体の流れ](../agent.md#全体の流れ)の 4 の 4）・保存の順序の 1（[`../agent.md`](../agent.md#保存の順序)。購読と過去分の取得で届いたイベント）・送り直しの確認（[保存した版のサイトイベントの再送](resend.md#手順)の 4）で、同じ `cid` の署名の正しいイベントを受け取ったときに入れる（`State::fill_latest_event`）。入れるのは、`event` の無い版（この項目を持たない形式で保存した版を含む）には `created_at` が版の `created_at` 以上のもの、`event` のある版にはそれより NIP-01 の置き換え規則で新しいもの（作者が同じ `cid` を後で署名し直したもの）で、`withdrawn` に当たるものは入れない。置き換えたときは `withdrawn` の `events` を空にする（`through` は残す）。版の `created_at` は変えない（MFS のパス `<site>/<created_at>` の名前なので）。そのため `event` の `created_at` は版の `created_at` 以上になり得て、保存ポリシーの判定（`agent::store::version_infos`）では版の時刻として大きい方（`VersionRecord::announced_at`）を使う。過去分の取得で選ばれるのはサイトごとに relay にある最新の版なので、relay にその `cid` の版が無い、または別の `cid` の新しい版がある間は埋まらず、埋まらない版は送り直さない。使い道は「[保存した版のサイトイベントの再送](resend.md)」。
- `withdrawn` は、作者が取り下げたことを確かめたサイトの記録（[保存した版のサイトイベントの再送](resend.md#手順)の 4 と 7、および購読と過去分の取得で最新の版より新しい `cid` の違う版が届いたとき。[保存の順序](../agent.md#保存の順序)の 1。どちらも `State::note_newer_version` か `Withdrawn::superseded` で同じ形に記録する）。`events` は取り下げられた版のイベントの `id`（NIP-09 の削除要求の `e` タグ、または同じ秒の新しい版での置き換え）、`through` は取り下げられた範囲（その `created_at` 以下の版。削除要求の `a` タグ、または新しい版での置き換え）。送り直すかだけに使い、内容のミラーには使わない。そのサイトの記録している最新の版より `created_at` が大きい版を記録すると（`State::apply_store`）、または最新の版の `event` を後の署名に置き換えると（`State::fill_latest_event`）、`events` を空にし（置き換えられたイベントは送り直す対象にならないため）、`through` は残す。両方無くなればキーごと消す。記録するのは `sites` にあるサイトだけで、サイトが `sites` から消えると（unfollow、または保持期間・突き合わせですべての版が消えたとき）一緒に消す。空の `events`・無い `through` はキーごと書かず、取り下げの無い state では `withdrawn` 自体を書かない。読むときは無くてもよい（`#[serde(default)]`）。
- `status` は `verified` / `mismatch` / `not_applicable` / `error`。`verifications` の扱いは [`../nip05.md`](../nip05.md)。
- `follow_set` は最後に保存した署名済みの Follow Set。無ければ `null`（キーが無くても `null`）。
- `sites` と `verifications` は必須キー。ファイルが無い、または空白だけなら空の state として扱う。
