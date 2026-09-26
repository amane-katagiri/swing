# Desktop のスクリーンがフォーカスでスクロールする不具合の修正

## 何をしたか

- `web/desktop.css`: `.desk-screen` を `overflow: hidden` から `overflow: clip` に変えた。
- `docs/todo.md` から該当の項目を消し、`docs/architecture/dashboard/desktop.md` に「スクリーンはスクロールしない」ことを足した。

## 原因

`overflow: hidden` の要素はスクロールバーを出さないだけで、スクロールコンテナのまま。SWING Explorer ウィンドウを画面の下にはみ出させた状態で、iframe（リンク集ページ）の中の、画面外にあるリンクにフォーカスが移ると、ブラウザがそのリンクを見せようとして親文書の祖先のスクロールコンテナ `#desk-screen` をスクロールさせていた。

## 決めたこと

| 決定 | 理由 |
|---|---|
| `overflow: clip` にする | `clip` はスクロールコンテナを作らないので、フォーカスによる自動スクロールの対象から外れる。`scroll` イベントで `scrollTop` を 0 に戻すやり方は、一瞬ずれて描画されうるうえ JS が増える |

## 検証

デモ環境の `#/desktop`（1280×800）で、ウィンドウをタイトルバーが y=350 付近に来るまで下へドラッグし、iframe の中で画面外にあるリンクに `focus()` した。

- 修正前: `#desk-screen.scrollTop` が 304 になった
- 修正後: `#desk-screen` とその祖先の `scrollTop` はすべて 0 のまま
