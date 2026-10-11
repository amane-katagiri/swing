# SWING 絵文字フィンガープリント 第 1 版

Nostr の公開鍵から 4 つの絵文字を決める方法の定義。SWING のダッシュボードと swing-connect はこの方法で同じ公開鍵に同じ絵文字を出す。ほかのクライアントもこの文書のとおりに求めれば、同じ絵文字になる。

この文書の「しなければならない」「してはならない」は必須の要件、「してよい」は任意を表す。

## 1. 目的と限界

- 似た名前のアカウントを見分けたり、離れた相手と画面を見比べたりするための、目で見る目印。
- **本人の証明ではない。** 情報量は 32 ビットしかなく、狙ったフィンガープリントの鍵は総当たりで作れる。アカウントを確かめる手段として使ってはならない。確かめるのは公開鍵（npub）そのもの（と NIP-05）。
- 実装は、フィンガープリントを出す場所の近くで公開鍵も確かめられるようにするべきである。フィンガープリントだけで送金先・署名先・フォロー先などを選ばせてはならない。

## 2. 版

- この文書は第 1 版（`v1`）を定める。版は、求め方（3 章）と絵文字の表（5 章）の組を表す。
- 第 1 版の求め方と表は変えない。表の行の入れ替え・差し替えも行わない。どちらかを変えるときは新しい版を定め、別の名前（`v2` など）で区別する。
- 版はイベントやデータに載らない。異なる版の結果を並べると同じ鍵でも絵文字が違って見えるので、実装は 1 つの画面で 1 つの版だけを使うべきである。第 1 版以外を使うときは、その版を画面に示すべきである。
- SWING と swing-connect は第 1 版を使う。

## 3. 求め方

入力は Nostr の公開鍵（NIP-01 の x-only の 32 バイト）。npub・nprofile（NIP-19）や 16 進の文字列で受け取ったときは、先に 32 バイトに戻す。

1. 公開鍵の 32 バイトの SHA-256 を取る。16 進の文字列や npub の文字列をハッシュしてはならない。
2. ダイジェストの先頭 4 バイトを順に取り出し、それぞれを 0〜255 の符号なし整数とする。
3. それぞれの整数を 5 章の表の「番号」に当て、その行の絵文字を得る。

結果は、この順に並んだ 4 つの絵文字である。同じ絵文字が 2 回以上出てもよい（そのまま出す）。

## 4. 表し方

- 各絵文字は、表の 1 つのコードポイントそのものである。
- 文字列として表すとき（コピー・貼り付け・テキストでの表示・読み上げ用のラベルなど）は、各コードポイントの直後に異体字セレクタ U+FE0F を付けなければならない。表には既定がテキスト表示の文字（U+2764 など）も含まれており、U+FE0F を付けると絵文字として表示される。もともと絵文字表示の文字に付けても害はない。
- 4 つをまとめた文字列は、「コードポイント + U+FE0F」を順に区切り無しでつないだもの（8 コードポイント）とする。
- 2 つのフィンガープリントを比べるときは、U+FE0F を除いたコードポイントの並びで比べる。
- 画面上で絵文字の間を空けること、画像（SWING は Twemoji を使う）で描くこと、先頭のいくつかだけを出すこと（4 つ全部を出せない狭い場所など）は実装に任せる。先頭以外の一部だけを出してはならない。

## 5. 絵文字の表

番号はダイジェストのバイトの値。正本は「コードポイント」の列で、「文字」と「Unicode の名前」は参考。

