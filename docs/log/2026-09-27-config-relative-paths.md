# 設定ファイルの相対パスを設定ファイルのディレクトリから解決する

## 背景

macOS の動作確認ワークフロー（[2026-09-27-macos-check-workflow.md](2026-09-27-macos-check-workflow.md)）で、`swing stop --config <path>` を別のディレクトリから実行すると `missing or invalid dashboard token or session` で失敗した。設定ファイルの `state_dir = "./data"` がプロセスのカレントディレクトリから解決され、動いている `swing up` とは別のディレクトリの `dashboard.token` を読んでいたため。`swing service install` はサービスの作業ディレクトリを設定ファイルの親にすることでこれを避けていたが、CLI を手で叩くときは設定ファイルのディレクトリに `cd` しないと同じ設定でも別の状態を見ることになる。

## 決めたこと

| 決定 | 理由 |
|---|---|
| 設定ファイルがあるときは、パスの相対パス（書いた値も既定値も）を設定ファイルのあるディレクトリを起点にする | 設定ファイルが同じなら、どこから実行しても同じ `state_dir` を指すようにする。対象はカタログの種類が `Path` の 8 キー（`[agent].state_dir`・`[kubo].binary`・`[kubo].repo`・`[dashboard]` の `custom_css`・`desktop_page`・`desktop_page_css`・`desktop_banner`・`mascots_dir`） |
| 環境変数で渡したパスと、設定ファイルが無いときの既定値はカレントディレクトリのまま | 環境変数はシェルやプロセスの側で与えるもので、設定ファイルとは結びつかない。既定値も最初は書いた値だけにしたが、`state_dir` を書いていない設定を `--config` で別のディレクトリから指すと同じ失敗が起きるので、設定ファイルがあれば既定値も寄せることにした。設定ファイルが無い構成（Docker など）では起点が無いのでカレントディレクトリのまま |
| 他の値から導く既定値（`[kubo].repo` の `<state_dir>/kubo`）は解決後の `state_dir` に従う | `state_dir` を先に解決してから導けば、特別扱いは要らない |
| 起点は設定ファイルのパスを `std::path::absolute` で絶対パスにした親 | `--config` が相対でも、読み込んだ時点のカレントディレクトリで固定し、後で作業ディレクトリが変わっても結果が変わらないようにする。`canonicalize` はシンボリックリンクを解決してしまうので使わない |
| 設定の書き換え（設定編集・セットアップ・ポートの固定）は TOML の文書をそのまま編集し、解決後のパスを書き戻さない | ファイルには利用者が書いたとおりの値を残す。パスの項目はダッシュボードから編集できないので `settings::raw_value` にも入れない |
| 古い解決方法への互換の処置は入れない | 公開前で、常駐の登録（`swing service install`）はもともと設定ファイルのディレクトリを作業ディレクトリにしていたので、その構成では結果が変わらない。Docker Compose はパスを環境変数の絶対パスで渡しているので影響が無い |
| `swing service install` の作業ディレクトリは設定ファイルの親のまま | 相対パスのためには不要になったが、変える理由も無い |

## 作ったもの

- `config::build::build_config` に起点のディレクトリ（`Option<&Path>`）を渡し、出どころが環境変数（`Source::Env`）でない相対パスを起点に連結する（`rebase_file_path`）。先頭の `./` は落とす。
- `Config::load` は設定ファイルが存在するときだけ起点を渡す。`build_config_from_str`（設定の書き換え前の検証とテスト用）は起点を渡さない。
- `swing.example.toml` の先頭に相対パスの起点を説明する 1 行を足した（`render_toml_example` の出力）。
- architecture（設定と環境変数・service・release・`GET /api/config` の `value`）と README を更新した。

## 検証

- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace` が通る。
- 追加したテスト: 設定ファイルを一時ディレクトリに置いて `Config::load` すると `state_dir`・`kubo.repo`・`custom_css` がそのディレクトリ起点の絶対パスになる。`build_config` に起点を渡したとき、設定ファイル由来の相対パスと `state_dir` の既定値は連結され、環境変数由来の相対パスと絶対パスはそのまま、`kubo.repo` の既定値は連結後の `state_dir` の下になる。起点が無ければ相対パスは書いたまま。
- 手で確かめたこと: `swing.example.toml` の写しを置いたディレクトリとは別のディレクトリから相対パスの `--config` で `swing up` をセットアップモードで起動すると `dashboard.token` が設定ファイルの隣の `data/` にでき、さらに別のディレクトリから絶対パスの `--config` で `swing stop` すると止まった。ポートの固定で書き換えられた設定ファイルの `state_dir` は `"./data"` のままだった。
- `state_dir` を書いていない設定ファイル（`[dashboard].listen` だけ）でも、別のディレクトリから `swing up --config` で起動すると `data/` が設定ファイルの隣にでき、カレントディレクトリには何もできず、さらに別のディレクトリからの `swing stop --config` で止まった。
