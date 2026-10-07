# インストールと起動

OS ごとの入れ方、設定ファイルの置き場所、サービス登録をまとめます。Docker Compose で動かす場合は [`docker.md`](docker.md) を参照してください。最短の手順は [README の「はじめかた」](../../README.md#はじめかた)を参照してください。

## 必要なもの

次のどちらかで動かせます。

- **バイナリで動かす場合**: `swing` バイナリと、IPFS ノードである Kubo のバイナリ（`ipfs`、v0.43.1）。`swing` は GitHub のリリースに Linux・macOS・Windows 向けのビルド済みアーカイブ（Kubo は含みません）があればそれを使い、無ければ自分でビルド（Rust 1.97 で `cargo build --release`）します。Kubo は[公式の配布ページ](https://dist.ipfs.tech/kubo/v0.43.1/)から取得します。`ipfs` は `swing` と同じディレクトリに置くか PATH に通しておけば、`swing up` が自動で見つけます
- **Docker Compose で動かす場合**: Docker と Docker Compose（`docker compose` コマンドが使えること）

常時起動のサーバでも普段使いの PC でも動かせます。使い方・スペック・通信量の目安は「[動かし方の目安](operation.md#動かし方の目安)」を参照してください。

どちらの場合も、次が必要です。

- Nostr の秘密鍵（nsec または hex 形式）。サイト保存・ミラー参加専用の鍵を新しく作ることをおすすめします。作り方はいくつかあります。
  - `swing key generate`（バイナリを直接使う場合。設定ファイルが無くても動きます）
  - `docker compose run --rm mirror key generate`（Docker Compose を使う場合。`.env` が無くても動きます）
  - `nak key generate`（[nak](https://github.com/fiatjaf/nak) の CLI。出力は hex の秘密鍵なので、そのまま `SWING_NOSTR_SECRET_KEY` に入れられます。`nak key public <hex>` で公開鍵を得られます）
  - Nostr クライアントで新規アカウントを作り、設定画面から nsec を書き出す（例: Damus、Amethyst、noStrudel、Nostur）
  - 秘密鍵をこのコンピュータに置きたくない場合は、代わりにスマホの署名アプリ（NIP-46）を使えます。セットアップ画面で QR コードを読み取るだけで、秘密鍵は署名アプリから出ません（「[署名アプリ（NIP-46）で署名する](security.md#署名アプリnip-46で署名する)」）
- 自分のサイトも公開したい場合は、そのビルド済み静的サイトのディレクトリ（例: `./public`）。あわせて、ルートを自分で管理しているドメインがあると NIP-05 で本人確認ができます（必須ではありません）

## 設定ファイルとデータの置き場所

`swing` は `--config` → 環境変数 `SWING_CONFIG` → カレントディレクトリの `swing.toml`（あれば）→ 次のユーザーごとの既定の場所の `swing.toml` の順で設定ファイルを決めます。状態ファイルと Kubo のリポジトリは、既定では設定ファイルの隣の `data` に置かれます。

- Linux: `~/.local/share/swing`（`XDG_DATA_HOME` を設定していれば `$XDG_DATA_HOME/swing`）
- macOS: `~/Library/Application Support/swing`
- Windows: `%LOCALAPPDATA%\swing`

この文書のインストールスクリプト・インストーラー・Homebrew で入れた場合も、`swing service install` で登録した場合もここを使います。アンインストールしても（`install.sh` の `--purge` を除き）ここは消えません。詳しくは [`docs/architecture/config.md`](../architecture/config.md#設定ファイルの場所) を参照してください。

## セットアップモード

`swing.toml` を用意せずに `swing up` を起動することもできます。設定ファイルが無い（かつ環境変数にも鍵が無い）状態で起動すると、ダッシュボードだけが動く「セットアップモード」になります。ダッシュボードにログインする（「[ダッシュボード](usage.md#ダッシュボード)」）とセットアップ画面が表示され、鍵の生成（または既存の鍵の貼り付け、署名アプリとの接続）・relay・保存上限を入力して送信すると `swing.toml` が作られ（場所は「[設定ファイルとデータの置き場所](#設定ファイルとデータの置き場所)」。Docker Compose ならコンテナの `/data`＝`swing-data` volume）、エージェントはプロセスを終了させずにそのまま通常モードで動き直します。

セットアップモードでは、ダッシュボード（8082）や Kubo のゲートウェイ（8080）のポートがほかのプログラムに使われていると、近くの空いているポートにずらして `swing.toml` に書き込みます（ずらしたときはログに出ます。環境変数で指定したポートはずらしません。ずらしたくなければ `swing up --no-port-shift` か環境変数 `SWING_NO_PORT_SHIFT=true`。Docker イメージでは最初から有効で、ずらしません）。

設定ファイルを事前に用意して起動しても構いません（「[バイナリで動かす](#バイナリで動かす)」「[Docker Compose で動かす](docker.md)」）。設定は後からダッシュボードの Settings 画面（環境変数で設定した値を除く）からも変更でき、変更後は再起動すると反映されます。詳しくは [`docs/architecture/dashboard.md`](../architecture/dashboard.md) と [`docs/architecture/up.md`](../architecture/up.md) を参照してください。

## Linux ではインストールスクリプトで入れる

Linux（x86_64・aarch64）では、リリースの `swing` と Kubo をまとめて入れるスクリプトを使えます。ビルドも Kubo の取得も要りません。

```bash
curl -fsSL https://github.com/amane-katagiri/swing/releases/latest/download/install.sh | sh
```

バージョンを指定するときは `sh -s -- --version v0.1.0` とします。`swing` のアーカイブと Kubo は SHA-256 / SHA-512 のチェックサムを確かめてから入れます。`~/.local/lib/swing/` に `swing`・`ipfs` などを置き、`~/.local/bin/swing` からのシンボリックリンクを作ります（`~/.local/bin` が PATH に無ければ警告します）。同じコマンドをもう一度実行すると、その場で更新します。systemd のユーザーサービスとして動いていれば、止めてから入れ替えて、動いていたなら再び起動します。

- `--service` を付けると、入れた後に `swing service install` まで行います。設定ファイルが無ければセットアップモードで起動するので、`swing dashboard open` でダッシュボードを開いて設定を進めます。
- `--prefix DIR` で入れる場所を変えられます（`DIR/lib/swing` と `DIR/bin`）。システム全体に入れるなら `curl -fsSL .../install.sh | sudo sh -s -- --prefix /usr/local` とします。

削除は `~/.local/lib/swing/swing-uninstall.sh` です（`--prefix` で入れたときは、その `lib/swing` の中の同名のスクリプト）。サービスの登録（`lib/swing` の `swing` を起動するものだけ）と入れたファイルを消し、[設定ファイルとデータ](#設定ファイルとデータの置き場所)は残します。それも消すときは `--purge` を付けます（確認を求められます。`--yes` で省けます）。詳しい動作は [`docs/architecture/install-sh.md`](../architecture/install-sh.md) を参照してください。

## Windows のインストーラーで入れる

Windows では、GitHub のリリースにある `swing-<版>-x86_64-pc-windows-msvc-setup.exe` を実行するだけで入れられます。管理者権限は要りません（ユーザーごとのインストールで、UAC の確認は出ません）。Kubo（`ipfs.exe`）も同梱しているので、別に用意する必要はありません。

- インストーラーには署名をしていないので、ブラウザで落としたものを開くと Microsoft Defender SmartScreen の「Windows によって PC が保護されました」が出ます。「詳細情報」を押すと出る「実行」で進めてください。
- `%LOCALAPPDATA%\Programs\SWING` に入り、ターミナルから `swing` で実行できるよう、ユーザーの環境変数 `Path` にこのフォルダーを足します（開いていたターミナルには反映されないので、開き直してください）。
- サインイン時に SWING とタスクトレイのアイコンが起動するよう登録します（`swing service install --no-start` と同じ）。インストールの最後に「SWING を起動してダッシュボードを開く」を選ぶと、その場で起動してダッシュボードが開き、セットアップ画面から始められます。選ばなければ次のサインインで起動します。
- 新しい版のインストーラーをそのまま実行すれば上書きで更新できます。動いていた SWING はいったん止めて、更新後に起動し直します。サインイン時の起動を別の場所の `swing.exe` で登録し直していた場合は、その登録には触れません。
- 削除は「設定」→「アプリ」→「インストールされているアプリ」の SWING から行います。このフォルダーの SWING を起動するサインイン時の登録も消えます（別の場所から登録したものは残し、最後に知らせます）。[設定ファイルとデータ](#設定ファイルとデータの置き場所)（`%LOCALAPPDATA%\swing`）は残るので、不要なら手で削除してください。
- コマンドラインでは `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART` で確認なしにインストールでき（この場合は SWING を起動しません）、アンインストールも `"%LOCALAPPDATA%\Programs\SWING\unins000.exe" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART` で行えます。
- winget での配布は準備中です。

インストーラーの動作の詳細は [`docs/architecture/installer-windows.md`](../architecture/installer-windows.md) を参照してください。

## macOS で Homebrew から入れる

macOS では Homebrew で `swing` と Kubo（`kubo`）をまとめて入れられます。

```bash
brew install amane-katagiri/swing/swing
swing service install
swing dashboard open
```

`swing service install` は、設定ファイルがまだ無ければ[既定の場所](#設定ファイルとデータの置き場所)（`~/Library/Application Support/swing`）に空の `swing.toml` を作り、`swing` とメニューバーのアイコン（`SWING.app`）をログイン時に起動するよう登録して、その場で起動します。`swing.toml` のあるディレクトリで実行するとその設定ファイルが使われるので、ホームディレクトリなどで実行してください。最初はセットアップモードで動くので、`swing dashboard open`（またはメニューバーのアイコンの「ダッシュボードを開く」）で開いたセットアップ画面で鍵と設定を入力します。ログは `~/Library/Logs/swing.log` です。

- `brew upgrade` の後も、動いている `swing` は古いバージョンのままです。`swing service install` をもう一度実行すると新しいバージョンで起動し直します（`swing stop --restart` では入れ替わりません）。
- アンインストールするときは、先に `swing service uninstall` で登録を消してから `brew uninstall swing` します。設定とデータは残るので、要らなければ手で消してください。
- Homebrew の Kubo のバージョンが `swing` の想定（v0.43.1）と違うときは、起動時に警告が出ますがそのまま動きます。

入る場所と `brew upgrade` での振る舞いは [`docs/architecture/homebrew.md`](../architecture/homebrew.md) を参照してください。

## バイナリで動かす

GitHub のリリースにビルド済みアーカイブがあればそれを展開して使います。無ければ `swing` バイナリをビルドします（Rust 1.97 が必要です）。

```bash
cargo build --release
```

`target/release/swing`（Windows は `swing.exe`）ができます。あわせて Kubo v0.43.1 を[公式の配布ページ](https://dist.ipfs.tech/kubo/v0.43.1/)から取得し、中の `ipfs`（Windows は `ipfs.exe`）を `swing` と同じディレクトリに置くか、PATH に通してください。

秘密鍵を生成します。

```bash
./target/release/swing key generate
```

設定ファイルを用意します。

```bash
cp swing.example.toml swing.toml
```

`swing.toml` を開き、少なくとも次を書き換えてください。

- `[nostr] secret_key`: 生成した鍵の nsec または hex
- `[nostr] relays`: 実際に使う Nostr relay
- `[policy] max_total_storage`: 保存する全サイト合計の容量上限

既定では `[kubo] managed = true` になっていて、`swing up` 自身が Kubo を子プロセスとして初期化・起動します。状態ファイルは `[agent] state_dir`（既定 `./data`。設定ファイルがあれば、相対パスは設定ファイルのあるディレクトリが起点）に、Kubo のリポジトリはその下の `kubo`（既定 `./data/kubo`）に置かれます。他の設定項目は「[設定一覧](operation.md#設定一覧)」を参照してください。

起動します。

```bash
./target/release/swing up
```

Kubo を初期化・起動し、その上で mirror-agent を動かします。ダッシュボードは `http://127.0.0.1:8082/`、Kubo のゲートウェイは `http://127.0.0.1:8080/` で待ち受けます。Kubo の RPC はループバックのランダムなポートで待ち受けるため外部からは触れず、公開する必要があるのは IPFS swarm 用のポート（`4001/tcp`・`4001/udp`）だけです（詳しくは「[プライバシーと鍵の扱い](security.md)」）。`swing up` を実行している間は、`swing mirror add` や `swing sites` などの他のサブコマンドも同じ設定ファイルを指定するだけで、管理下の Kubo を自動で見つけて使えます。

ログイン時に自動で起動させたい場合は、OS のサービスとして登録します。

```bash
./target/release/swing service install
```

Linux では systemd のユーザーユニット、macOS では launchd の LaunchAgent、Windows ではタスクスケジューラに登録します（Linux はログアウト後も動かし続けるために `loginctl enable-linger` を試み、失敗すれば案内を表示します）。Windows と macOS では、`swing` と同じフォルダに `swing-tray.exe`（macOS は `SWING.app`）があれば、タスクトレイのアイコン（「[ダッシュボード](usage.md#ダッシュボード)」）もログイン時に起動するよう登録し、その場で起動します。トレイが要らなければ `--no-tray` を付けてください。設定ファイルがまだ無いときに（`--config` も `SWING_CONFIG` も付けず、カレントディレクトリに `swing.toml` も無い状態で）実行すると、[ユーザーごとの既定の場所](#設定ファイルとデータの置き場所)に空の `swing.toml` を作ってから登録するので、サービスはセットアップモードで起動します。Linux で systemd のシステムユニットにしたいときは `sudo swing service install --system` とします（設定ファイルは先に用意しておきます）。サービスは `sudo` を実行したユーザーの権限で動きます（別のユーザーで動かすなら `--run-as <user>`。root で動かすには `--allow-root` も要ります）。登録の前に、`swing` の実行ファイル・設定ファイルとそのディレクトリ・Kubo のバイナリと、それらの親ディレクトリが、root かサービスのユーザーの持ち物で、ほかのユーザーやグループから書き込めないことを確かめ、そうでなければ問題のあるパスを示して止まります。別のユーザーや root で動かすなら、`swing` は `install.sh --prefix /usr/local` のように root の持ち物の場所に入れ、設定ファイルは `/etc/swing` のような場所に置いてください。状態確認は `swing service status`、起動は `swing service start`、停止は `swing service stop`、削除は `swing service uninstall` です。詳しくは [`docs/architecture/up.md`](../architecture/up.md) と [`docs/architecture/service.md`](../architecture/service.md) を参照してください。
