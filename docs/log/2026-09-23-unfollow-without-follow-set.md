# 2026-09-23 Follow Set が見つからない間の unfollow 表示を直し、デモの seed を API 経由にする

## きっかけ

デモ環境で、Sites 画面の `Unfollowed but still stored` に「次にミラーリストを更新するときに削除されます」と出たサイトが、いつまで経っても消えなかった。

## 分かったこと

- agent の `refresh_follow_set` は Follow Set が決まらない tick では `remove_unfollowed` まで進まずに return する。一方で `/api/sites` と `swing sites` は、Follow Set が無いと対象の pubkey を空として扱うので、state にある全アカウントが unfollowed に並ぶ。注記は `remove_on_unfollow` しか見ていなかったので、Follow Set が無いときも「次で消える」と出ていた。
- state に保存した Follow Set は `is_follow_set_of`（今の鍵と `mirror_set`）で絞るので、鍵か `mirror_set` を変えて起動するとこの状態になり、新しい Follow Set ができるまで続く。本番でも起こる。relay が Follow Set を失っただけなら保存した版を使って再送するので、この状態にはならない。
- デモで `SWING_MIRROR_SET` を書き換えて mirror を再起動して再現した。

## 決めたこと

- **Follow Set が見つからないときに自動で消す処理は入れない。** `mirror_set` を打ち間違えただけで、ミラーしていたサイトが全部消えてしまうため。削除は保留したままにして、表示とログで状況を正しく伝えるだけにした。
- Web・CLI の注記は `remove_on_unfollow = true` かつ Follow Set が見つからないときだけ切り替える。`false` のときは元から消さないので、今までの注記のままでよい。判定には `/api/sites` に既にある `follow_set.found` を使い、DTO は変えていない。
- agent のログは、state にアカウントが残っているときだけ `stored_accounts` と「鍵か `mirror_set` を変えたか」という問いを添える。初回起動時（state が空）は今までどおりの文言にした。

## seed の修正

直前のコミットで `swing mirror add` がダッシュボード API のクライアントになり、`publish` も `[kubo].managed` の既定値（`true`）と `SWING_IPFS_API` がぶつかるようになったので、`demo.sh up --seed` は `[ipfs].api conflicts with [kubo].managed = true` で失敗していた。

- `seed` サービスに `SWING_KUBO_MANAGED=false` を渡した。
- 自分の Follow Set は `mirror` コンテナの中で `swing mirror add` を実行して作る。
- 他の参加者の分は、`seed` コンテナの中で使い捨ての `swing up` を立ててから `swing mirror add` を実行し、`swing stop` で止める。この `swing up` は mirror と同じ Kubo と relay を使うので、`SWING_MFS_ROOT=/swing-seed` で MFS を分け（分けないと garbage collection で mirror の保存物を消してしまう）、`SWING_MAX_UPDATE_SIZE=0` で何も保存させない（保存しなければレプリカ報告も出ないので、デモのレプリカ数が変わらない）。署名だけする小さなツールを seed のイメージに足す案もあったが、製品の CLI だけで済むこちらを選んだ。
- `docker/demo/README.md` の深さの表では heidi を深さ 2 に置いていたが、実際は eve（深さ 2）からしか辿れないので深さ 3 だった。`swing webring` の結果に合わせて直した。

## 検証

- `cargo fmt` / `cargo clippy --all-targets -j 3 -- -D warnings` / `cargo test -j 3`（358 passed）を通した。
- `demo.sh down` → `demo.sh up --seed` が最後まで通ることを確認した。`/api/sites` では alice・bob・carol の 4 サイトが保存済みで、レプリカ数は mirror の分だけ（使い捨ての `swing up` からの報告は無い）。`swing webring --depth 5` の結果は README の表と同じになった。
- `SWING_MIRROR_SET` を書き換えて mirror を再起動し、agent の warn（`stored_accounts=3`）、`swing sites` の見出し、Sites 画面の注記が新しい文言になることを確認した。確認のあとで元に戻した。
