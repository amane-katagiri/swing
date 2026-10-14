# 外からの到達性を表示する

NAT の内側の参加者が多い前提で、到達性を上げる最初の一歩として、自分の Kubo にインターネットから直接つながるか（AutoNAT V2 の判定）を参加者に見せるようにした。届かないと分かれば、swarm のポート（既定 4001 の TCP と UDP）を外から受けると配りやすくなることを案内できる。アンカー参加者への常時接続や PeerID の告知などの次の段階は [`docs/todo.md`](../todo.md) に積んだ。

## Kubo の RPC

SWING が同梱する Kubo v0.43.1 の `POST /api/v0/swarm/addrs/autonat`（CLI は `ipfs swarm addrs autonat`）を使う。

- 応答の型は `core/commands/swarm_addrs_autonat.go` の `autoNATResult`（タグ `v0.43.1`、コミット `acc53c093810a8fe9a4bc2f023f0b1e0c1d629b5`）:
  `reachability`（`Public`・`Private`・`Unknown`。go-libp2p の `network.Reachability` の文字列）、`nat`（`omitempty`。`cgnat`・`double-nat`）、`reachable`・`unreachable`・`unknown`（`omitempty` の multiaddr 文字列の配列）。
- アドレスの情報が無いときの応答は `{"reachability":"Private"}` だけだった（`ipfs/kubo:v0.43.1` のコンテナで確認）。空のリストは省かれるので、どのリストも無くてよい形で読む。
- `nat` は `core/node/libp2p/cgnat.go` の `DetectNAT` の推定。UPnP などで分かったルーターの外側のアドレスが、自分のインターフェースに無い `100.64.0.0/10` なら `cgnat`、プライベートアドレスなら `double-nat`。`Public` のときは常に空。依頼の時点の想定には無かったが、CGNAT ではルーターでポートを転送しても届かないので、案内を変えるために API にも出すことにした。
- AutoNAT V2 は daemon の起動から判定までしばらくかかり、それまでは `Unknown`。
- daemon が `--offline` だと `this command must be run in online mode` で 500 を返す（デモ環境で確認）。

## 作ったもの

- `ipfs::IpfsClient::autonat`（タイムアウト 10 秒）と `ipfs::parse_autonat`。知らない `reachability` は `Unknown`、知らない `nat` は無しとして読む。
- `GET /api/network`（`dashboard::api::network`）。`{reachability, nat, reachable, unreachable, unknown, guide}`。ほかの Kubo を使う API と同じく、agent の準備前は 503、Kubo の失敗は 502。Kubo の持っている判定を読むだけなので軽い。
- `guide` は `docs/guide/operation.md#外から届くか` の版付きの URL（`dto::REACHABILITY_GUIDE_URL`）。publish のリンクの確認が手引きの URL を API から返しているのに合わせ、画面側で URL を組み立てない。見出しが手引きにあることはテストで確かめる。
- `swing status` の先頭に `Reachability:` の行と、`private`・`unknown` のときの案内を出す。`/api/status` を先に呼び、それが失敗したら今までどおり止まる。`/api/network` の失敗は理由を添えて `unknown` と出すだけで、問題に数えず終了コードも変えない（監視に使っている人の終了コードを、到達性で変えないため）。
- Settings 画面に「ネットワーク」パネル（`web/network.js`）。判定を `swing-status` の 1 行（届く・届かない・判定中）で出し、説明・CGNAT の注記・手引きへのリンク（届くとき以外）・アドレスごとの表を続ける。画面を開くたびと「再読み込み」ボタンで呼ぶ。AutoNAT の判定を呼び出しで進めることはできないので、ボタンは「再確認」ではなく既存の「再読み込み」の文言にした。アドレスの表は折りたたまない（既存の画面に折りたたみの部品が無いため）。
- 手引き `docs/guide/operation.md` に「外から届くか」の節。Kubo が自動でしていること（UPnP・NAT-PMP、AutoNAT、中継とホールパンチ（DCUtR）、QUIC のほうが通りやすいこと）、公開の中継はデータを運ばないので届かないノードどうしはホールパンチに失敗するとやりとりできないこと、サイトごとに届く参加者が 1 人いれば広がること、ポートの転送・UDP の許可・Docker のブリッジでは UPnP が効かないこと・CGNAT を書いた。`docker.md`・`usage.md`・リリースの `README.md` からも触れた。

