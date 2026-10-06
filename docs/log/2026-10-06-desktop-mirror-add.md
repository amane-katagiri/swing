# Desktop 画面のツールバーに「ミラー」ボタンを足す

## 決めたこと

- SWING Explorer のツールバーの「更新」の右に、星のアイコンの「ミラー」ボタンを置く。IE の「お気に入り」に見立てた。
- 押すと Sites 画面の「ミラーに追加」と同じ `POST /api/mirror/add` を呼ぶモーダルダイアログを開く。殻はコントロール パネルと同じ `createDialog` を使い、Win95 風の見た目に合わせる。
- 1 件以上追加できたらダイアログを閉じ、リンク集を取り直してステータスバーに結果を出す。何も追加されなかったとき・失敗したときは、入力をやり直せるようにダイアログを開いたまま中に文面を出す。
- 鍵の分割と上限（100 件）は Sites 画面と共有するため `ui.js` の `parseMirrorKeys`・`MAX_MIRROR_KEYS` に移した。

## 作ったもの

- `web/desktop-mirror-add.js`（`DesktopMirrorAdd`）と、`index.html` のボタン・ダイアログ。
- `desktop-icons.svg` に星を 2 つ。ツールバーとタイトルバー用の 16×16 の `icon-desk-tb-mirror` と、ダイアログ内用の 32×32 の `icon-desk-mirror-add`（16×16 を 2 倍で出すとドットが周りより大きく見えるため、32×32 で描き直した）。
- `desktop-dialog.css` に Win95 風のテキスト入力 `.desk-text-input` とダイアログのレイアウト。
- タブの無いダイアログなので、コントロール パネルのタブページ用の内側の枠（`.desk-dialog-body` の縁）は外し、地の上に直接並べた。

## 検証

- デモ環境で、ダイアログの表示（通常幅・ナロー幅）、未ミラーの鍵の追加（ダイアログが閉じてステータスバーに件数とリレーの送信結果が出る）、ミラー済みの鍵（ダイアログ内に「すべてミラー済み」）を確認した。
- `cargo fmt --all --check`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace dashboard` が通る。
- README の Desktop 画面の素材（`docs/assets/dashboard-desktop.webp`・`.png`）を「ミラー」ボタン入りで撮り直した。撮影スクリプトはログインリンクを開いたあと「ログイン」を押す手順が抜けていて止まったので、フォームを送信してから進むように直した。
