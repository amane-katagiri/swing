# 2026-09-18 `publish` で IPFS だけのサイトを公開できるようにする

## 問題

- `swing publish` は `--url` が必須で、`--site` を省略すると URL のホスト名を `d` にしていた。HTTP で配信していない、IPFS だけのサイトを publish できなかった。
- プロトコル（protocol.md）では `url` タグはもともと任意で、受信側も `url` なしのイベントを扱える。制約は CLI だけにあった。
- `--site` を指定すると `--url` は検証されず、http / https でない値もそのままタグになっていた（受信側で黙って捨てられる）。

## 決めたこと

| 決定 | 理由 |
|---|---|
| `--site` を必須、`--url` を任意にする | `d` はドメインでなくてもよく、URL のホストとは別の識別子。ドメインのルートを管理している前提で使う値なので、URL から暗黙に決めず明示させる |
| `--site` を省略したときに URL のホスト名を使う動作は残さない | 後方互換のフォールバックは入れない方針（オタクくんに確認済み）。`--site` 省略時は clap が必須引数のエラーを出す |
| publish 時に `d` と `url` を受信側と同じ規則で検証する | 受信側で拒否・無視されるイベントを出さない。`nostr::validate_d_tag` と `nostr::valid_http_url` を使い回す |
| protocol.md は変えない | `url` はすでに任意 |

## 作ったもの

- `main.rs`: `--site` を `String`、`--url` を `Option<String>` にした。
- `publish.rs`: `host_from_url` とそのテストを削除。`d` と `url` を先に検証し、`Site: <d>` と（あれば）`URL:` を表示する。`url` がなければ `url` タグを付けない。
- `nostr::valid_http_url` を `pub` にした。
- architecture.md の CLI 書式と publish の説明、README の例と説明、`docs/examples/publish.sh`（`url` に空文字を渡すと `url` タグを付けない）を更新した。todo から削除。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`（182 passed, 11 ignored）。
- `swing publish --help` の表示と、`--site` を省略したときに必須引数エラーになることを確認。
