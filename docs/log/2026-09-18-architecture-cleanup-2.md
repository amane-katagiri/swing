# 2026-09-18 architecture の再整理と分割

## 問題

ダッシュボードの追加で `docs/architecture/dashboard.md` が 458 行になり、UI 調整の経緯、理由の説明、serde 属性や内部関数名などの実装の細部、CLI・設定サンプルと重複する記述が混ざっていた。`docs/architecture.md` も CLI 節が 100 行あり、索引として読みにくかった。architecture 全体で 1055 行。

## 決めたこと

| 決定 | 理由 |
|---|---|
| `architecture.md` の各サブコマンドの説明を `architecture/cli.md` に出し、`architecture.md` には一覧と設定ファイルの探索順だけ残す | `architecture.md` を構成・設定・検証・テストと索引に絞る |
| `cli.md` の冒頭に `<key>` の形式、Follow Set の定義、読み取り専用の範囲をまとめ、各節の繰り返しを削る | 同じ括弧書きが 4 か所にあった |
| `dashboard.md` を概要・起動と終了・設定・ガード・タイムアウト・静的ファイルに絞り、`dashboard/http-api.md` と `dashboard/web.md` に分ける | API リファレンスと画面の説明が大半を占めていた |
| ダッシュボードの文書から、UI 調整の経緯（撤去した UI、やめたフォントやアニメーション、揃えた訳語）、理由の説明（SSRF 対策、422 の読み替えの理由、上限値の目的）、外から観測できない実装の細部（一時ディレクトリ名の作り方、413 の判定方法、serde 属性）を削る | 経緯と理由は `2026-09-18-dashboard.md` にある。architecture は結果だけ書く |
| API の説明は CLI と同じ部分を `cli.md` へのリンクにし、差分だけ書く。JSON 例は形が分かる最小限にする | CLI 節と同じ集計の説明を繰り返していた |
| `web.md` は CSS 変数・class・data 属性・localStorage キーなど、カスタマイズする側が依存できるものを残し、レイアウトの叙述と `graph.js` の計算の書き下しを削る | コードを読めば足りる |
| `[dashboard]` のキー・環境変数・既定値の表を `dashboard.md` から削り、`architecture.md` の設定サンプルを正本にする。`listen` と `gateway` の制約は `dashboard.md` を正本にし、`architecture.md` からはリンクする | 同じ表と注記が両方にあった |
| `agent.md` の「シグナルと終了」から `tokio::select!` の性質の説明などを削り、挙動だけにする | 理由は `2026-09-18-dashboard.md` にある |
| `architecture.md` のテスト節からダッシュボードのテスト項目の列挙を削る | テストコードを読めば足りる |

architecture 全体で 927 行（`dashboard.md` 系は 458 → 322 行、`architecture.md` は 263 → 189 行）。AGENTS.md の文書の役割表と、README の `status` の参照先を合わせて更新した。

## 検証

- 整理前に、`architecture.md`・`agent.md`・`kubo.md`・`nip05.md`・`docker.md` を `src/`・`Dockerfile`・`compose.yaml`・`tests/` と、`dashboard.md` を `src/dashboard/`・`web/` と照らし合わせた。食い違いは `nip05.md` が `swing publish` の位置引数 `DIR` を `--dir` と書いていた 1 か所だけで、直した。
- README・AGENTS.md・`docs/` 配下（`plan.md` と `log/` を除く）の相対リンクとアンカーを機械的に確かめ、切れているものが無いことを確認した。
