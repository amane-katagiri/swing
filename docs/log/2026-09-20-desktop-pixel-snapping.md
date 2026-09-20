# Desktop 画面の字が幅によって滲む問題

## 症状

ブラウザの幅を 1px 変えるたびに、Desktop 画面の「ウィンドウの字（タイトルバー・アドレス行・ステータスバー）が滲む幅」と「ページ本文の字が滲む幅」が交互に現れる、という報告。PixelMplus12 はビットマップ由来なので、半ピクセルずれるとアンチエイリアスが乗って滲む（`-webkit-font-smoothing: none` は効かない）。

## 原因（測って確かめた）

`web/` を静的配信して Chrome で幅を 1px ずつ変えながら `getBoundingClientRect()` を取った。原因は独立した 2 つで、たまたま互いを打ち消し合っていた。

| 幅 | `.desk-window` の left | iframe の left | `.desk-page-inner` の left（iframe 内） | 本文の絶対位置 |
|---|---|---|---|---|
| 1400 | 344 | 346 | 98.5 | 444.5（滲む） |
| 1401 | 344.5（滲む） | 346.5 | 98.5 | 445 |

- ウィンドウ側: 既定ジオメトリの中央寄せが `(画面幅 - ウィンドウ幅) / 2`。画面幅が奇数・ウィンドウ幅が 920（偶数）だと 0.5px になり、ウィンドウごと—中の iframe まで—半ピクセルずれる。
- ページ側: `.desk-page-inner` の `max-width: 720px; margin: 0 auto`。iframe の内幅は窓枠（ウィンドウの border 1px×2 + `.desk-page` の border-left 1px）の分だけ奇数になりがちで、`auto` 中央寄せが常に 0.5px を吐いていた。

## 直したこと

- `desktop.js` の `applyWindowGeometry()` で `left`/`top`/`width`/`height` を `Math.round` してから当てる。状態（`winState.geom`）は実数のまま持ち、描画の瞬間だけ丸める（ドラッグの累積誤差を作らないため）。
- 最大化を `width: 100%; height: 100%` から `Math.floor` した実寸 px に変えた。画面幅が半端なときに右端のタイトルバーボタンが半ピクセルに載るのを防ぐ。`ResizeObserver` → `reflowWindow()` で追従するので挙動は変わらない。
- `desktop-page.css` の `.desk-page-inner` を CSS の `round()` で置く。幅は 2px 単位に切り下げ（`--desk-col`）、左マージンは 1px 単位に切り下げ、右は `auto`。幅まで偶数にするのは、中の `text-align: center`（`.desk-hero` など）も半ピクセルを踏まないようにするため。`@supports (width: round(down, 50%, 2px))` で囲ってあるので、`round()` 未対応のブラウザでは従来の `margin: 0 auto` のまま。

## 検証

同じ計測を幅 1400〜1403 / 900 / 901 / 700 / 701 / 375、および最大化・ナロー幅（760px 以下）で回し、ウィンドウ・iframe・`.desk-page-inner`・ステータスバー・タイトルバーの left/top/width がすべて整数になることを確認した。

## マーキー

幅とは関係なく常に滲んでいたので、続けて直した。デモ環境のスクリーンショットからマーキー帯の色を数えると、静止テキストが 2 階調（純色 + フォント自身のハーフトーン 1 段）なのに対し、アニメーション中は 7〜8 階調。`transform` のアニメーションは合成レイヤごと半端な位置に置かれるため。

試した順:

1. `steps(160)` を挟む → 7 階調のまま。各ステップの位置（距離 × i/N）が整数にならないので意味がない。
2. 合成を諦めさせる（`background-position` を同じ `@keyframes` に混ぜてメインスレッド実行にする）→ 6〜7 階調のまま。
3. 位置を登録済みカスタムプロパティで補間し、`round()` で 1px に丸めてから `transform` に渡す → 2 階調（+ 無関係な白 1 色）。静止テキストと同じになった。

採用したのは 3。

```css
@property --desk-marquee-x { syntax: "<length-percentage>"; inherits: false; initial-value: 0%; }
@keyframes desk-marquee-snapped { from { --desk-marquee-x: 0%; } to { --desk-marquee-x: -100%; } }
@supports (transform: translateX(round(-100%, 1px))) {
  .desk-marquee-track {
    animation-name: desk-marquee-snapped;
    transform: translateX(round(var(--desk-marquee-x), 1px));
  }
}
```

距離は従来どおり `-100%`（`padding-left: 100%` を含んだ自分の幅）なので、文面の長さに関わらず流れ方は変わらない。速度も 16s のまま。

`@supports` が見ているのは `round()` だけで `@property` は見ていない。両方入っていないブラウザ（Firefox 118〜127 / Safari 15.4〜16.3 だけがこの隙間に入る）では、カスタムプロパティが未登録＝離散補間になり、マーキーが途中で 1 回飛ぶ。CSS から `@property` の対応を判定する方法が無いため（`@supports at-rule(@property)` は Chrome 145 でも false、登録済みプロパティの構文チェックも `@supports` には効かない）、この隙間は許容した。どちらもとっくに自動更新で抜けているバージョン。
