# 2026-09-24 NIP-46 の署名アプリ対応

秘密鍵を `.env` や `swing.toml` に置かずに済むように、NIP-46 の署名アプリ（remote signer）に署名をリクエストできるようにした回（[`docs/plan.md`](../plan.md) §12、todo の「高」）。現状の仕組みは [`architecture/signer.md`](../architecture/signer.md)。

## 決めたこと

- 使うかどうかはセットアップ画面でユーザーが選ぶ。「新しい鍵を生成する」「既存の鍵を使う」に並べて「スマホの署名アプリで署名する（NIP-46）」を置き、選んだときだけ仕組みと注意点を説明する。
- 接続はスマホ前提で、SWING が `nostrconnect://` の QR コードを出し、署名アプリがそれを読む方式だけにした。署名アプリが作った `bunker://` を貼る方式は todo に回した。
- レプリカ報告（kind 35981）は agent が数日おきに自動で署名する必要があり、人が毎回承認することはできない。そこで次の 2 案から選んだ。
  - 全部の署名を署名アプリに任せ、署名アプリ側で kind 35981 を自動許可にしてもらう（採用）。鍵は 1 つのままで、protocol は変えなくてよい。署名アプリが落ちていると報告の出し直しが止まり、`expiration`（既定 3 日）で数えられなくなるが、壊れ方はおだやか。
  - 報告だけローカルの別鍵で署名する: 署名アプリが落ちていても報告できるが、protocol §8 は報告者が作者か受信側のミラー対象リストに入っているかで信頼度を分けているので、別鍵の報告は未検証扱いになる。本当の鍵から報告用の鍵への委任を protocol に足す必要があり、重いので見送った。
- 自動許可は NIP-46 の `perms` で「リクエスト」するしかない（仕様は自動承認を義務付けていない）。`perms` は常に `get_public_key,sign_event:35981`。公開（35980）とミラー対象の変更（30000）はチェックを入れたときだけ足し、既定では署名アプリで毎回承認してもらう。自動許可になったかどうかは要求しただけでは分からないので、接続直後に kind 35981 の空のイベントの署名を 1 回試し（probe）、通らなければ画面で署名アプリの設定を促す。probe の署名は relay に送らない。
- probe は、保存するのと同じ内容から接続を作り直して行う。再起動後に agent がつなぎ直す手順と同じなので、再接続できることもここで確かめられる。
- 再接続（`NostrConnect` が bunker URI で送る `connect`）には、ペアリング時の secret を入れる。rust-nostr の `NostrConnectRemoteSigner` は secret を覚えていて、secret 無しの `connect` を不一致として拒否する（実機の署名アプリがどうするかは未確認）。
- 自分の公開鍵はペアリング時に受け取ったものを `remote-signer.json` に保存し、起動時に `non_secure_set_user_public_key` で設定する。起動・購読・一覧系の CLI が署名アプリのオンライン状態に左右されないようにするため。その代わり、署名アプリが返したイベントは `pubkey`・`id`（リクエストした内容と同じか）・署名を毎回確かめる。
- 接続情報（アプリ鍵・署名アプリの公開鍵・relay・secret・ユーザーの公開鍵）は設定ファイルではなく `<state_dir>/remote-signer.json`（0600）に置いた。アプリ鍵は秘密なので `swing.toml` の編集 API（ホワイトリスト）や `/api/config` の表示と混ぜたくなかった。`secret_key` と両方あれば起動エラーにする。
- 署名アプリとのやりとりの relay は、`[nostr].relays` とは別に画面で指定する。既定は NIP-46 用の `wss://relay.nsec.app`。
- 署名の待ち時間は 90 秒（ユーザーの承認を含む）。ダッシュボードのリクエストタイムアウト 120 秒の内側に収めた。
- レプリカ報告の送信は、1 件でもエラーになったらその回を打ち切る。署名アプリがオフラインのとき、報告の件数ぶん 90 秒を待ち続けないようにするため。
- 最後の署名リクエストの失敗は `GET /api/overview` の `signer.last_failure` で返し、Publish 画面の「自分の情報」に警告として出す。
- 画面の文言では、署名アプリへの依頼を「お願い」ではなく「リクエスト」と書く。

## 作ったもの

