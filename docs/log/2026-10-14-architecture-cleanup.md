# architecture の整理（2026-10-14）

`docs/architecture/` のうち、機能の追加で肥大化したページ・ページ間の重複・理由や経緯の説明・一般論を整理し、20KB を超えた（または近い）ページを分けた。全ページをコードと照らし、見つけたずれも直した。整理の後は、作業を知らない新しい担当が評価し、クリティカルな指摘が無くなるまで直して評価し直した。コードは変えていない。

## 分けたページ

| 元 | 新しい子ページ | 中身 |
|---|---|---|
| `agent.md`（21KB） | `agent/store.md` | 取得した版の保存の順序・拒否の記録・取得の試行の間引き |
| `dashboard/views.md`（20KB） | `dashboard/views/webring.md`・`dashboard/views/settings.md` | Webring 画面とグラフ、Settings 画面 |
| `dashboard/web.md`（19.6KB） | `dashboard/web/ui.md` | 共通の UI 部品 |
| `dashboard/desktop.md`（18KB） | `dashboard/desktop/link-page.md` | リンク集ページと、差し替えるページが使える契約 |
| `service.md`（18KB） | `service/system-user.md` | `--system` の unit の実行ユーザーの決め方 |

外からのリンク（`AGENTS.md`・`docs/guide/usage.md` など）は子ページを直接指すように書き換えた。元のページの見出しのうち `web.md#共通の-ui-部品`・`desktop.md#リンク集ページiframe` は、親ページの中の位置を保つために子ページへの案内として残した。`desktop.md#コントロール-パネル` は過去のログが指しているので、案内の見出しとして残した。

## 決めたこと

- 記録の形の正本を寄せた。`state.json` の `sites[].event` と `withdrawn` の形は `agent/state.md`、`relays[].error` の切り詰めと `fingerprint` の形は `dashboard/http-api.md` の「形式とエラー」、ペアリングの条件と待ち時間は `signer.md`。ほかのページはリンクにした。
- 理由の説明は、守らないと実装を変える人が踏む制約（例: `#a` と `#e` を 1 つのフィルタにまとめると両方を満たすものしか返らない、`sandbox` を付けない）だけを 1 文で残し、ほかは削った。
- 設定ファイルの探す順は `config.md` だけに置き、`architecture.md`・`tray.md` はリンクにした。
- `architecture.md` の「テスト」節は索引の表と同じだったので消した。`format.rs` の無害化の規則は `cli.md#共通` へ移した。

## 直したコードとのずれ

- `architecture.md`: 設定ファイルを探す順。既定の場所が `./swing.toml` より先（`config::locate_config_with`）。
- `cli.md`: ログの既定の出力先は標準出力で、`publish --json` のときだけ標準エラー。`--log-file` の CLI の help（"instead of stderr"）はコードの挙動と食い違ったまま残っている。
- `webring.md`・`cli/views.md`: `webring::collect` と `replicas::collect` は relay に無いサイトイベントを補うために `state.json` を読む。「state は読まない」を消した。
- `mfs.md`・`nostr.md`: ディレクトリの確認（`files/stat`）は取得（`dag/export`）の前で、保存の順序の 4。
- `dashboard/web.md`・`desktop/link-page.md`: Desktop 画面に出すのは保存済みと更新待ちのサイトで、更新待ちを集めるのは `accounts` からだけ。
- `dashboard/http-api.md`・`http-api/publish.md`: `/api/publish/check` は `nip05` が `off` でなければ、セットアップモードの間だけ 503 `agent is not configured`。`agent is not ready` にはならない。
- `http-api/config.md`: ペアリングの `perms` を 3 種と書いていた（正しくは kind 1 を含む 4 種）ので、`signer.md` へのリンクにした。`secret_key` は空白だけでも新しい鍵を作る。
- `publish/dry-run.md`: 入口は `publish::dry_run::run`。`skipped` はパスの配列。
- `service.md`: 一時ファイルは `sync_all` してから rename する。
- `install-sh.md`・`release.md`・`homebrew.md`: `obtain_self` の条件、`bin/swing` がディレクトリのときの失敗、最後の案内、版の形式の検査、`check.sh` の強制 push と 5 分の待ち、`render.sh` の終了コード 2 など。
- そのほか、書かれていなかった挙動を足した（`nip05.md` の `/api/publish/check` と dry run、`mfs.md` の `only-hash`、`kubo.md` の `ipld: could not find` への依存、`testing.md` の `kubo_client_probe`、Webring 画面の表示倍率と depth の丸めなど）。

## 検証

- docs（log を除く）・README・AGENTS・skills・packaging の相対リンクと見出しのアンカーをスクリプトで確かめ、切れは 0 件。
- 20KB を超える architecture のページは無い。
- 評価では、差分で消えた行ごとに、今の挙動の事実が移し先に残っていることを確かめた。

## 残したこと

- `AGENTS.md` はリンク先を直しただけで、「リリース」「CI での確認」節は `release.md` と各スクリプトの中身が重なっている。`docs/mascot-guide.md` の「確かめ方」はパックの上限値を書き写している。どちらも architecture の外なので今回は整理していない。
