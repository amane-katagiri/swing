# publish の共通処理（`src/publish.rs`, `src/publish/checks.rs`, `src/publish/clock.rs`, `src/publish/new_files.rs`, `src/publish/staged.rs`）

CLI の `swing publish`（[`cli/publish.md`](cli/publish.md)）とダッシュボードの `POST /api/publish/upload`（[`dashboard/http-api/publish.md`](dashboard/http-api/publish.md#post-apipublishupload)）が共有する段階。ここには表示に依存しない判定と処理だけを書き、出力の文言・ステータスコード・JSON はそれぞれのページに置く。

## 段階の順

1. 引数の検証。`d`・`url` は [`nostr.md` の検証](nostr.md#検証)の条件（`validate_site_fields`）、`title` は前後の空白を削り、空なら付けない扱いにしてから同じく検証する（`normalize_title`）。`content`（メッセージ）は `MAX_CONTENT_BYTES`（4096 バイト）まで（`validate_message`）。通常の投稿を頼むなら `url` が要る（`check_note_request`。無ければ `needs a URL`）。確認のモード 4 つ（`nip05`・`check_dotfiles`・`check_size`・`check_unchanged`、それぞれ `off`/`warn`/`require`）は省略時に `[publish]` の同名の設定を使う（`resolve_modes`）。
2. [保護パスの拒否](#保護パスの拒否)。
3. NIP-05: モードが `off` でなければ `d` と自分の pubkey で検証する（`check_nip05`。[`nip05.md`](nip05.md)）。`require` で `Verified` 以外（`NotApplicable` を含む）なら add せずに止める。
4. [サイトの一覧とローカルの確認](#サイトの一覧とローカルの確認)。
5. Kubo と relay の準備。
6. [前の版と relay の時計](#前の版と-relay-の時計)を同時に取る。
7. [時計の確認](#時計の確認)をして [`created_at`](#created_at-の決め方) を決め、[add して版を置く](#add-と版の配置)。
8. [同じ内容かの確認](#同じ内容かの確認)。
9. [署名と送信](#署名と送信)。
10. 投稿を頼まれていれば[通常の投稿](#通常の投稿)。
11. [古い版の削除](#古い版の削除)。

7 以降で失敗したときに MFS に置いた版をどうするかは[告知できなかった版の後始末](#告知できなかった版の後始末)。

## 保護パスの拒否

`publish::refuse_protected_paths`。確認のモードにかかわらず、次のどちらかなら何もせずに止める。存在しないパスは見ない。

- サイトのディレクトリの実体（`canonicalize`）の中に、設定ファイル・`[agent].state_dir`・`[kubo].repo` のどれかの実体がある（ディレクトリそのものである場合を含む）。
- サイトのディレクトリの実体が `[kubo].repo` か `[agent].state_dir` の実体の中にある（そのものである場合を含む）。ただし `<state_dir>/upload/`（ダッシュボードの展開先。`DASHBOARD_UPLOAD_DIR`）の下は止めない（`upload/` そのものは止める）。設定ファイルのあるディレクトリの中は止めない。

## サイトの一覧とローカルの確認

サイトのディレクトリを 1 回だけ一覧し（`ipfs::SiteListing`。シンボリックリンクを辿り、ドットファイルも含める。リンク先がディレクトリの外ならこの時点でエラー。[`mfs.md`](mfs.md#rpc)）、以降の確認と add はこの一覧を使う。一覧の後にディレクトリへ増えたファイルは公開しない。

`LocalChecks::evaluate` が、モードが `off` でない項目だけを判定する。

- ドットファイル（`find_dotfiles`）: 各パスをルートから順にセグメントごとに見て、名前が `.` で始まり `[publish].dotfiles_allow` のどれとも一致しない最初のセグメントまでを 1 件とする（ディレクトリは 1 回だけ数え、その下は見ない）。一致するセグメントはそれ自身だけを見逃し、その下は続けて見る（`.well-known/.env` は `.well-known/.env` を 1 件とする）。表示や応答に並べるのは先頭 `LISTED_DOTFILES`（10）件。
- サイズ: ファイルの大きさの合計（ブロックの共有やディレクトリのノードは数えない）が `SIZE_GUIDELINE`（512 MiB、固定）を超えたら引っかかる（ちょうどは超えない扱い）。ミラーが保存するかは各ミラーの `max_update_size` で決まるので、これは目安。
- `require` の項目が引っかかったら、項目ごとの対処の案内を `; ` でつないだメッセージ（`abort_message`）で、add せずに止める。

## 前の版と relay の時計

`publish::RelayState::fetch` が次の 2 つを同時に行う。結果はこの回の publish の終わりまで使い回すので、CLI の確認の待ち時間で判定は変わらない。

- 前の版: 自分の pubkey・この `d` のサイトイベントのうち `parse_site_event` を通る最新 1 件（`fetch_own_latest_site`。[`nostr.md` の検証](nostr.md#検証)）。`created_at` が[未来ずれの許容](nostr.md#未来ずれの許容nostrmax_future_skew)を超えるものは取得の段階で除かれる。`check_unchanged` が `off` でも取る。取れなかったときは理由を持ち、`created_at` の計算には使わない。
- relay の時計（`clock::probe_relay_offsets`）: 設定の relay ごとに `ws://` を `http://`、`wss://` を `https://` に読み替えて `Accept: application/nostr+json`（NIP-11）で GET し、応答の `Date` ヘッダー（HTTP-date）だけを読む。全 relay に同時に問い合わせ、1 件 5 秒で打ち切り、リダイレクトは辿らず、プロキシは使わない。応答を受け取った時点のこの機械の時刻から `Date` を引いた値を relay ごとのずれとし、接続できない・`Date` が無い・読めない relay は無視する。

## 前の版のファイル一覧

`publish::PreviousFiles::load`。CLI の増えたファイルの確認と `GET /api/publish/previous-files` が使う。

- 前の版があれば、その CID を Kubo でオフラインに（ローカルにあるブロックだけで）たどり、ファイルのパス（ディレクトリは含まない）をバイト順で一覧する（`ipfs::list_files_local`。[`mfs.md`](mfs.md#rpc)）。たどった項目が `MAX_PREVIOUS_ENTRIES`（100 000）を超えたら一覧できない扱い。
- 状態は「一覧できた」「前の版が無い」「不明（relay から取れなかった、または Kubo で一覧できなかった。理由付き）」の 3 つ。一覧できなかったときは前の版を空として扱う。
- サイトの一覧のファイル（ディレクトリは除く）のうち、前の版に同じパスが無いものが「増えたファイル」（`new_files`）。中身が変わっただけのファイルは数えない。

## 版

`<mfs_root>/publish/<pubkey hex>/<site>/` の中で、名前が 10 進の正規形（先頭の `0` や `+` が無い。`0` は可）の整数の項目。ディレクトリが無ければ版も無い扱い。名前は署名したサイトイベントの `created_at` と同じ値。

## 時計の確認

`publish::plan_version`。MFS の版を一覧し（失敗したらエラー）、次の順に確かめ、当たれば何も add せずに `publish::ClockError` で止める。自動では直さない。メッセージ中のずれの大きさは `1 second` / `N seconds` / `about N minutes` / `about N hours` / `about N days` で示す。

署名の上限は現在時刻 + [`MAX_FUTURE_SKEW`](nostr.md#未来ずれの許容nostrmax_future_skew)（900 秒）− `SIGN_MARGIN`（60 秒）。ミラーや relay の時計がこの機械より少し遅れていても、署名したイベントが未来すぎるとして捨てられないための余裕。

1. MFS の未来の版: 最大の版 + 1 が署名の上限を超える。その版が現在時刻 + `MAX_FUTURE_SKEW` 以内なら、待てば署名できるので待つ秒数（版 + 1 − 署名の上限）を案内する。それより先なら、この機械の時計が遅れている（時計を直してやり直す）か、その版を時計が進んでいたときに作った（`ipfs files rm -r <その版のパス>`、Docker Compose なら `docker compose exec ipfs ipfs files rm -r <その版のパス>` で消してやり直す）かのどちらかだと案内する。
2. relay の前の版が先の日付: 前の版の `created_at` + 1 が署名の上限を超える。前の版は取得の段階で現在時刻 + `MAX_FUTURE_SKEW` までに限られるので、超える幅は最大 `SIGN_MARGIN` + 1 秒で、待つ秒数を案内する。
3. relay より進んだ時計: [relay の時計](#前の版と-relay-の時計)のずれの最小が `MAX_FUTURE_SKEW` を超える。答えた relay が 1 つも無ければ確かめずに進む。1 つの relay に公開を止めさせないことを優先した最善努力の確認で、`Date` が遠い未来の relay が 1 つでも答えれば止めない。

relay が持つ自分のサイトイベントが未来ずれの許容を超えて先の日付なら、swing にもミラーにも見えず、その relay では時刻が過ぎるまで新しい公開で置き換わらない（上書きする手段は無い）。

## `created_at` の決め方

`clock::version_time`。現在時刻・MFS の最大の版 + 1・前の版の `created_at` + 1 の最大。前の版を取れなかったときはそれを使わない。別の publish（CLI とダッシュボードのような別のプロセスを含む）と同じ秒を選んだときは、[配置](#add-と版の配置)で 1 秒ずつ進めて空いているパスを取る。

## add と版の配置

`publish::add_and_measure`。add の直前に[時計の確認](#時計の確認)をして `created_at` を決め、サイトの一覧を CIDv1・pin なしで add し（MFS にはまだ置かない）、その CID を `<mfs_root>/publish/<pubkey hex>/<site>/<created_at>` に置く（`IpfsClient::mfs_place`。[`mfs.md`](mfs.md#rpc)）。そのパスに既に項目があれば Kubo が配置を断るので、既存の項目は消さずに `created_at` を 1 秒進めて置き直す。置けたパスだけがこの publish の版になり、[後始末](#告知できなかった版の後始末)もこのパスだけを消すので、並行する別の publish の版を上書きしたり消したりしない。続けて `dag/stat`（`offline=true`）の `TotalSize` をサイトイベントの `size` タグにする。pin なしの add は Kubo の GC を止めないので、この `dag/stat` がブロックの欠けを確かめる役も持つ（add から配置までの間にルートのブロックが消されていれば、配置が `offline=true` で失敗する）。

## 同じ内容かの確認

`publish::check_unchanged`。モードが `off` でなければ、add した CID を前の版の CID と比べる。状態は「変わった」「同じ」「前の版が無い」「不明（relay から取れなかった）」。同じで `require` なら、置いた版を MFS から消して、署名・送信・古い版の削除をせずに成功として終わる（消せなければエラー）。前の版が無い・不明のときは `require` でも続ける。

## 署名と送信

サイトイベント（`alt` は `SWING site announcement: <d>`）を [`created_at`](#created_at-の決め方) で作って署名し（`sign_site_event`）、全 relay に送る（`send_site_event`）。

- どの relay にも受理されなければ、置いた版も古い版も残してエラーにする（`NO_RELAY_ACCEPTED`: `no relay accepted the site event; old versions were kept`）。
- relay が断った理由が `created_at` を未来すぎるとするもの（`rejected_as_future`）なら、理由の後ろに ` (the relay thinks the event is dated in the future; check your clock)`（`FUTURE_REJECTION_HINT`）を足す。成否は変えない。判定は大小文字を無視し、英数字と `_` の続きを 1 語として、次のどれか。
  - `future` の語がある
  - `too late` か `too new` と 2 語が続く
  - `created_at` と `too` の語があり、`early`・`old`・`past` の語が無い
- 案内を足すときは、全体が 500 文字（`MAX_REJECTION_DISPLAY_CHARS`。CLI が断った理由を表示で切る長さ）に収まるよう、理由を `format::sanitize_display_text` で 429 文字（切ったときは省略記号を足して 430 文字）までにしてから足す（`nostr::with_rejection_hint`）。案内を足さない理由はそのまま返す。

## 通常の投稿

`publish::post_site_note`。サイトイベントがどれかの relay に受理された後だけ行い、[同じ内容](#同じ内容かの確認)で止めたときや受理されなかったときは行わない。

- `kind 1` で、`content` は `title`・`url`・`content`（メッセージ）のうち空でないものと、ハッシュタグ `#swingpublish` をこの順に半角スペース 1 つでつないだもの（`nostr::site_note_content`）。タグはサイトを指す `a`（`<site_event_kind>:<pubkey hex>:<d>`）と、NIP-24 のハッシュタグ `t`（`swingpublish`）（`nostr::build_site_note_builder`）。`created_at` は署名する時点の現在時刻。
- 署名してから全 relay に送り、relay ごとの成否を返す。断った理由への案内の足し方は[署名と送信](#署名と送信)と同じ。
- 署名・送信の失敗や、どの relay にも受理されなかったこと（`NO_RELAY_ACCEPTED_NOTE`: `no relay accepted the note`）は publish の成否を変えず、古い版の削除も続ける。

## 告知できなかった版の後始末

置いた版（[配置](#add-と版の配置)で置けたパス）は `publish::StagedVersion` が持ち、署名できるまでの失敗では MFS から消す。

- 消す: CID が読めない、`dag/stat` の失敗、署名できない、[同じ内容](#同じ内容かの確認)で止めるとき。消せなければその理由も元のエラーに続ける。
- 処理が途中で打ち切られた（ダッシュボードのタイムアウトやクライアントの切断）ときも、後から削除を始める（失敗は警告のログだけ）。
- 署名できた後は、送信の失敗や打ち切りがあっても消さない。

## 古い版の削除

`publish::prune_old_versions_collect`。受理された後、[版](#版)のうち今回の版は必ず残し、それ以外を新しい順に `[publish].keep_versions - 1` 個残して消す。一覧や削除に失敗しても publish は成功のままで、失敗の内容だけを返す。
