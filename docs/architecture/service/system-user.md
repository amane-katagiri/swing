# system unit の実行ユーザー（service/linux.rs）

[`../service.md`](../service.md) の一部。`swing service install --system`（Linux）が unit の `User=` を決め、unit が動かすものを実行ユーザー以外が差し替えられないことを確かめる手順。

`--system` の unit は root では動かさない。`install --system` は次の順に実行ユーザーを決め、passwd（`getpwnam`・`getpwuid`）で引いた名前を `User=` に書く。引けなければ `no such user: <name>`・`no user with uid <uid>` でエラー。

1. `--run-as <user>`（名前か数字の uid）。引いた uid が 0 なら、`--allow-root` も付けないとエラー。
2. 環境変数 `SUDO_UID`。`0` のときは使わない。数字でなければエラー。
3. 実行している uid（root でなければ）。
4. どれでもなければ `refusing to register a system service that runs swing as root: ...` でエラーにし、unit を書かない。

続けて、実行ユーザー以外が unit の動かすものを差し替えられないことを確かめる。対象は実行ファイル（登録するパスと、シンボリックリンクを解決したパス）・設定ファイル・作業ディレクトリと、設定を読めて `[kubo].managed` が true なら Kubo のバイナリ（`[kubo].binary`、無ければ `kubo::locate_binary` が見つけたもの。見つからなければ確かめない）。それぞれ自身と祖先のディレクトリすべてが次をみたさなければ、問題のあるパスを並べた `refusing to register a system service that runs as <name>: ...` でエラーにし、unit を書かない。

- 所有者が root か実行ユーザー（シンボリックリンクそのものは所有者だけを見る）。
- その他のユーザーが書き込めない。グループの書き込みは、そのグループが所有者の個人グループ（名前が所有者のユーザー名と同じで、ほかのメンバーがいない）のときだけ許す。持ち主が root でも許さないので、Debian の `root:staff 2775` の `/usr/local` も断る。
- root が持つ sticky ビット付きの祖先のディレクトリ（`/tmp` など）は上の 2 つを問わない。ただし確かめる対象そのものと、実行ファイル・Kubo のバイナリを直接収めるディレクトリ（書かれたパスと解決したパスのそれぞれの親）にはこの例外を当てはめない。
- 調べられない（存在しないなど）ものもエラーに含める。

決めたユーザーは `The service runs as user <name>.` と表示する。`swing up` はそのユーザー（`HOME` もそのユーザーのもの）で設定ファイルと `state_dir` を読み書きし、Kubo のバイナリを実行するので、設定ファイルと `state_dir` はそのユーザーが書ける場所に置く。
