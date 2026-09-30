# セキュリティレビューの差分を読んだ後の対応

前日のセキュリティレビュー（[2026-09-29](2026-09-29-security-review-and-cleanup.md)）の差分を読み、質問への答えから決めたことを入れた。

## 決めたこと

- Desktop 画面だけで使う素材（`/desktop-page.html`・`/desktop-page.css`・`/desktop-banner`・`/mascots/` の下）もログインを要るようにした。これまで認証は `/api/*` だけで、画面の素材はまとめて公開していたが、公開にする理由があって決めたものではなかった。リバースプロキシで外に出すと、運営者のリンク集やマスコットがログインせずに見えていた。
  - iframe・`<img>`・CSS から読み込むので `X-Swing-Dashboard` ヘッダは送れない。GET/HEAD では cookie か bearer トークンだけで認証する。
  - 同じホストの別ポート（Kubo のゲートウェイで開いた peer の HTML など）にも `SameSite=Strict` の cookie は送られるので、`<img>` で埋め込まれないよう、返すときに `Cross-Origin-Resource-Policy: same-origin` を付ける。
  - ログイン画面の前に読み込むもの（`index.html`・同梱の JS/CSS・フォント・favicon・`/custom.css`）は公開のまま。`/custom.css` は `index.html` が読み込み、ログイン画面にも効くため。
  - これまで `index.html` の iframe の `src` とマスコットの読み込みはページを開いた時点で走っていたので、ログインの前にこれらを要求して 401 を受けないよう、ログインを確かめてから読み込むようにした。
- Docker 構成の Kubo（管理外）の RPC には認証を付けない。compose の内部ネットワークにいるのが `ipfs` と `mirror` だけで、ホストにも公開していないため。architecture の注意書きのままにする。
- MFS のパスは `mfs::site_name` と RPC のクエリで 2 回 percent-encode される。これが実 Kubo で正しく往復することを確かめる `#[ignore]` の統合テスト（`tests/mfs_kubo_integration.rs`）を足した。空白・`!`・`%`・`/`・`.`/`..`・日本語・最大長の `d` を、書き込み・一覧・読み戻し・削除まで通す。符号化の不具合は見つからなかった。片方の符号化を外すとテストが失敗することも確かめた。
- publish の保護は最低限の対策にとどまる（SWING 自身の秘密とサイトの外へのシンボリックリンクだけを止める）。一般的な対策のうち、「公開前に前の版から増えたファイルを見せる」と「site-guide で秘密スキャナを紹介する」を todo に積んだ。「ミラーしているサイトをサイト単位で消す操作」も todo に積んだ。
- README・site-guide の「その下にある」を「そのディレクトリの中にある」に言い換えた。実装済みだった NIP-05 の IPv6 の todo を消した。

## 検証

- 保護した素材は未認証なら GET・HEAD とも 401、cookie（ヘッダ無し）と bearer なら 200 で CORP ヘッダが付く。ログイン画面の素材は未認証でも 200。
- デモ環境で、ログインの前は Desktop の素材を要求しないこと、ログイン後にリンク集・バナー・マスコットが出ること、cookie を消すとログイン画面に戻ることを確かめた。
- MFS の往復テストは Kubo 0.43.1 で通る。
