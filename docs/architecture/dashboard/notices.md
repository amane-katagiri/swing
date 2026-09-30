# おしらせ（`web/desktop-updates.js`, `web/desktop-notify.js`, `web/notify-settings.js`）

[`../dashboard.md`](../dashboard.md) の子ページ。どの画面でも動き、新しく保存した版・自分の publish・自分のサイトをミラーする人の増加を見つけ、マスコットの吹き出しかブラウザの通知で知らせる仕組み。マスコット側の出し方は [`mascot.md#おしらせ`](mascot.md#おしらせ)。

## 設定

Desktop 画面の「コントロール パネル」→「通知」タブ（[`desktop/control-panel.md`](desktop/control-panel.md)）と Settings 画面の「通知」パネル（[`views.md#通知`](views.md#通知)）の両方が同じ値を読み書きする。読み書きの関数は `notify-settings.js`（[`web.md#構成`](web.md#構成)）。

| 項目 | 保存先 | 値 | 既定 |
|---|---|---|---|
| 確認の間隔 | `swing:desktop:mascot` の `interval`（キーの形は [`mascot.md#マスコットタブ`](mascot.md#マスコットタブ)） | `60`・`300`・`900`・`1800`（秒）か `null`（確認しない） | `60` |
| 種類ごとのオン・オフ（マスコット側） | `swing:desktop:notify` の `mascot` | `{stored, published, replica}` の真偽値 | すべて `true` |
| ブラウザの通知を使うか・種類ごとのオン・オフ（ブラウザ側） | `swing:desktop:notify` の `browser` | `{enabled, stored, published, replica}` の真偽値 | `enabled` は `false`、種類はすべて `true` |

- 種類は `stored`（フォロー中のサイトを新しくミラーした）・`published`（サイトを publish した）・`replica`（自分のサイトが新しくミラーされた）。
- 壊れた値・想定外の値は項目ごとに既定へ落とす。`swing:desktop:mascot` はマスコットの設定と共有するので、書く側は今の値を読み直して自分の項目だけを書き換える（`writeMascotSettings`）。
- 間隔はページ読み込み時に 1 回読み、変えた側（「通知」タブの OK/適用か Settings 画面）が `desktopUpdates.setInterval` で当てる。「通知」タブは開くたびに読み直し、前回から変わっていればそのとき当て直す。`swing:desktop:notify` は watcher・マスコット・ブラウザの通知が使うたびに `localStorage` から読むので、保存した時点で効く。
- 保存に失敗したらエラーを出し、保存値は変えない。

### ブラウザの通知の許可

「ブラウザの通知を使う」をオンにする操作の中で、許可がまだなら `Notification.requestPermission()` を呼ぶ（`requestBrowserPermission`）。

- 許可されなければチェックを外し、理由（ブロックされている・許可が得られなかった）を出す。
- 安全なコンテキストでない（`window.isSecureContext` が偽。LAN の `http://` で開いたときなど）か `Notification` が無いブラウザでは、チェックボックスを無効にして理由を出す（`unavailableReason`）。
- 保存済みの値がオンでも、画面を開いたときに許可が無くなっていれば理由を出す（`blockedReason`）。
- ブラウザの通知が使える（`browserNotifyReady`）のは、「使う」がオンで、ブラウザ側の種類が 1 つ以上オンで、`Notification.permission` が `granted` のとき。

## 更新の確認

`desktop-updates.js` の watcher（`desktop.js` が 1 つだけ作る `desktopUpdates`）が、どの画面を開いていても更新を確かめ、おしらせを受け手（マスコットとブラウザの通知）に流す。

- **確認の時機**: 間隔ごと、ページを開いて最初の画面を出した直後、Desktop 画面を表示したとき、タブが前面に戻ったとき。ログイン画面・セットアップ画面では確認しない。タブが裏にあるときは、ブラウザの通知が使えるときだけ確認する。同時には 1 回しか走らない。「確認しない」のときは `/api/activity` を呼ばない。失敗が続く間は間隔を延ばし、接続できなかった後は cookie を付けない確認が通るまで `/api/activity` を呼ばない（[`web.md#止まっている間の呼び出し`](web.md#止まっている間の呼び出し)）。
- **確認の中身**: [`GET /api/activity`](http-api/status.md#get-apiactivity) の 3 つの値（カーソル）を、種類ごとの既読の値（`localStorage`）とおしらせ済みの値（メモリだけ）と比べ、進んだ種類だけ一覧を取り直して新しいものをおしらせする。ページを読み込み直すと、おしらせしたが既読になっていないものはもう一度おしらせする。

| カーソル | 既読のキー | 取り直すもの | おしらせするもの | イベント |
|---|---|---|---|---|
| `latest_stored_at` | `swing:desktop:seen` | `/api/sites`（手元の一覧が新しければ取り直さない。Explorer と Sites 画面の表示もこれで更新される） | `accounts` と `unfollowed.accounts` の `stored` な版のうち、リンク先（`gateway_url`、無ければ `url`）が `http://`・`https://` で、`stored_at` が前回より新しいもの | `sites-stored` |
| `latest_published_at` | `swing:desktop:seen-published` | [`/api/publish/sites`](http-api/publish.md#get-apipublishsites) | `created_at` が前回より新しい版 | `published` |
| `latest_replica_report_at` | `swing:desktop:seen-replicas` | [`/api/replicas`](http-api/nostr.md#get-apireplicaskeykey)（自分） | ミラーする人が加わったサイト（下記） | `replicas-added` |

- **初めてのとき**: 既読の値が無ければ、その時点の値を既読として記録するだけでおしらせしない。`null`（まだ分からない）の間は何も記録しない（`latest_stored_at` の `null` は版が無いことなので 0 を記録する）。`0` は記録するので、その後の最初の publish や最初にミラーする人はおしらせする。
- **ミラーする人の増加**: `swing:desktop:replica-reporters` に、サイトの `d` ごとの報告者（`latest: true` で tier が `author`・`chosen`（[`../nostr.md#レプリカ報告の信頼度replicastier`](../nostr.md#レプリカ報告の信頼度replicastier)）の、作者本人以外）を持ち、確認のたびに今の集合で置き換える。前回の集合に無い報告者がいるサイトだけをおしらせし、報告の出し直しや撤回ではおしらせしない。いったん集合から消えてまた加わった報告者と、`other` から `chosen` になった報告者は新しく加わったものとして扱う。サイトのタイトルとリンク先は `/api/publish/sites` から引き、見つからなければタイトル・リンク無しで出す。
- **ダッシュボードからの publish**: Publish 画面の publish が成功し応答の `published` が `false` でないとき（`publish.js` が `swing:published` を投げる）、確認を待たずにその版をおしらせする。
- **種類のオン・オフ**: ある種類を確認するのは、マスコット側でオンか、ブラウザ側でオンでブラウザの通知が使えるときだけ（設定は確認のたびに読む）。どちらでもない種類は既読を今の値まで進めるだけで、オンに戻しても外していた間の分はおしらせしない。ミラーする人の増加は、外すと既読と報告者の集合を消し、オンに戻したら初めてとして扱う。マスコット側がオフの種類は、流した時点で既読にする。
- **既読**: 受け手が `acknowledge(notices)` を呼ぶと、種類ごとに `at`（`stored_at`・`created_at`・報告者を初めて見たときの `latest_replica_report_at`）の最大値まで既読を進める（戻しはしない）。既読の判定は `localStorage` の今の値で行うので、別のタブで既読にしたものも既読になる。
- **流すイベント**: `{kind: 'sites-stored' | 'published' | 'replicas-added', notices}`（各おしらせは合流に使う `key`（ミラーする人の増加はサイトごと）・`at`・`href` を持ち、`at` の昇順）、`{kind: 'fetch-error', error}`（取得に失敗したとき。失敗が続く間は最初の 1 回だけ）、`{kind: 'recovered'}`（失敗の後に最初に確認し終えたとき）。

## おしらせの出し分け

1 回ぶんのおしらせ（`sites-stored`・`published`・`replicas-added`）は、Desktop 画面が表示中でタブが前面にあり、マスコットが 1 体以上表示されている（`DesktopMascots.showing()`）ならマスコットの吹き出しで、それ以外ならブラウザの通知で出す。種類のオン・オフは、マスコットが見えているときはマスコット側、見えていないときはブラウザ側で判定する。マスコットが見えているときにマスコット側がオフの種類は、どちらにも出さない。

- **マスコット**: 受ける種類・預かり・出し方は [`mascot.md#おしらせ`](mascot.md#おしらせ)。
- **ブラウザの通知**（`desktop-notify.js::createBrowserNotifier`）: ブラウザの通知が使えるときだけ、ブラウザ側でオンの種類だけを出す。
  - 既読のものを除き、さらに `swing:desktop:notified`（種類ごとに、通知を出した `at` の最大値の JSON）より新しいものだけを出してその値を進める。出しただけでは既読にしない（マスコット側がオフの種類は watcher が流した時点で既読にする。[更新の確認](#更新の確認)）。
  - 1 回ぶんを 1 つの `Notification` にする。題は `SWING`、本文は 1 件ならサイトのタイトル（サニタイズして最大 60 文字、無ければ `d`）入りの文、2 件以上なら件数入りの文（表示言語の設定に従う i18n）。`tag` は `swing:<kind>:<最も新しい at>` で、複数のタブが同じおしらせを出しても重ならない。
  - `new Notification` が例外を投げる環境（ページからの通知を作れないモバイルのブラウザなど）では何もしない。
  - クリックすると通知を閉じてタブを前面に出し、通知を出した時点でのその種類のマスコット側のオン・オフで次のように動く。クリックでは既読を進めない。

| 種類 | マスコット側オン | マスコット側オフ |
|---|---|---|
| `stored` | Desktop 画面（`#/desktop`） | 1 件ならリンク（`href`）を新しいタブで開く。2 件以上（またはリンクが `http://`・`https://` でない）なら起動時の画面（`desktop-system-settings.js::startupView()`。`desktop` か `sites`） |
| `published` | Desktop 画面 | Publish 画面（`#/publish`） |
| `replica` | Desktop 画面 | Webring 画面で自分のノードを選んだ状態（`swing:show-self-in-webring` を投げる。[`views.md#webring-画面`](views.md#webring-画面)）。詳細パネルは watcher が `cache.replicasByKey` に入れた最新の `/api/replicas` をそのまま使う |