| 番号 | コードポイント | 文字 | Unicode の名前 |
|---:|---|---|---|
| 0 | U+1F40E | 🐎 | HORSE |
| 1 | U+1F404 | 🐄 | COW |
| 2 | U+1F416 | 🐖 | PIG |
| 3 | U+1F411 | 🐑 | SHEEP |
| 4 | U+1F42A | 🐪 | DROMEDARY CAMEL |
| 5 | U+1F999 | 🦙 | LLAMA |
| 6 | U+1F418 | 🐘 | ELEPHANT |
| 7 | U+1F99B | 🦛 | HIPPOPOTAMUS |
| 8 | U+1F992 | 🦒 | GIRAFFE FACE |
| 9 | U+1F993 | 🦓 | ZEBRA FACE |
| 10 | U+1F98C | 🦌 | DEER |
| 11 | U+1F998 | 🦘 | KANGAROO |
| 12 | U+1F412 | 🐒 | MONKEY |
| 13 | U+1F415 | 🐕 | DOG |
| 14 | U+1F408 | 🐈 | CAT |
| 15 | U+1F405 | 🐅 | TIGER |
| 16 | U+1F981 | 🦁 | LION FACE |
| 17 | U+1F98A | 🦊 | FOX FACE |
| 18 | U+1F43B | 🐻 | BEAR FACE |
| 19 | U+1F43C | 🐼 | PANDA FACE |
| 20 | U+1F99D | 🦝 | RACCOON |
| 21 | U+1F9A1 | 🦡 | BADGER |
| 22 | U+1F994 | 🦔 | HEDGEHOG |
| 23 | U+1F43F | 🐿 | CHIPMUNK |
| 24 | U+1F407 | 🐇 | RABBIT |
| 25 | U+1F401 | 🐁 | MOUSE |
| 26 | U+1F987 | 🦇 | BAT |
| 27 | U+1F984 | 🦄 | UNICORN FACE |
| 28 | U+1F43E | 🐾 | PAW PRINTS |
| 29 | U+1F413 | 🐓 | ROOSTER |
| 30 | U+1F423 | 🐣 | HATCHING CHICK |
| 31 | U+1F986 | 🦆 | DUCK |
| 32 | U+1F9A2 | 🦢 | SWAN |
| 33 | U+1F427 | 🐧 | PENGUIN |
| 34 | U+1F989 | 🦉 | OWL |
| 35 | U+1F99C | 🦜 | PARROT |
| 36 | U+1F99A | 🦚 | PEACOCK |
| 37 | U+1F422 | 🐢 | TURTLE |
| 38 | U+1F40A | 🐊 | CROCODILE |
| 39 | U+1F40D | 🐍 | SNAKE |
| 40 | U+1F996 | 🦖 | T-REX |
| 41 | U+1F409 | 🐉 | DRAGON |
| 42 | U+1F438 | 🐸 | FROG FACE |
| 43 | U+1F40B | 🐋 | WHALE |
| 44 | U+1F42C | 🐬 | DOLPHIN |
| 45 | U+1F988 | 🦈 | SHARK |
| 46 | U+1F41F | 🐟 | FISH |
| 47 | U+1F421 | 🐡 | BLOWFISH |
| 48 | U+1F419 | 🐙 | OCTOPUS |
| 49 | U+1F980 | 🦀 | CRAB |
| 50 | U+1F990 | 🦐 | SHRIMP |
| 51 | U+1F41A | 🐚 | SPIRAL SHELL |
| 52 | U+1F40C | 🐌 | SNAIL |
| 53 | U+1F41D | 🐝 | HONEYBEE |
| 54 | U+1F41E | 🐞 | LADY BEETLE |
| 55 | U+1F98B | 🦋 | BUTTERFLY |
| 56 | U+1F331 | 🌱 | SEEDLING |
| 57 | U+1F33F | 🌿 | HERB |
| 58 | U+1F340 | 🍀 | FOUR LEAF CLOVER |
| 59 | U+1F341 | 🍁 | MAPLE LEAF |
| 60 | U+1F332 | 🌲 | EVERGREEN TREE |
| 61 | U+1F334 | 🌴 | PALM TREE |
| 62 | U+1F335 | 🌵 | CACTUS |
| 63 | U+1F337 | 🌷 | TULIP |
| 64 | U+1F338 | 🌸 | CHERRY BLOSSOM |
| 65 | U+1F33B | 🌻 | SUNFLOWER |
| 66 | U+1F490 | 💐 | BOUQUET |
| 67 | U+1F344 | 🍄 | MUSHROOM |
| 68 | U+1F330 | 🌰 | CHESTNUT |
| 69 | U+2600 | ☀ | BLACK SUN WITH RAYS |
| 70 | U+2601 | ☁ | CLOUD |
| 71 | U+2614 | ☔ | UMBRELLA WITH RAIN DROPS |
| 72 | U+2744 | ❄ | SNOWFLAKE |
| 73 | U+26C4 | ⛄ | SNOWMAN WITHOUT SNOW |
| 74 | U+1F300 | 🌀 | CYCLONE |
| 75 | U+1F308 | 🌈 | RAINBOW |
| 76 | U+1F319 | 🌙 | CRESCENT MOON |
| 77 | U+1F320 | 🌠 | SHOOTING STAR |
| 78 | U+2728 | ✨ | SPARKLES |
| 79 | U+1F525 | 🔥 | FIRE |
| 80 | U+1F4A7 | 💧 | DROPLET |
| 81 | U+1F30A | 🌊 | WATER WAVE |
| 82 | U+1F30B | 🌋 | VOLCANO |
| 83 | U+1F305 | 🌅 | SUNRISE |
| 84 | U+1F30D | 🌍 | EARTH GLOBE EUROPE-AFRICA |
| 85 | U+1F5FA | 🗺 | WORLD MAP |
| 86 | U+1F34A | 🍊 | TANGERINE |
| 87 | U+1F34B | 🍋 | LEMON |
| 88 | U+1F34C | 🍌 | BANANA |
| 89 | U+1F349 | 🍉 | WATERMELON |
| 90 | U+1F347 | 🍇 | GRAPES |
| 91 | U+1F353 | 🍓 | STRAWBERRY |
| 92 | U+1F352 | 🍒 | CHERRIES |
| 93 | U+1F34D | 🍍 | PINEAPPLE |
| 94 | U+1F965 | 🥥 | COCONUT |
| 95 | U+1F95D | 🥝 | KIWIFRUIT |
| 96 | U+1F951 | 🥑 | AVOCADO |
| 97 | U+1F966 | 🥦 | BROCCOLI |
| 98 | U+1F955 | 🥕 | CARROT |
| 99 | U+1F952 | 🥒 | CUCUMBER |
| 100 | U+1F33D | 🌽 | EAR OF MAIZE |
| 101 | U+1F360 | 🍠 | ROASTED SWEET POTATO |
| 102 | U+1F95C | 🥜 | PEANUTS |
| 103 | U+1F950 | 🥐 | CROISSANT |
| 104 | U+1F968 | 🥨 | PRETZEL |
| 105 | U+1F9C0 | 🧀 | CHEESE WEDGE |
| 106 | U+1F953 | 🥓 | BACON |
| 107 | U+1F356 | 🍖 | MEAT ON BONE |
| 108 | U+1F95A | 🥚 | EGG |
| 109 | U+1F373 | 🍳 | COOKING |
| 110 | U+1F354 | 🍔 | HAMBURGER |
| 111 | U+1F35F | 🍟 | FRENCH FRIES |
| 112 | U+1F355 | 🍕 | SLICE OF PIZZA |
| 113 | U+1F32D | 🌭 | HOT DOG |
| 114 | U+1F32E | 🌮 | TACO |
| 115 | U+1F32F | 🌯 | BURRITO |
| 116 | U+1F96B | 🥫 | CANNED FOOD |
| 117 | U+1F371 | 🍱 | BENTO BOX |
| 118 | U+1F359 | 🍙 | RICE BALL |
| 119 | U+1F363 | 🍣 | SUSHI |
| 120 | U+1F35C | 🍜 | STEAMING BOWL |
| 121 | U+1F35D | 🍝 | SPAGHETTI |
| 122 | U+1F95F | 🥟 | DUMPLING |
| 123 | U+1F961 | 🥡 | TAKEOUT BOX |
| 124 | U+1F361 | 🍡 | DANGO |
| 125 | U+1F366 | 🍦 | SOFT ICE CREAM |
| 126 | U+1F369 | 🍩 | DOUGHNUT |
| 127 | U+1F36A | 🍪 | COOKIE |
| 128 | U+1F382 | 🎂 | BIRTHDAY CAKE |
| 129 | U+1F9C1 | 🧁 | CUPCAKE |
| 130 | U+1F36E | 🍮 | CUSTARD |
| 131 | U+1F36B | 🍫 | CHOCOLATE BAR |
| 132 | U+1F36D | 🍭 | LOLLIPOP |
| 133 | U+1F36F | 🍯 | HONEY POT |
| 134 | U+1F37F | 🍿 | POPCORN |
| 135 | U+2615 | ☕ | HOT BEVERAGE |
| 136 | U+1F964 | 🥤 | CUP WITH STRAW |
| 137 | U+1F37C | 🍼 | BABY BOTTLE |
| 138 | U+1F374 | 🍴 | FORK AND KNIFE |
| 139 | U+1F388 | 🎈 | BALLOON |
| 140 | U+1F389 | 🎉 | PARTY POPPER |
| 141 | U+1F381 | 🎁 | WRAPPED PRESENT |
| 142 | U+1F380 | 🎀 | RIBBON |
| 143 | U+1F38F | 🎏 | CARP STREAMER |
| 144 | U+1F386 | 🎆 | FIREWORKS |
| 145 | U+2764 | ❤ | HEAVY BLACK HEART |
| 146 | U+1F3AA | 🎪 | CIRCUS TENT |
| 147 | U+1F3A0 | 🎠 | CAROUSEL HORSE |
| 148 | U+1F3A1 | 🎡 | FERRIS WHEEL |
| 149 | U+1F3A2 | 🎢 | ROLLER COASTER |
| 150 | U+1F3AB | 🎫 | TICKET |
| 151 | U+1F3A8 | 🎨 | ARTIST PALETTE |
| 152 | U+1F3AC | 🎬 | CLAPPER BOARD |
| 153 | U+1F4F7 | 📷 | CAMERA |
| 154 | U+1F3A4 | 🎤 | MICROPHONE |
| 155 | U+1F3A7 | 🎧 | HEADPHONE |
| 156 | U+1F3B5 | 🎵 | MUSICAL NOTE |
| 157 | U+1F3B8 | 🎸 | GUITAR |
| 158 | U+1F941 | 🥁 | DRUM WITH DRUMSTICKS |
| 159 | U+1F4FB | 📻 | RADIO |
| 160 | U+1F4BF | 💿 | OPTICAL DISC |
| 161 | U+1F3AE | 🎮 | VIDEO GAME |
| 162 | U+1F3B2 | 🎲 | GAME DIE |
| 163 | U+1F9E9 | 🧩 | JIGSAW PUZZLE PIECE |
| 164 | U+1F004 | 🀄 | MAHJONG TILE RED DRAGON |
| 165 | U+265F | ♟ | BLACK CHESS PAWN |
| 166 | U+2660 | ♠ | BLACK SPADE SUIT |
| 167 | U+2663 | ♣ | BLACK CLUB SUIT |
| 168 | U+1F3AF | 🎯 | DIRECT HIT |
| 169 | U+1F52E | 🔮 | CRYSTAL BALL |
| 170 | U+267E | ♾ | PERMANENT PAPER SIGN |
| 171 | U+303D | 〽 | PART ALTERNATION MARK |
| 172 | U+2668 | ♨ | HOT SPRINGS |
| 173 | U+26BD | ⚽ | SOCCER BALL |
| 174 | U+1F3C8 | 🏈 | AMERICAN FOOTBALL |
| 175 | U+1F3F8 | 🏸 | BADMINTON RACQUET AND SHUTTLECOCK |
| 176 | U+1F3D2 | 🏒 | ICE HOCKEY STICK AND PUCK |
| 177 | U+1F945 | 🥅 | GOAL NET |
| 178 | U+1F94C | 🥌 | CURLING STONE |
| 179 | U+1F3B3 | 🎳 | BOWLING |
| 180 | U+1F3BF | 🎿 | SKI AND SKI BOOT |
| 181 | U+1F6F9 | 🛹 | SKATEBOARD |
| 182 | U+1F94F | 🥏 | FLYING DISC |
| 183 | U+1F3A3 | 🎣 | FISHING POLE AND FISH |
| 184 | U+1F3C6 | 🏆 | TROPHY |
| 185 | U+1F455 | 👕 | T-SHIRT |
| 186 | U+1F9E6 | 🧦 | SOCKS |
| 187 | U+1F45F | 👟 | ATHLETIC SHOE |
| 188 | U+1F460 | 👠 | HIGH-HEELED SHOE |
| 189 | U+1F3A9 | 🎩 | TOP HAT |
| 190 | U+1F393 | 🎓 | GRADUATION CAP |
| 191 | U+1F451 | 👑 | CROWN |
| 192 | U+1F453 | 👓 | EYEGLASSES |
| 193 | U+1F48D | 💍 | RING |
| 194 | U+1F48E | 💎 | GEM STONE |
| 195 | U+1F484 | 💄 | LIPSTICK |
| 196 | U+1F45C | 👜 | HANDBAG |
| 197 | U+1F9F3 | 🧳 | LUGGAGE |
| 198 | U+1F6AA | 🚪 | DOOR |
| 199 | U+1F6CB | 🛋 | COUCH AND LAMP |
| 200 | U+1F5BC | 🖼 | FRAME WITH PICTURE |
| 201 | U+1F6C1 | 🛁 | BATHTUB |
| 202 | U+1F9FC | 🧼 | BAR OF SOAP |
| 203 | U+1F9F9 | 🧹 | BROOM |
| 204 | U+1F9FA | 🧺 | BASKET |
| 205 | U+1F9F8 | 🧸 | TEDDY BEAR |
| 206 | U+1F9F6 | 🧶 | BALL OF YARN |
| 207 | U+1F4A1 | 💡 | ELECTRIC LIGHT BULB |
| 208 | U+1F526 | 🔦 | ELECTRIC TORCH |
| 209 | U+1F50C | 🔌 | ELECTRIC PLUG |
| 210 | U+1F4BB | 💻 | PERSONAL COMPUTER |
| 211 | U+1F5B1 | 🖱 | THREE BUTTON MOUSE |
| 212 | U+231A | ⌚ | WATCH |
| 213 | U+23F0 | ⏰ | ALARM CLOCK |
| 214 | U+1F6D2 | 🛒 | SHOPPING TROLLEY |
| 215 | U+1F4E6 | 📦 | PACKAGE |
| 216 | U+1F4EE | 📮 | POSTBOX |
| 217 | U+1F4DA | 📚 | BOOKS |
| 218 | U+1F4DC | 📜 | SCROLL |
| 219 | U+270F | ✏ | PENCIL |
| 220 | U+2702 | ✂ | BLACK SCISSORS |
| 221 | U+1F4CE | 📎 | PAPERCLIP |
| 222 | U+1F4D0 | 📐 | TRIANGULAR RULER |
| 223 | U+1F9EE | 🧮 | ABACUS |
| 224 | U+1F9ED | 🧭 | COMPASS |
| 225 | U+1F9F2 | 🧲 | MAGNET |
| 226 | U+1F9EA | 🧪 | TEST TUBE |
| 227 | U+1F9EC | 🧬 | DNA DOUBLE HELIX |
| 228 | U+269B | ⚛ | ATOM SYMBOL |
| 229 | U+1F52D | 🔭 | TELESCOPE |
| 230 | U+1F528 | 🔨 | HAMMER |
| 231 | U+1F5DC | 🗜 | COMPRESSION |
| 232 | U+1F9F1 | 🧱 | BRICK |
| 233 | U+1F3E0 | 🏠 | HOUSE BUILDING |
| 234 | U+1F3E2 | 🏢 | OFFICE BUILDING |
| 235 | U+1F3ED | 🏭 | FACTORY |
| 236 | U+1F3EF | 🏯 | JAPANESE CASTLE |
| 237 | U+1F5FC | 🗼 | TOKYO TOWER |
| 238 | U+1F3DF | 🏟 | STADIUM |
| 239 | U+26F2 | ⛲ | FOUNTAIN |
| 240 | U+1F3FA | 🏺 | AMPHORA |
| 241 | U+1F3EE | 🏮 | IZAKAYA LANTERN |
| 242 | U+1F488 | 💈 | BARBER POLE |
| 243 | U+1F4BA | 💺 | SEAT |
| 244 | U+1F697 | 🚗 | AUTOMOBILE |
| 245 | U+1F6B2 | 🚲 | BICYCLE |
| 246 | U+1F682 | 🚂 | STEAM LOCOMOTIVE |
| 247 | U+1F6A1 | 🚡 | AERIAL TRAMWAY |
| 248 | U+26F5 | ⛵ | SAILBOAT |
| 249 | U+1F6F6 | 🛶 | CANOE |
| 250 | U+2693 | ⚓ | ANCHOR |
| 251 | U+1F6EB | 🛫 | AIRPLANE DEPARTURE |
| 252 | U+1F681 | 🚁 | HELICOPTER |
| 253 | U+1F680 | 🚀 | ROCKET |
| 254 | U+1F6F8 | 🛸 | FLYING SAUCER |
| 255 | U+1F6F0 | 🛰 | SATELLITE |

