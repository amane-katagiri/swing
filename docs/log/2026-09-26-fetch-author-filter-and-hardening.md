# 取得結果の作者の照合と 3 件の堅牢化

relay から取った結果を要求した条件と照合していない取得関数があったのを直し、あわせて内蔵 gateway の応答ヘッダ待ち・`report_ttl` の上限・アップロードの 413 の判定を直した。いずれもプロトコル（`protocol.md`）は変えていない。

## 要求していない作者のイベントを捨てる

nostr-sdk 0.45 は受信したイベントが REQ のフィルタに一致するかを確かめない（`verify_subscriptions` が既定で無効）。`fetch_site_events` は relay の結果を素通しで返し、`fetch_follow_sets` はイベント自身の `pubkey` と比べるだけだったので、フィルタを無視して他人のイベントを返す relay があると、webring のクロールに偽のノード・辺が入って `MAX_CRAWL_NODES` の枠を食い、`swing sites`・`swing replicas`・`/api/publish/sites` にも混ざっていた。agent の保存は `submit` の対象判定で弾かれるので影響は無かった。

決めたこと:

- `fetch_site_events` は kind と作者（要求した `authors` に含まれるか）、`fetch_follow_sets` は作者で、取得直後に捨てる。単数版 `fetch_follow_set` が自分の公開鍵と照合していたのと同じ考え方で、呼び出し側ごとに照合を足すより取得関数の中で閉じる方が漏れが無い。
- 同じファイルの他の取得も見直した。`fetch_replica_reports` は作者ではなく座標（`#a`）で取るので、kind と「`a` タグのどれかが要求した座標に一致する」ことで照合する。これまでも集計側（`replicas::collect` の座標での引き当て）で結果的に無視されていたが、`mirror` 側の経路も含めて取得関数の段階で揃えた。
- `fetch_follow_set_authors_referencing` は作者を指定しない `#p` の取得なので、既存の「`p` タグのどれかが要求した相手に含まれる」と `is_follow_set_of`（kind・`d`・署名）の照合のままにした。
- `fetch_own_reports` は呼び出し側（`agent::replicas`）が作者が自分であることを確かめており、kind は `parse_replica_report` が確かめるので変えていない。`ReportRelay` はテストで差し替えるトレイトなので、照合は実装ではなく呼び出し側にある方が差し替えても効く。
- 照合に使う集合は `HashSet` にした。`authors` はクロールで最大 1000 件、取得は最大 20,000 件になり得る。

検証: nostr-sdk の `LocalRelay` に、受け取ったフィルタを空のフィルタに置き換える `QueryPolicy` を付けて「フィルタを無視する relay」を作り、2 人分のサイトイベント・Follow Set・レプリカ報告を入れた。素の取得では要求外の作者のイベントが返ってくることを確かめたうえで、`fetch_site_events`・`fetch_follow_sets`・`fetch_replica_reports` が要求した側だけを返し、`fetch_follow_set_authors_referencing` が名指ししていない Follow Set を拾わないことを確かめた（公開 relay には接続しない）。

## gateway の応答ヘッダ待ちのタイムアウト

内蔵 gateway の HTTP クライアントは接続タイムアウト（10 秒）だけで、`send()` は upstream が応答ヘッダを返すまで無期限に待っていた。全体タイムアウトを付けないのは大きなファイルの配信を打ち切らないため（[architecture のドキュメント整理](2026-09-25-architecture-docs-cleanup.md)）なので、その方針は保ったまま、ヘッダが返るまでだけを区切る。

決めたこと:

- `proxy` の `send()` を `tokio::time::timeout(UPSTREAM_HEADER_TIMEOUT, …)` で包み、時間切れは `504 Gateway Timeout`（空ボディ）と warn。ヘッダを受け取った後のボディのストリーミングは対象外。接続や送信のエラーは従来どおり 502 で、原因の違い（落ちている／応答しない）がステータスで分かる。
- 値は 60 秒。upstream の Kubo の Gateway は `Gateway.NoFetch=true` でローカルのブロックだけを返すので、正常なら応答ヘッダは DNSLink の解決を含めても数秒以内に返る。60 秒はそれに対して GC や大きな pin の最中の遅れを吸収できる余裕を持たせた値で、なおかつ前段に置くことの多いリバースプロキシの既定（nginx の `proxy_read_timeout` 60 秒、Cloudflare の 100 秒）以下なので、前段より先に gateway 自身が 504 を返して warn を残せる。
- テストから短くできるよう、`router` は `router_with_header_timeout` に定数を渡すだけにした。設定値にはしていない（変えたい理由がまだ無い）。

検証: accept するだけで何も返さない TCP リスナーを upstream にし、タイムアウトを 200 ミリ秒にした gateway が 504 と空ボディを返すことを確かめた。

## `report_ttl` の上限

レプリカ報告は `report_ttl / 2` ごとに出し直し、受信側は `created_at` から `MAX_REPORT_AGE`（7 日）を過ぎた報告を数えない。`report_ttl` の検証は `report_ttl / 2 > poll_interval` の下限だけだったので、14 日を超えると出し直しの間に必ず数えられない期間ができ、7 日を超えるだけでも `expiration` が受信側の打ち切りより後になって意味を失っていた。

決めたこと:

- 設定の検証に `report_ttl <= MAX_REPORT_AGE` を足した。エラー文言は既存の `report_ttl must be more than twice poll_interval` に合わせて `report_ttl must be at most 7d`（値は定数から `format_duration_secs` で作る）。上限を 14 日ではなく 7 日にしたのは、`expiration` を受信側が数えなくなる時刻より先に置く意味が無いため。
- 既存の設定で `report_ttl` を 7 日より長くしていると、この変更で起動時にエラーになる。移行や読み替えは入れていない。
- `expiration` の `created_at + ttl` を `saturating_add` にした。上限を付けたので実際には溢れないが、`created_at` は記録からも来るので足し算の側でも閉じておく。
- 設定カタログの説明文に上限を足し、`swing.example.toml` と `.env.example` を生成し直した。

検証: `SWING_REPORT_TTL=7d` は通り、`604801s` は `report_ttl must be at most 7d` でエラーになることを確かめた。生成物のテスト（`swing_example_toml_matches_generator`・`env_example_matches_generator`）も通した。

## アップロードの 413 の判定

`multipart_error_to_api` はエラーチェーンの Display に `length limit` が含まれるかで 413 を判定していた。axum 0.8 の `MultipartError::status()` がボディの上限超過（multer の `FieldSizeExceeded`・`StreamSizeExceeded` と、`DefaultBodyLimit` の `LengthLimitError`）を `413 Payload Too Large` にするので、それを使うようにし、文字列一致とその理由のコメントを消した。

決めたこと:

- `status()` が 413 なら 413、それ以外は従来どおり 400。
- `status()` が 500 を返すのは、ボディのストリームの読み取り自体が失敗した場合（クライアントの切断・接続のリセットなど）と multer の内部エラーで、前者が実際に起こるほぼすべて。サーバの不具合ではなくリクエスト側の事情なので 500 にはせず、400 のままにした。どのみち切断ならレスポンスはクライアントに届かず、500 を返すと監視やログでサーバ側の障害と取り違えられる。この理由だけをコメントに残した。

検証: 既存の回帰テスト `upload_over_the_body_limit_is_rejected_with_json_413` が JSON の 413 を返すことを確かめた。

## 全体の検証

- `cargo fmt --all --check`
- `cargo clippy -j 2 --workspace --all-targets -- -D warnings`
- `cargo test -j 2 --workspace`
