# 2026-09-22 サイト一覧の容量を実測値優先にする

## きっかけ

自己申告のイベント内容を信用しすぎている箇所の監査（`docs/todo.md` の該当行）で見つかった。`web/sites.js` のカードとテーブルは、サイトイベントの `size` タグ（作者の自己申告）を `formatBytes` でそのまま出しているだけで、実測値かどうかの区別が無かった。`mirror::apply_site_event`（`src/agent/store.rs`）は保存に成功した版ごとに `dag_size_local` で測った値を `VersionRecord.size` として `state.json` に記録しており、`mirror::collect_sites`（`src/mirror.rs`）はどのみち `state.sites.get(&key)` を引いて `stored` 判定をしているので、対応する `VersionRecord` の `size` を取り出すのは追加の Kubo 呼び出しなしの安いルックアップで済む。

[実容量の計測](2026-09-22-actual-storage-size.md)で入れた Storage check の実測値（`dag/stat` に全版を渡した重複排除後の合計）とは別物。あちらはサイト単位で DAG を毎回たどる重い計測で、サイトをまたいだ合計まで出すのが目的。今回のは state に載っている 1 件の `VersionRecord.size` を読むだけで、現在のバージョン 1 件についての「その版が実際に何バイトだったか」を示す。

## 決めたこと

- 現在の CID が保存されている（`state.json` に `cid` の一致する `VersionRecord` がある）なら、その版の実測サイズをそのまま数値で出す。
- 保存されていなければ、自己申告の `size` タグを括弧書き（例 `(12.3 MB)`）で出す。ラベルは付けない。同じ行に出る `[not stored]` バッジがすでに「未確認である」ことを示しているので、`(declared)` のような文言を重ねると冗長になる。
- どちらも無ければ従来通り `-`（CLI）/ `–`（web）。
- Storage check の重複排除込みサイト単位合計（`/api/status` の `sites[].actual`）はそのまま残す。今回のルックアップは 1 版だけの実測値で、版をまたいだ共有ブロックの重複排除はしていない。

## やったこと

- `src/mirror.rs`: `SiteRow` に `stored_size: Option<u64>` を追加。`collect_sites` で、サイトイベントの `cid` と一致する `VersionRecord` を state から探し、見つかれば `.size` を `stored_size` に入れる（`stored` 判定と同じルックアップを 1 回にまとめた）。`Unfollowed but still stored` の行は元から state の `VersionRecord` そのものなので `stored_size` も同じ値にする（`stored` は元から常に true）。`format_site_line` の `size=` 列は `stored_size` があればそれを数値で、無ければ `size`（申告値）を括弧書きで、どちらも無ければ `-` で出す（`format_size_column`）。
- `src/dashboard/dto.rs`: `SiteDto` に `stored_size` を追加し `SiteRow::stored_size` をそのまま渡す。`/api/publish/sites`（`PublishSiteDto`）は既存の設計どおり state.json を読まないエンドポイントなので対象外（下記「見送ったこと」）。
- `web/util.js`: `formatSiteSize(site)` を追加（`stored_size` があれば `formatBytes`、無ければ `size` を `formatBytes` して括弧で包む、どちらも無ければ `–`）。
- `web/sites.js`: カードのメタ行とテーブルの Size 列を `formatBytes(site.size)` から `formatSiteSize(site)` に差し替え。`Unfollowed but still stored` も同じ関数を共有しているので自動的に揃う。Storage check の実測合計表（`renderStatusCheck` 内、`site.actual` を使う箇所）は対象外なのでそのまま。
- `docs/architecture/cli.md`（`sites` サブコマンド）・`docs/architecture/dashboard/web.md`（Sites 画面）・`docs/architecture/dashboard/http-api.md`（`GET /api/sites` と `GET /api/publish/sites`）を更新。`docs/architecture.md` は `SiteRow`/`SiteDto` のフィールド一覧を持っていないので変更なし。
- `docs/todo.md` から該当行を削除。

## 検証

- `src/mirror.rs` に `format_size_column` の単体テストを 3 件追加（実測優先・申告値の括弧書き・両方無しで `-`）。
- `cargo fmt` / `cargo clippy -j 3 --all-targets -- -D warnings` / `cargo test -j 3` を通した。JS 側の lint/format 設定（package.json・Makefile・justfile・CI）はリポジトリに無いので実行していない。
- `web/desktop.js` と `web/webring.js` は `.size` でサイト行を表示している箇所が無いことを確認した（`web/desktop.js` はウィンドウのジオメトリの `size`、`web/webring.js` はダイアログの `size: 'normal'` で無関係）。`docs/architecture/dashboard/web.md` の Desktop 画面の節にも「npub・cid・size・replicas は表示しない」と明記されている。

## 見送ったこと

- `web/publish.js` の「My sites」（`buildMySiteEntry`、`/api/publish/sites` から取得）にも `formatBytes(site.size)` があるが、このエンドポイントは `http-api.md` に「state.json は見ない」と明記された既存の設計で、`stored`/`stored_size` の概念自体が無い。今回のルールは「`state.json` にある `VersionRecord` と照合できるか」が前提なので、この設計を変えずに適用することはできない。エンドポイントに state 参照を足すかどうかはオタクくんの判断を仰ぎたいので、今回は手を付けず `formatBytes(site.size)` のまま残した。
- README のスクリーンショット `docs/assets/dashboard-desktop.png` は `#/desktop` 画面のもので、Desktop 画面はそもそもサイズを表示しないため、今回の変更で見た目は変わらない。撮り直していない。