- `src/signer.rs`: `Signer`（`Local(Keys)` / `Remote(Arc<RemoteSigner>)`）、`RemoteSignerFile`、`Pairing`、`nostrconnect_uri`・`requested_perms`・`qr_svg`。
- `RelayClient` は `Keys` の代わりに `Signer` を持ち、`sign`・`public_key`・`shutdown` を持つ。`Config::require_secret_key` は消し、各コマンドは `Signer::require` を使う。
- `swing up` は起動時に `Signer::load` を 1 回だけ呼び、`AppState.signer` に置いて agent に使い回させる。
- ダッシュボード: `POST`／`GET /api/setup/signer`、`POST /api/setup` の `remote_signer`、`/api/overview` の `signer`。セットアップ画面の選択肢・QR 表示・状態表示、Publish 画面の「署名」の行。
- 依存: `nostr-connect` 0.45、`qrcode` 0.14（SVG だけ）。dev-dependency で nostr-sdk の `local-relay` を有効にした。
- `auth::write_private_file` を切り出し、`dashboard.token` と `remote-signer.json` の書き込みで共有する。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`、`cargo xwin clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings` が通った。
- `signer::tests` で、プロセス内の relay（`LocalRelay`）と署名アプリ役（`NostrConnectRemoteSigner`）を相手に、QR 用 URI の発行 → 接続 → probe → `remote-signer.json` から読み直して署名、までを通した。署名アプリ役が `sign_event` を拒否すると `auto_report: false` で接続でき、`last_failure` が残ることも確かめた。
- `tests/nostr_relay_integration.rs`（`#[ignore]`）を `LocalRelay` 相手に流して通した。
- 手元で `swing up` をセットアップモードで起動し、ブラウザでセットアップ画面から QR を出して、署名アプリ役（`LocalRelay` を同じプロセスで動かす使い捨てのプログラム）に URI を読ませた。接続・probe の成功表示、`POST /api/setup` 後の再起動、`remote-signer.json`（0600）と `secret_key` の無い `swing.toml` ができることを確かめた。オフラインの Kubo（`ipfs/kubo:v0.43.1 daemon --offline`）をつないで、CLI の `swing publish`・`swing mirror add`（ダッシュボード API 経由）・agent のレプリカ報告の 3 つが署名アプリ経由で署名されることを確かめた。
- 署名アプリ役を止めて `swing mirror add` すると 90 秒でタイムアウトし、`signing mirror set event: the signer app did not answer in time; ...` で終了すること、Publish 画面に最後の失敗が警告として出ることを確かめた。

## まだ確かめていないこと

- 実機の署名アプリ（Amber など）との接続。`perms` の扱い、再接続の `connect`、`ws://` の relay を読めるかは todo に書いた。

## 追記: Amber の実機で確かめた

捨てアカウントを入れた Amber（Android）で、手元の `swing up`（Nostr の relay はローカル、Kubo はオフライン）とつないだ。署名アプリとのやりとりだけ公開 relay を通した。

- `wss://relay.nsec.app` には、こちらの回線から TCP でもつながらなかった（DNS は引ける）。ペアリングは「接続を待っています」のまま進まず、10 分のタイムアウトまで何も起きなかった。`wss://relay.damus.io` に変えると通った。
  - 対応: 15 秒たっても relay に 1 つもつながらなければ、すぐ失敗にするようにした（`RELAY_CONNECT_TIMEOUT`）。
  - 対応: 画面の relay の既定を `wss://relay.damus.io` に変えた。
- Amber は接続のときに `perms` を見せず、kind ごとの許可も出なかった。署名のリクエストには承認を求め、そのとき「常に許可」を選べる。後からアプリごとの設定で、イベントごとの許可や、kind を指定したカスタムイベントの許可ができる。
  - 確認の署名（probe）は、ユーザーがその場で承認して通っていた。それなのに、画面には「レプリカ報告は自動で署名されます」と出ていた。自動で許可されたのかその場で承認されたのかは、SWING からは見分けられない。
  - 対応: probe の結果は「確認の署名が通った」とだけ伝えるようにした（`auto_report` を `probe_signed` に改名）。承認を求められたら常に許可にするよう、確認中の表示と説明文で案内する。毎回確認したい人向けに、kind 35981 だけを許可する方法も案内する。承認する時間を見込んで、probe の待ち時間を 20 秒から 60 秒に延ばした。
  - 対応: 「公開とミラー対象の変更も自動署名の許可をリクエストする」のチェックを消した。`perms` は常に 3 種類（レプリカ報告・サイトイベント・Follow Set）をまとめてリクエストする。
