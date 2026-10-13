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
- `sites[].event` は、その版を保存したときの署名済みのサイトイベント（relay から受け取ったもの）。サイトごとに最新の版（`created_at` が最大の版）だけが持ち、新しい版を記録すると（`State::apply_store`）ほかの版からは消す。無い版ではキーごと書かない。読むときは無くてもよく（`#[serde(default)]`）、この項目を持たない形式で保存した版は、過去分の取得で同じ `cid`・`created_at` のイベントを受け取ったときだけ埋める（[全体の流れ](../agent.md#全体の流れ)の 4 の 4）。過去分の取得で選ばれるのはサイトごとに relay にある最新の版なので、relay にその版が無い、またはそれより新しい版がある間は埋まらず、埋まらない版は送り直さない。使い道は「[保存した版のサイトイベントの再送](resend.md)」。
- `withdrawn` は、作者が取り下げたことを確かめたサイトの記録（[保存した版のサイトイベントの再送](resend.md#手順)の 4 と 7）。`events` は取り下げられた版のイベントの `id`（NIP-09 の削除要求の `e` タグ、または同じ秒の新しい版での置き換え）、`through` は取り下げられた範囲（その `created_at` 以下の版。削除要求の `a` タグ、または新しい版での置き換え）。送り直すかだけに使い、内容のミラーには使わない。記録するのは `sites` にあるサイトだけで、サイトが `sites` から消えると（unfollow、または保持期間・突き合わせですべての版が消えたとき）一緒に消す。空の `events`・無い `through` はキーごと書かず、取り下げの無い state では `withdrawn` 自体を書かない。読むときは無くてもよい（`#[serde(default)]`）。
- `status` は `verified` / `mismatch` / `not_applicable` / `error`。`verifications` の扱いは [`../nip05.md`](../nip05.md)。
- `follow_set` は最後に保存した署名済みの Follow Set。無ければ `null`（キーが無くても `null`）。
- `sites` と `verifications` は必須キー。ファイルが無い、または空白だけなら空の state として扱う。