## 6. テストベクタ

| 公開鍵（16 進） | ダイジェストの先頭 4 バイト | コードポイント | 文字列（4 章） |
|---|---|---|---|
| `79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798` | `13 2f 39 a9`（19, 47, 57, 169） | U+1F43C U+1F421 U+1F33F U+1F52E | 🐼️🐡️🌿️🔮️ |
| `3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d` | `10 93 b2 85`（16, 147, 178, 133） | U+1F981 U+1F3A0 U+1F94C U+1F36F | 🦁️🎠️🥌️🍯️ |
| `2b9404067c270c5fccd0c249778bd27c432672acbd0a41f773868b4e6a01722c` | `25 83 a1 77`（37, 131, 161, 119） | U+1F422 U+1F36B U+1F3AE U+1F363 | 🐢️🍫️🎮️🍣️ |

## 7. 表の選び方（参考）

第 1 版の表は変えないので、この章は表がどう選ばれたかの説明にとどまる。

- Unicode 11 までの絵文字で、1 コードポイントだけのもの（ZWJ の組み合わせ・肌の色・国旗・キーキャップを含まない）。
- 人の顔・手・人物を含まない。形の違いがはっきりした動物・植物・食べ物・道具・乗り物・天気・記号などから選ぶ。
- 状態・警告・セキュリティの合図に見えるもの（鍵・錠・チェック・禁止・矢印・星など）、宗教・政治・星座・お金・暴力・酒・薬にかかわるもの、苦手な人の多い虫や微生物を含まない。
- 同じものの変種や、小さく描くと似る組は片方だけにする。