- 「常に許可」にした後、ダッシュボードからの publish は Amber の通知なしで署名された。agent のレプリカ報告も送れた。`swing up` を再起動した後も署名できたので、再接続時の `connect`（ペアリング時の secret を再送する）も Amber で通っている。
- 案内が Amber に寄りすぎないよう、使える署名アプリの例を Android（Amber。Google Play にはなく、Zapstore・Obtainium・GitHub のリリースから入れる）と iPhone / iPad（nsec.app。ブラウザで動く）に分けて出すようにした。nsec.app の README によると、スマホがロックされていると応答しないことがある。そこで、iPhone では 1〜2 日に一度は署名アプリを開くよう注記した。QR は署名アプリの中の読み取りで読むこと、読めなければリンクを貼り付けることも案内する。
- iPhone の署名アプリとの組み合わせは、まだ確かめていない（todo）。

## 追記: Primal と、つなぎ直し・公開結果の表示

- 使える署名アプリを調べ直した。Primal は Google Play と App Store にあり、Android 版は 2.6.18（2026 年 1 月）で他のアプリ向けの署名役（NIP-46・NIP-55）に対応、iOS 版は「Remote Login」で `nostrconnect://` の QR を読めるとされている（iOS では試していない）。ほかに Aegis・Nowser（iOS は TestFlight）、Signeur（公開前）がある。画面と README の例に Primal を足した。
- Android の Primal で試した。接続・確認の署名（full trust に当たる選択肢があった）・公開はできた。
- Primal のアプリ一覧で SWING の接続を End Session → Start Session すると、以後の署名リクエストが `We don't accept connect requests with new secret.` で断られ続けた（Start Session しても戻らない）。NIP-46 は secret を使い回さないよう求めていて、Primal は終えた接続にペアリング時の secret で戻ることを許さない。Amber と rust-nostr の署名アプリ役は、再起動後の `connect` に同じ secret を受け付けるので、再接続で secret を送る今のやり方は変えていない。
  - 対応: 署名アプリで動いている間も `/api/setup/signer` でペアリングできるようにし、`POST /api/signer/reconnect` で `remote-signer.json` を書き換えて再起動するようにした。別のアカウントでつないだら断る。公開画面の「自分の情報」に「署名アプリとつなぎ直す」を置き、最後の署名リクエストが失敗していれば、つなぎ直しを促す一文を警告に足した。ペアリングの画面部品は `web/pairing.js` に切り出し、セットアップ画面と共有した。
  - todo の「つなぎ直し」を片付け、秘密鍵との切り替えだけを残した。Primal の接続がスマホの再起動などで切れるかは、まだ確かめていない（todo）。
- 公開が終わったあとの表示が分かりにくかった（成功の一言が無い、NIP-05 が `error — unreachable` のような内部の値、relay の結果に見出しが無い、進捗バーが残る）。成功時は「<site> を公開しました。relay N つのうち M つが受け付けました」（一部だけなら `warn`）を出し、結果に小見出しを付け、NIP-05 を文に言い換え、relay の結果を「relay」の行に入れ、進捗バーを隠すようにした。

## 追記: 再接続で `connect` を送らないようにした

- Primal で、つなぎ直した直後（確認の署名は通った）に再起動すると、最初の署名リクエストが `We don't accept connect requests with new secret.` で断られた。確認の署名のときに送った `connect`（ペアリングの secret 付き）は受け付け、同じ secret での 2 回目の `connect` を断っている。上の追記で「Primal は End Session のあと断る」と書いたのも、同じ原因だった可能性が高い。
- `nostrconnect://` でペアリングした後は、署名アプリはこのアプリを知っているので、そもそも `connect` は要らない。nostr-connect の `NostrConnect` は、bunker URI で使うと最初のリクエストの前に必ず `connect` を送る作りだった。
  - 対応: ペアリング（`nostrconnect://` の待ち受け）にだけ `NostrConnect` を使い、それ以後の署名リクエストは `RemoteSigner` が `nostr_sdk::Client` と nip46 の型（`NostrConnectMessage`・`NostrConnectEventBuilder`）で直接送るようにした。`connect` と `get_public_key` は送らない。ペアリングの secret は `remote-signer.json` に保存しないようにした。
  - テストの署名アプリ役は `connect` を断るようにして、SWING が `connect` を送ったらテストが落ちるようにした。
