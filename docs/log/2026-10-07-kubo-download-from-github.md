# Kubo の取得先を GitHub のリリースにする

`install.sh` が Kubo を取得する先を `dist.ipfs.tech` から GitHub の `ipfs/kubo` のリリースに変え、ドキュメントの Kubo の入手先も同じにした。

## 決めたこと

- `dist.ipfs.tech` は 2026-10-02 から接続を受け付けず、少なくとも 10-07 時点でもつながらない。Kubo を使う別のプロジェクトでも同じ日から CI が落ちていた。公式の告知は見つからず、いつ戻るか分からない。このままだと Linux の `install.sh` が Kubo の取得で失敗する。
- `dist.ipfs.tech` を先に試して GitHub に落とすフォールバックにはしない。落ちている間は毎回接続のタイムアウトを待つことになる。GitHub のリリースには同じ名前のアーカイブがあり、SHA-512 も同じなので、取得先を 1 つにして困ることはない。Windows のインストーラー（`build.ps1`）はもともと GitHub から取っている。
- 固定している SHA-512 は変えない。GitHub のアーカイブの値が `KUBO_SHA512_AMD64`・`KUBO_SHA512_ARM64` と一致することを確かめた。
- v0.1.0 はまだ公開していないので、版を上げずにタグを打ち直す。

## 変えたもの

- `packaging/linux/install.sh` の `KUBO_BASE_URL`。
- `docs/release/README.md`・`docs/guide/install.md` の Kubo の入手先のリンク、`docs/architecture/install-sh.md`・`docs/architecture/kubo.md` の取得先と SHA-512 の出どころ。

## 検証したこと

- `packaging/linux/test-install.sh` がすべて通り、`shellcheck` が警告なしで通る。
- 新しい `KUBO_BASE_URL` から `curl --proto '=https' --tlsv1.2` で amd64 のアーカイブを取得でき（リダイレクト先も HTTPS）、SHA-512 が固定値と一致した。
