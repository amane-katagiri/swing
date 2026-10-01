# 設定ファイルのユーザーごとの既定の場所

## 背景

Windows（Inno Setup の GUI と winget）・Linux（`install.sh`）・macOS（Homebrew）向けのインストーラーを用意する。インストーラーで入れた人は特定のディレクトリで `swing` を実行するわけではないので、これまでの「`--config`・`SWING_CONFIG` が無ければ、無くても `<カレントディレクトリ>/swing.toml`」では、実行した場所ごとに別の設定ファイルとデータができてしまう。

## 決めたこと

- 探索順を `--config` → `SWING_CONFIG` → `<カレントディレクトリ>/swing.toml`（あるときだけ）→ ユーザーごとの既定の場所の `swing.toml`（無くても使う）にした。カレントディレクトリに `swing.toml` を置いて使ってきた人の動作は変わらないので、移行のための処置は入れていない。
- 既定の場所は Linux が `$XDG_DATA_HOME/swing`（無ければ `~/.local/share/swing`）、macOS が `~/Library/Application Support/swing`、Windows が `%LOCALAPPDATA%\swing`。
  - 設定ファイルの隣の `data` に Kubo のリポジトリが入り、既定の上限で 100GiB 規模になる。Linux で `~/.config` を選ばなかったのは、設定用のディレクトリにその量のデータを置くことになり、バックアップや dotfiles の管理に巻き込まれるため。
  - Windows で `APPDATA`（Roaming）を選ばなかったのは、ドメイン環境で移動プロファイルとしてサインイン・サインアウトのたびに同期されるため。
- 既定の場所が決まらないとき（`HOME`・`LOCALAPPDATA` が無い、絶対パスでない、既存のディレクトリでない）は、これまでどおり無くても `<カレントディレクトリ>/swing.toml` を使う。`XDG_DATA_HOME` は XDG の仕様どおり絶対パスのときだけ使い、存在は問わない（無ければ作る）。
- 既定の場所で設定ファイルが無いときも、状態ファイルの既定（`./data`）は設定ファイルのディレクトリを起点にする。セットアップモードの書き込みや Kubo のリポジトリが、実行した場所ではなく既定の場所の `data` に揃う。カレントディレクトリにフォールバックしたときの扱いは変えていない。
- `swing service install` は、パスが既定の場所に決まりファイルが無ければ、ディレクトリ（`0700`）と空の設定ファイル（`0600`）を作って登録する。インストーラーが何も無いマシンで `swing service install --no-start` を実行しておけば、次の起動でセットアップモードになる。`--config`／`SWING_CONFIG` で指したパスが無いときはこれまでどおりエラーにする（打ち間違いで空のファイルを作らないため）。`--system` でも作らない（`sudo` 下では root のホームが既定の場所になり、サービスを動かすユーザーが書けないため）。
- ダッシュボードの `writable` は、親ディレクトリが無いとき存在する最も近い祖先で判定するようにした。設定ファイルの書き込み（`settings::write_atomic`）はもともと親ディレクトリを `0700` で作るので、セットアップ・設定編集・ポートの書き込みはどれも既定の場所が無くても書ける。
- セットアップモードに入るときのログに、書き込み先の設定ファイルのパスを出すようにした。どこに書かれるか分かりにくくなったため。
- 既定の場所を決める関数は環境変数の読み取りと存在確認を引数で受け取り、テストが実際のホームディレクトリを読み書きしないようにした。依存クレート（`dirs` など）は Cargo.lock に無く、環境変数 3 つで足りるので足していない。

## Docker への影響

Docker イメージ（`Dockerfile`・`docker/release.Dockerfile`）のユーザー `swing` は `useradd --no-create-home` で作られ、`HOME` は存在しない `/home/swing` になる。既定の場所は決まらないので、これまでどおり `WORKDIR` の `/data/swing.toml` を無くても使い、セットアップモードもそこへ書く。`compose.yaml`・デモ環境・`tests/` は変更不要。

`SWING_CONFIG=/data/swing.toml` をイメージに固定する案は採らなかった。`SWING_CONFIG` で指したファイルが無いと起動エラーになるので、volume に `swing.toml` が無い状態でのセットアップモードが動かなくなる。

## 検証

- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace`・`cargo xwin clippy --workspace --target x86_64-pc-windows-msvc --all-targets -- -D warnings` が通ることを確かめた。
- 一時ディレクトリを `HOME` にして、`swing.toml` の無いディレクトリで `swing up` を起動し、`$HOME/.local/share/swing/swing.toml` を書き込み先としてセットアップモードに入ること、ポートの書き込みでディレクトリ（`0700`）と設定ファイルが作られ、`data` もその下にできることを確かめた。
- 空の `swing.toml` を `--config` で渡すとセットアップモードに入り、ポートが書き込まれることを確かめた。
- カレントディレクトリに `swing.toml` があればそれを使うことを確かめた。
- `HOME` を存在しないディレクトリにし、カレントディレクトリを状態ディレクトリにして起動すると、書き込み先が `<カレントディレクトリ>/swing.toml` になることを確かめた（Docker イメージと同じ条件）。`debian:trixie-slim` で `useradd --no-create-home` したユーザーの `HOME` が存在しないことも確かめた。
- `swing service install` を実機で登録する確認はしていない。空の設定ファイルを作る処理は単体テストで確かめた。