- 前の版（毎回 `connect` を送る）で Primal に断られた接続は、新しい版で再起動しても返事が来ず、タイムアウトになった。つなぎ直してから確かめる。

## 追記: 時計のずれ、Primal のセッション、載せる署名アプリ

- `RemoteSigner` の返事の購読を `since` 今にしていたため、つないだ直後の確認の署名の返事を取りこぼした。この WSL2 の時計は実際より約 23 秒進んでいて（Google と relay.primal.net の `Date` ヘッダと比べた）、Primal は返事の `created_at` をスマホの今にするので、リクエストから 23 秒以内の返事は `since` より前になって relay が渡さなかった。購読を `limit(0)`（開いたあとに届いたものだけ）に変え、返事の `created_at` が 10 分前でも受け取れるテストを足した（`since` に戻すと落ちることも確かめた）。
- Primal（Android）のソースを読んだ。署名役は、アプリの中で開始した「セッション」の間だけその relay につないでリクエストを聞き、セッションは 15 分リクエストが無いと終わる（`sessionInactivityTimeoutInMinutes = 15`）。終わると relay から離れるので、SWING のリクエストは届かず、SWING からは 90 秒のタイムアウトにしか見えない。既知のアプリからの `connect` は、secret が空なら ack、secret 付きなら `We don't accept connect requests with new secret.` を返す作りだった。Start Session してから公開すると通知が来て署名でき、Start / End を繰り返しても `connect` を送らない版では問題なかった。数日おきのレプリカ報告には答えられないので、Primal は例から外し、README に理由を書いた。
- `connect` を送らない版を Amber で試した。セットアップから公開までは通った。その後のレプリカ報告（kind 35981）は 90 秒で返事が無かった。原因を確かめる前に Amber を消したので、todo に残した。
- セットアップ画面の QR まわりを `pairing.js` に切り出したときに、relay 欄を読む `collectRelays` まで消していて、「セットアップ」ボタンが `ReferenceError` で止まった。戻して、別のテスト用 SWING でセットアップの送信までブラウザで通した。
- 画面と README に載せる署名アプリは、Android が Amber、iPhone / iPad が Clave（App Store。閉じていてもプッシュ通知で起きて署名する作り。プッシュの中継サーバーに渡るのは relay の URL とイベント ID だけ。Medium でも毎回聞く kind は 0・3・5・10002・30078 で、SWING の kind は入らない）にした。nsec.app はロック中に答えないことがあるので外した。NostrKey（Android）は、他のアプリの署名役になれるとは読めなかったので載せていない。Clave は iPhone が無く試していない（todo）。

## 追記: Amber でもう一度（`wss://relay.primal.net`）

- Amber を入れ直し、署名アプリとの relay を `wss://relay.primal.net` にして、まっさらな状態からセットアップした。確認の署名・セットアップ・ダッシュボードからの公開が通り、`swing stop --restart` の直後のレプリカ報告も署名できた（再起動から 2 秒）。
- スマホの画面を消して 5 分置いてから、CLI の `swing publish`（kind 35980）と、続けて再起動してのレプリカ報告（kind 35981）を試し、どちらも署名できた。公開は Amber の通知で承認を求められ、承認して通った（約 19 秒）。レプリカ報告は承認なしで 2 秒で通った。確認の署名で選んだ「常に許可」は kind 35981 にだけ効いていて、公開は毎回承認する設定になっていた（画面で案内している「kind 35981 だけ許可」の形）。
- 前回（原因を確かめる前に Amber を消した回）のレプリカ報告のタイムアウトは再現しなかった。todo から外した。
- relay.damus.io は、この回線からだと 1〜2 分おきに `503 Service Unavailable` で切れていた。relay.primal.net では切れなかった。
- 署名アプリとの relay の既定を `wss://relay.primal.net` に変えた（セットアップ画面・つなぎ直しの画面）。
- サイドナビのフッタで長いミラーセット名が途中で切れていた。`text-overflow: ellipsis` がフッタの枠（flex コンテナ）に付いていて、中の行（`span`）には効いていなかった。行ごとに付け直して「…」で省略し、ミラーセットの行には省略前の文字列を `title` に入れた。
