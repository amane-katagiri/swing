# 起動時に Desktop 画面を出す設定（コントロール パネルの「システム」タブ）

ハッシュの無い URL で開いたときの画面を Sites ではなく Desktop にできるよう、Desktop 画面の「コントロール パネル」に「システム」タブを足した。

## 決めたこと

- 設定はブラウザごとの `localStorage["swing:desktop:startup"]`（`"1"`/`"0"`）に置く。ほかの Desktop 画面の設定と同じくサーバの設定には持たせない。
- 既定の画面を決める `app.js::currentRoute` の落とし先を、この値から求める（`desktop-system-settings.js::startupView()`）。知らないハッシュやログイン済みでの `#/login` も同じ落とし先になる。
- 実際に働く項目が 1 つだけだとタブが寂しいので、Win95 のシステムのプロパティ風の架空の項目（起動音、コンピュータの役割、ドラッグ中の表示、仮想メモリ、モデムの速度、テレホーダイの時間外の警告など）を無効の状態で並べた。HTML で `disabled` にしてあるだけで、JS からは触らず保存もしない。
- 無効のチェックボックスとラベルの見た目（地の色・灰色の印・浮き彫りの文字）はダイアログの汎用部品として `desktop-dialog.css` に置き、タブ固有のもの（ラベルとコンボボックスの行）は `desktop-system-settings.css` に分けた。

## 確かめたこと

- デモ環境で「システム」タブの表示を確かめた。
- チェックを入れて OK すると保存され、`/` を開き直すと Desktop 画面になる。外すと Sites に戻る。
- `cargo fmt --all --check`・`cargo clippy --workspace --all-targets -- -D warnings`・`cargo test --workspace` が通る。