## 検証

- `cargo fmt --all`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`。テストは応答の解析（最小の形・リストと `nat` のある形・知らない値）、`/api/network` の 503・200・502、`swing status` の到達性の行。
- デモ環境（`docker/demo/demo.sh up --seed`）で `swing status` を実行し、Kubo が `--offline` なので `Reachability: unknown (swarm/addrs/autonat failed: 500 …online mode…)` の後に今までの表示が続き、終了コードが 0 のままであることを確かめた。
- 同じ環境で Settings 画面を開き、失敗の表示と、`/api/network` の応答をブラウザ側で差し替えた `public`・`private`（CGNAT あり）・`unknown` の表示を、ライトとダーク、幅 390px で確かめた。隣の「リソース使用量」「プロセス」のパネルとの間隔は既存のパネルどうしと同じ。

## 追記: オフラインの Kubo

デモ環境のように Kubo が `--offline` で動いていると、`/api/network` が 502 のエラーになり、画面に Kubo の JSON がそのまま出ていた。オフラインは失敗ではなく状態なので、独立した判定 `offline` にした。

- Kubo v0.43.1 の `core/commands/root.go` の `ErrNotOnline`（`this command must be run in online mode. Try running 'ipfs daemon' first`）の前半 `this command must be run in online mode` を本文に含むエラーだけを `offline` として扱い（`IpfsClient::autonat`）、ほかの失敗は今までどおり 502。
- `/api/network` は 200 で `reachability: "offline"`・`nat: null`・空のリストを返す。
- `swing status` は `Reachability: offline` と、ほかの参加者と何もやりとりしない旨の 1 行。問題には数えない。
- Settings 画面は「外から: オフライン」を `swing-status` の `empty`（中立の表示）で出し、説明だけを添えてリンクと表は出さない。
- テスト: Kubo の client（オフラインのエラーと別のエラー）、`/api/network`（200 の `offline` と 502）、`swing status` の行。デモ環境の Settings 画面をライトとダークで確かめた。

## 追記: パネルの文言と色

- 「外から: 届く」のような短いラベルは何を指すか分かりにくいので、判定の行を文にした。`public` は「このノードは外から直接アクセスできます。」（`ok`）、`private` は「このノードは外から直接アクセスできません。」（`warn`）、`offline` は「このノードはオフラインです。」（`error`）、`unknown` は「外からこのノードにアクセスできるか判定中です。」（`info`）。
- 判定中とオフラインも枠付きで目立つようにした。`empty` と `loading` は枠が無く、ほかの段落に紛れるため。判定中のための枠付きの中立の表示が無かったので、`swing-status` に `data-kind="info"` を足した。色は新しいトークン `--swing-info`・`--swing-info-soft`（ライトは `--swing-accent` と同じ青、ダークは `#8ab4f8`）。ダークの `--swing-accent` は黄緑で、`ok` の緑と見分けにくいため。
- 再読み込みのボタンは、パネル末尾から見出しの右のアイコンボタンに移した（Publish 画面の「自分のサイト」と揃える）。「自分のサイト」は見出しの下線を消しているが、Settings のほかのパネルは下線があるので、`h2` に `swing-heading-row` を付けてボタンを中に置き、下線を全幅に残した。手引きへのリンクは動作性の確認の「修正ガイド」と名前を揃えた。
- アドレスの表の判定は文字だけだと見分けにくいので、`swing-badge` に `data-reach` を足して色付きのバッジにした。「OK / NG」にしなかったのは、状態が 3 つあり「未確認」（AutoNAT がまだ試していない）がどちらにも当たらないことと、届かないのは異常ではなくホールパンチ頼みというだけなので、エラーに見せたくないため。列の見出しは「判定」では何の判定か分からないので「外からの接続」にした。
- スマホ幅ではアドレスの表が横にはみ出し、バッジの列はスクロールしないと見えない。アドレスを途中で折り返すとかえって読みにくいので、はみ出しは許容した（リソース使用量の表も同じ）。
