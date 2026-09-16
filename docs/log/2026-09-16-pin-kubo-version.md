# 2026-09-16 Kubo のバージョン固定

## 問題

compose の Kubo が `ipfs/kubo:latest` で、再作成のたびに新しいバージョンに上がりうる。SWING は Kubo RPC のエラー文面（`file does not exist`）、`files/rm` が失敗時も 200 を返すこと、JSON の形、MFS と GC の挙動などに依存しており、上がったときに気づかず壊れる。特に文面の判定が外れると、`files/stat` の「無い」がエラー扱いになり、`reconcile` が「確認できないので残す」側に倒れて、MFS から消えた版を取り直さなくなる。止まらずに警告を出し続けるだけなので気づきにくい。

## 決めたこと

| 決定 | 理由 |
|---|---|
| Kubo 以外の IPFS 実装は想定しない | `/api/v0` の RPC を実用的に提供しているのは Kubo だけで、MFS も Kubo の機能 |
| compose を `ipfs/kubo:v0.43.1` に固定する | これまでの検証（MFS の保護、offline の挙動）はすべて 0.43.1 で行っている。手元の `latest` と同じダイジェスト |
| 統合テストの手順も同じタグにする | テストした版と本番の版をそろえる |
| Kubo を上げる手順を architecture に書く | 統合テストを新しい版で通してからタグを上げる。`--migrate=true` で repo が移行されると戻せないことがある |

## 検証

- `ipfs/kubo:v0.43.1`（`IPFS_PROFILE=test`）で `kubo_integration` の 7 件と `agent_stores_and_removes_through_real_kubo` が通った。
