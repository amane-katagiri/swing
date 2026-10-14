# Desktop 画面のトレイのネットワークアイコンに到達性を映す

## 決めたこと

- タスクバーのトレイのネットワークアイコンを飾りから外し、`GET /api/network` の `reachability` に合わせて 4 つの見た目を切り替える。`public` は今までの絵を 2px 下げたもの、`private` は右下に黄色い警告の三角、`unknown` は画面を灰色にしたモニタ、`offline` は灰色のモニタの右下に赤い ×。
- 状態の印は、Windows XP/7 の通知領域のネットワークアイコンにならって右下に置き、前のモニタに重ねる。初めは空いている左下に置いたが、見慣れた位置のほうが状態の印として読み取りやすいので右下に移した。
- 取得の失敗（止まっている・古い `swing up` の 404・準備中の 503 など）は全部 `unknown` の見た目にする。判定できないことに変わりはないので、別の見た目を作らない。
- 取得の時機は更新の確認の watcher に相乗りする。止まっている間の cookie を付けない確認・失敗時の間隔の延長・ログイン画面での停止を別に持たずに済む。watcher に `onChecked`（確認を試みた後に成否付きで呼ぶ）を足し、確認が通った回だけ `/api/network` を取り直し、通らなかった回は取得せず `unknown` にする。`subscribe` のイベントに混ぜなかったのは、マスコットが読み込み前のイベントを溜めるので、確認のたびに溜まるのを避けるため。
- 取り直すのは Desktop 画面を表示していてタブが前面にあるときだけにした。アイコンが見えない間に呼んでも意味がなく、Desktop 画面を表示したときは watcher が確認するのでそこで追いつく。
- 間隔が「確認しない」のときは watcher が確認しないので、Desktop 画面を表示するたびに 1 回だけ直接取得する（watcher の `enabled()` で見分ける）。
- 文言は Settings 画面の「ネットワーク」パネルの状態の 1 行と同じ i18n のキー（`networkPublic`・`networkPrivate`・`networkUnknown`・`networkOffline`）を使う。Desktop 画面の中は UI 表示言語に関わらず日本語に固定する決まりだが、ツールチップは Desktop 画面の絵の外に出るので訳しても見た目が崩れず、サイドナビのラベルと同じく例外にして UI 表示言語に従わせた（初めは日本語に固定していた）。`span` の `lang` も UI 表示言語にし、言語の切り替え（`swing:langchange`）で引き直す。
- アイコンは情報を持つので `aria-hidden` を外し、`<span role="img">` に `aria-label` と `title` を付ける。SVG 要素の `title` 属性はツールチップにならないので、`svg` は `aria-hidden` のまま `span` で包む。押せず、フォーカスも受けない。

## 作ったもの

- `web/desktop-icons.svg`: `icon-desk-tray-network-private`・`-unknown`・`-offline` を `icon-desk-tray-network` の後ろに足した。スプライトは手で書いたもので、生成元は無い。2 台のモニタは元の絵を 2px 下げ（y 2〜13）、前のモニタを 1px 左へ寄せて重なりを増やし（x 0〜14）、`icon-desk-tray-network` もそれに合わせた。元の絵は上に寄っていて下の 4 行が空き、右下の印だけが下へはみ出して見えたため。警告の三角は 9×9（x 7〜15・y 7〜15）で、黄色の外側を上下左右 1px の黒で縁取り、「!」は縦棒・すき間・点を分け、上に黄色を 1 行残す（右端・下端に接した 7×7 では縁が切れ、「!」が縁とつながって「A」に見えた）。× は 7×7（x 9〜15・y 9〜15）の 2〜3px 幅の太い形にして縁取りは付けない（縁取りを付けると × のくぼみが埋まって黒い四角に見えた）。印の右端が前のモニタの右端と同じ列に揃うと、印がモニタに収まって見えて落ち着かないので、前のモニタを 1px 左へ寄せ、印だけが前のモニタから 1px 右へはみ出すようにした。幅を 17px に広げる案も試したが、トレイのほかのアイコンと大きさが揃わなくなるので、16×16 のまま重なりを増やすほうにした。灰色の画面は奥が `#808080`、手前が `#a0a0a0`（タスクバーの地と同じ `#c0c0c0` では手前の画面が抜けて枠だけに見えた）。
- `web/desktop-network.js`（新規）: `DesktopNetwork`（`init`・`onShow`）。
- `web/desktop-updates.js`: `onChecked`・`enabled` を足した。
- `web/desktop.js`: `DesktopNetwork` の初期化と、表示時の `onShow` の呼び出し。
- `web/index.html`・`web/desktop.css`: トレイのネットワークアイコンを `#desk-tray-network` の `span` で包んだ。トレイ（`.desk-tray`）は飾りとして `pointer-events: none` なので、ツールチップが出るよう `span` だけ `pointer-events: auto` に戻した。
- `src/dashboard/assets/mod.rs`: `desktop-network.js` を配信に加え、`index.html` と `web/*.js` が参照する `icon-desk-*` がすべてスプライトにあることを確かめるテストを足した。
- `docs/architecture/dashboard/desktop.md`・`notices.md`・`web.md`: トレイのネットワークアイコンの状態・時機・失敗、watcher の `onChecked`・`enabled`、Desktop 画面の日本語固定の例外。

## 検証

- `cargo fmt --all`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace` が通った。
- デモ環境の `#/desktop` で、`window.fetch` を包んで `/api/network` の応答を 4 つの状態と 404 に差し替え、確認をやり直してアイコン・`title`・`aria-label` が切り替わることを確かめた。実際のデモ環境は Kubo が `--offline` なので `offline` になる。
- 間隔が「確認しない」のとき、Desktop 画面を表示するたびに `/api/network` だけが 1 回呼ばれ、`/api/activity` は呼ばれないこと、間隔があるときは表示時に `/api/activity` に続いて `/api/network` が呼ばれることを確かめた。
- ライトとダークのどちらのテーマでも、Desktop 画面は固定のパレットなのでトレイの見た目が同じで、等倍で 4 つの見た目を見分けられることを確かめた。
- README の画面素材（`docs/assets/dashboard-desktop.webp`・`.png`）は撮り直していない。デモ環境で撮ると、トレイのネットワークアイコンは `offline` の見た目（赤い ×）になる。
