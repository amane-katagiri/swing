# 残タスク

優先度は 高 / 中 / 低。出所は「plan §17」（初期計画の拡張項目）か「レビュー」（初期実装のレビュー・監査で出たもの）。

| 優先度 | タスク | 出所 |
|---|---|---|
| 高 | レプリカ報告イベント（kind 35981、`a` タグで 35980 を参照）を agent が保存後に publish し、サイトごとのレプリカ数を集計・表示する | plan §17 |
| 高 | NIP-46 remote signer 対応。秘密鍵を `.env` に置かずに済む構成にする | plan §12 |
| 中 | Follow Set を集計して相互フォロー関係を Webring グラフとして表示する | plan §17 |
| 中 | サイトイベント（35980）に NIP-31 `alt` タグを付ける | レビュー |
| 中 | NIP-05 の実 HTTP 経路の統合テスト（ローカル TLS エンドポイント相手、`#[ignore]`） | レビュー |
| 中 | `publish` の `--url` を省略可、`--site` を必須にして IPFS のみのサイトを publish できるようにする | レビュー |
| 中 | 取得に失敗した CID を覚えて指数バックオフで再試行する。今は poll ごとに同じ CID の取得を試み、そのたびに最大 `SWING_FETCH_IDLE_TIMEOUT` の間、並行枠を 1 つ使う | レビュー（DoS） |
| 低 | relay から取得するサイトイベントの件数上限。`fetch_events` は件数無制限で、30 秒のタイムアウトだけで抑えている | レビュー（DoS） |
| 低 | NIP-05 のアドレスフィルタで NAT64（`64:ff9b::/96`）や 6to4（`2002::/16`）に埋め込まれた IPv4 を判定する | レビュー |
| 低 | 「全履歴保持」オプション（`keep_versions` / `keep_days` を無制限にする明示的な設定） | plan §4 |
| 低 | private mode: WireGuard / Tailscale / private IPFS network を使う別モード | plan §17 |
| 低 | サブパス公開サイト向けに NIP-05 の代替検証（例: `<url>/.well-known/swing.json`）を検討 | レビュー |
