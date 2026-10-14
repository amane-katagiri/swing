# ルートのブロックが取れなかったサイトを次の poll で取り直す

アンカーの広告が出る前にサイトを公開すると、ミラーする側は作者のノードにもアンカーにも届かず、取得の前の `files/stat /ipfs/<cid>`（ルートのブロックだけを取る。60 秒）が失敗する。この失敗は取得の失敗と同じ扱いで、試行から `min_update_interval`（既定 1 時間）が過ぎるまで取り直さなかった。そのため、直後にアンカーとつながってアンカーが版を持っていても、1 時間近く落ちてこなかった。

## 決めたこと

- `files/stat` の失敗では、取得の試行の記録を取り消す（`Attempts::forget_attempt`）。次の poll（既定 5 分）でまた試す。拒否の記録は残す。
- `dag/export` の取得の失敗やブロックの欠けは、今までどおり `min_update_interval` に 1 回まで。これらは大きなデータを取り始めた後の失敗で、繰り返すと帯域を使う。`files/stat` はルートのブロック 1 つで、失敗しても 1 回あたり最大 60 秒待つだけなので、毎 poll 試しても安い。
- アンカーを選び直したときに間引きの記録を消す案もあった。アンカーに限らず、作者のノードが後から届くようになった場合も同じく遅れるので、`files/stat` の失敗を間引かない方を選んだ。

## 確かめたこと

- Kubo 0.36 以降の bitswap は want の broadcast を一部のピアに絞る。実行中に `swarm/peering/add` で足したアンカーにも届くかを boxo の `bitswap/client/internal/peermanager/peerwantmanager.go` で確かめた。`connMgr.IsProtected(peerID, peering.ConnmgrTag)` のピアには常に送る。peering サービスは設定でも実行中でも、足したピアをこのタグで保護する。そのためアンカーにつながっていれば、want はアンカーに届く。
- `src/agent/store/tests.rs` に、`min_update_interval = 3600` で `files/stat` が 1 回失敗した後、次の処理で取得して保存するテストを足した。
