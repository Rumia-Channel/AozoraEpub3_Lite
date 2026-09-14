# AozoraEpub3_Lite 引継ぎメモ

更新日: 2026-09-12
作業ディレクトリ: `C:/Users/rumia/Desktop/APP/Rust/AozoraEpub3_Lite`
作業ブランチ: `develop`

## 目的

Java 版 AozoraEpub3 の本文変換・EPUB生成機能を、GUI とネットワーク通信を持たない Rust 軽量版として再構築する。成果物は次の2形態とする。

- `aozora_epub3_lite` Rust ライブラリ
- `AozoraEpub3_Lite` 単体 CLI

ライセンスは AozoraEpub3 に合わせて GPL v3。

## 軽量版の対象範囲

### 対象

- ローカルの TXT / ZIP / TXTZ / CBZ からの EPUB 3 生成
- CLI による入力、出力先、タイトル種別、表紙、文字コード、書字方向、端末指定の操作
- INI、プリセット、注記資産、外字資産などの外部ファイルによる設定
- 青空文庫注記の本文変換、メタデータ抽出、ローカル画像・表紙処理
- EPUB の構造、目次、画像、外字フォント、タイトルページの生成
- Java 版のローカル変換結果との互換性向上

### 明確な対象外

次の機能は軽量版には不要であり、未実装でも欠陥とは扱わない。今後の優先実装にも含めない。

- GUI、Swing画面、GUI上の確認・編集・プロファイル管理
- Web小説取得、サイト別HTML抽出、Web変換キャッシュ
- HTTP / HTTPS を含むネットワーク通信
- HTTP表紙、外部画像、外部文書などのネットワーク資源取得
- RAR入力

Java版にこれらの機能が存在していても、軽量版の互換対象はローカル入力からEPUBを生成する経路に限定する。

## 完了条件

軽量版の完了は、Java版の全機能を移植したことではなく、次を満たすこととする。

1. CLI の全オプションがヘルプに掲載され、正常系・異常系・境界値をテストできる。
2. 外部 INI / プリセット / 注記資産 / 外字資産を指定して、設定が変換結果に一貫して反映される。
3. TXT / ZIP / TXTZ / CBZ のローカル入力を、エラーなく再現可能な EPUB に変換できる。
4. EPUB の ZIP 整合性、manifest、spine、nav、NCX、画像、外字フォントを検証できる。
5. 対象範囲内の Java 版フィクスチャについて、差分を管理しながら互換性を継続的に改善できる。
6. 対象外の Web小説取得、ネットワーク通信、RAR、GUIを実装しないまま軽量性を維持する。

完了条件を満たすために必要な差分修正・回帰テスト・設定テストは実装対象とする。対象外機能を理由に完了を保留しない。

## 現在の状態

実装は初期基盤を超え、TXT / ZIP / TXTZ / CBZ から EPUB 3 を生成できる状態。主な処理は以下。

- 青空文庫注記の本文変換
- ルビ、縦中横、外字、IVS、文字置換、注記付き装飾
- 改ページ、改丁、改段、見出し、字下げなどのブロック処理
- 画像注記と raw `<img>` の収集・EPUB 内配置
- 表紙、タイトル・著者・出版社メタデータ
- EPUB 3 の manifest / spine / nav / title page
- Java 版に近い CLI オプション

公開 API は `src/lib.rs` から次を再 export している。

- `AozoraConfig`, `IniSettings`
- `EpubBook`, `EpubAsset`, `EpubMetadata`, `EpubSection`
- `Input`, `TextEntry`, `decode_text`
- `BookMeta`, `TitleType`, `detect_meta`
- `plain_text_to_xhtml`, `aozora_text_to_xhtml_sections_with_config`
- `image_references`, `escape_html`

CLI のヘルプは次で確認できる。

```text
cargo run -- --help
```

## 重要な直近修正

### 2026-09-12: `docs/xhtml-diff-investigation.md` の4パターンを解消、長編2本が完全一致

`origin/investigate/xhtml-diff` の調査メモにあった4パターンと、実データ比較で追加判明した
差分を修正した。**縦書き長編87ファイル・49ファイル、横書き49ファイルが Java 出力と
byte 一致**（`dcterms:modified` は変換時刻のため実行ごとに異なる）。

修正内容:

- **パターンD (text.css)**: `@page` margin を `0 0 0 0`、`html.vrtl`/`html.hltr` も `0 0 0 0` に。
  外字フォント挿入時の空行位置も Java と同じ1行に。
- **パターンB (replace.txt)**: 実行ファイル隣に `replace.txt` が無い場合は置換を適用しない
  （Java `jarPath/replace.txt` と同じ条件）。`AozoraConfig::load_from_dirs` は
  manifest フォールバック時のみ `character_replacements` をクリアする。
- **パターンC (merge_same_line_block_pieces)**: ブロック注記を含む行は分割せず1行で出力する
  （`contains_block_note` + `bare` フラグ）。Java `printLineBuffer` の `noBr`
  （`chuki_tag.txt` 4列目=1）と `isBlockTag` 相当。行内のブロック開閉・横組み状態は
  `render_lines` の no_br 分岐で追跡する。
- **パターンA (空行)**: `noBr` 行では空行カウントを行わない Java 挙動に追従（上記修正に含む）。
- **SpaceHyphenation**: `SpaceHyphenation` INI を実装。20文字目以降の「文字に挟まれた」
  全角スペースのみ `<span class="fullsp">`/U+2000×2 に変換（`java_pos` で Java の
  phase-1 文字位置を追跡）。
- **連続ルビ**: `｜A《x》｜B《y》` を1つの `<ruby>` にまとめる。`｜` 分岐に `continue` が
  無く次文字を literal 化していたバグと、`has_following_implicit_ruby` の基底判定を修正。
- **対応する `《》` が無い `｜`**: マーカーだけ消費する（`〝｜♡〟` → `〝♡〟`）。
- **正立 (`upr`)**: Java は `converter.vertical && !inYoko` の時だけ付与する。Lite は無条件に
  付けていたため、`-hor` でも `upr` が付いていた。`allow_upright && config.vertical` に修正し、
  `-hor` を `config.vertical` に反映させる。
- **水平表題ページ (`TitlePage=2`)**: `title_horizontal.vm` 相当のレイアウトを実装。
  表題行は `converter.vertical=false` で変換する（`upr` が付かない）。
- **目次/NCX の表題項目**: 生の表題文字列（`bookInfo.title`）を使う。変換済みマークアップを使うと
  `upr` などが混入する。
- **manifest**: 外字フォントの item を xhtml セクション群の後・ncx の前へ（Java と同じ順）。
- **`dcterms:modified`**: 変換時刻を ISO 8601 UTC で出力（自前の civil-from-days 計算）。
- **画像取得失敗時の空 span 除去**: class 付きの空 span（二分アキ等）は残し、画像ラッパー
  （class なし）だけを除去する。

回帰テスト:

```text
text::tests::keeps_block_note_lines_out_of_paragraph_wrappers
text::tests::merges_consecutive_explicit_ruby_groups
text::tests::applies_space_hyphenation_for_late_full_width_spaces
tests::keeps_class_carrying_empty_spans_when_images_are_missing
```

### 2026-08-16: 縦中横の後置注記

後置注記による縦中横がルビの途中に入り、不正な XHTML を生成していた問題を修正した。

入力例:

```text
Ｂ｜29［＃「29」は縦中横］《二十九》
```

期待する出力:

```html
Ｂ<span class="tcy"><ruby>29<rt>二十九</rt></ruby></span>
```

回帰テスト:

```text
text::tests::keeps_suffix_tcy_notes_outside_following_ruby
```

### 2026-08-16: パリティ差分を 64行 → 8行 に縮小（19/21 完全一致）

Java 参照との XHTML 差分をカテゴリ単位で解析し、以下を修正した（11カテゴリ・45行分）。

- 単ページ画像のレンダリング: `split_image_page_sections` で `<p>` を除去し `<span><img class="fit">` を `<html class="hltr">` + `<body class="p-image">` セクションに（画像回り込み0002解消）
- 画像幅の拡張子フォールバック: `CollectedAsset` に参照単位の `available` を導入し、元の参照名で解決できない画像は ratio=0 → fit。装飾を書き換え前に実行（画像回り込み0010解消）
- 画像 alt の入力間共有: 入力ごとに `image_alt_map` をクリア（出版社0002解消）
- 横組み内の `“→〝` 変換抑止: `convert_inline_with_yoko` + `in_yoko` 状態を `convert_inline` / `normalize_vertical_character` に導入（横書き横組み・0043解消）
- ブロック内インライン注記の行頭空白保持（pb1 btm 解消）
- 窓中見出し2個目の抑止: 行単位 `in_mado` + 行頭プレフィクス判定（0028・窓見出し解消）
- 画像 alt 内の正立タグ: `apply_alt_upright` を `decorate_image_tags` に適用（0030解消）
- 「ここから中見出し」+同Line内容+閉注記の1行化: `merge_same_line_block_pieces`（0049解消）
- 章名のタグ保持: `parse_raw_anchor` を `<a `（属性必須）に限定、生タグ素通しを tcy タグのみに（目次0002解消）
- SVG ページ末尾改行の除去（test_png解消）
- タイトル後の空行保持: `remove_metadata_lines` の先頭空行削除を除去（横書き横組み・目次0001解消）

## 検証済みコマンド

以下は 2026-08-16 時点で成功済み。

```text
cargo fmt --all -- --check
cargo test --all
cargo test --test epub_structure
cargo test --test cli_config
cargo clippy --all-targets --all-features -- -D warnings
cargo build --all-targets --all-features
ccc index
```

結果:

- `cargo test --all`: 163 passed、1 ignored
- `cargo test --test epub_structure`: 11 passed
- `cargo test --test cli_config`: 3 passed
- clippy: 警告なし
- build: 成功
- `ccc index`: 38 files / 1628 chunks / error 0

代表入力による CLI 変換も確認済み。

```text
cargo run --quiet -- -d target/progress-check \
  sample/AozoraEpub3/test_data/test_title.txt \
  sample/AozoraEpub3/test_data/test_chuki.txt \
  sample/AozoraEpub3/test_data/test_png.zip
```

3件のEPUBが生成され、`unzip -t` によるZIP整合性確認に成功した。さらに現行実装で
`test_data` 内の21件のローカルフィクスチャをCLI変換し、すべてEPUBCheck: R生成20/21クリーン（注記のみJ参照と同一のplayOrder/#linkエラー3件）

## 既知の残存事項

### 1. Java 版との一致状況（2026-09-12: 長編2本が完全一致）

`sample/AozoraEpub3/test_data` の21件について、2026-08-16 時点で 19/21 が XHTML 完全一致。
2026-09-12 に実データ（Web小説長編）で追加の差分を解消し、**以下が byte 一致**した。

- `n7783eg いろはにサキュバス～サキュバスの倒し方教えます～`（縦書き・87ファイル）
- `n2878hd いろはにサキュバスⅡ～今度こそ、サキュバスの倒し方教えます～`（縦書き・49ファイル、横書き49ファイル）

比較方法: `java -cp AozoraEpub3.jar AozoraEpub3 -i AozoraEpub3.ini -enc UTF-8 -ext .epub -of` と
Lite CLI を同じ INI / config-dir で実行し、EPUB 内の xhtml / css / opf / ncx / nav を
CR 除去して byte 比較（`dcterms:modified` は変換時刻のため比較から除外）。

残差分（21件フィクスチャの旧記録）:
- **出版社0001（3行）**: タイトル前の表紙画像（`［＃（img/表紙.jpg）］` + 直後改ページ）。Java は `isImageSectionLine`（画像単独行+直後改ページ→pなし）とタイトル前バッファ処理（preTitleBuf）で `<p><br/></p>` + pなし `<span>` を出力し、Rust は `<p><span>` で出力する。解消には Java のタイトル前バッファ処理の再現が必要（2026-09-12 時点で未検証・未解消）。
- **目次0005（5行）**: 章名内の `※※［＃米印］※［＃始め二重山括弧］`（《の直前の※が偶数）で Java が《をルビ開始と誤認し、未閉じルビの行末破棄で章名と `</h2>` を欠落させるデータ欠落バグ。**再現しない方針**。Java 版へ issue 報告済み（`kyukyunyorituryo/AozoraEpub3#34`。公式バイナリ 1.1.1b33Q でも再現確認済み）。Java 側の修正が反映されれば自動的に解消する。

EPUBCheck（21件）: 19件 0 エラー。注記（4件）・外字画像（1件）は J 参照と同一のエラー（alt 内 `<span class="upr">` と `&times;` 未宣言）。

### 2. CLI と外部設定の残存事項

CLI の主要オプションと外部設定の基本経路は実装・テスト済みである。残る項目は次に限定する。

- INI の全画像・改ページキーについて、変換結果まで含む代表ケースの固定。
- ローカル入力に対する Java 版互換差分を、再現可能なフィクスチャとして管理する。

### 3. 検証環境

- 差分計測: `target/parity-check/{java,rust,java2,rust2,java-hor,rust-hor}` に Java / Lite の出力を置き、EPUB 内エントリを CR 除去して byte 比較（Python + zipfile/difflib）。`dcterms:modified` は変換時刻のため比較前に除去する。
- `tests/epub_parity.rs` は比較用のJava/Rust生成ディレクトリが必要なため通常は ignored（`AOZORA_JAVA_DIR` / `AOZORA_RUST_DIR` を設定して `cargo test -- --ignored`）。
- `target/parity-rust-head` に現行実装で生成した21件を EPUBCheck で検証し、19件 0 エラー。注記（4件）・外字画像（1件）は J 参照と同一のエラー（alt 内 `<span class="upr">`、`&times;` 未宣言、playOrder 重複、未定義フラグメント）。
- Java 参照の再生成・デバッグには、`target/java-build`（sample ソースの javac ビルド）または公式バイナリ（`C:/Users/rumia/Documents/AozoraEpub3/AozoraEpub3.jar`、CLI は `java -cp AozoraEpub3.jar AozoraEpub3` で起動）が使える。
- テスト: `cargo test --all` 170 passed / 1 ignored、`cargo clippy --all-targets --all-features -D warnings` 通過。

### 4. 対象外

Web小説取得、HTTP / HTTPS リソース取得、RAR入力、GUIは、未実装事項ではなく軽量版の明確な対象外である。これらを理由に完了判定を遅らせない。

## 軽量版機能監査（2026-08-13）

### 対応済み

- TXT / ZIP / TXTZ / CBZ のローカル入力、UTF-8 / Shift_JIS 系の自動判定、タイトル・著者・出版社の推定。
- 青空文庫注記の資産ロード、通常の注記タグ、改ページ・改丁・左右中央・ページ左寄せ、見出し、字下げ、行頭強制字下げ。
- ルビ、設定由来の後置注記、自動縦中横、外字コード・IVS・代替文字、拡張ラテン、文字置換。
- 注記画像・raw `<img>`、ローカル画像解決、EPUB内配置、ローカル表紙、CBZの画像専用EPUB。
- EPUB 3 の container / OPF / spine / nav / NCX / title page / cover。`mimetype` は非圧縮で先頭に配置。
- 画像の寸法解析、リサイズ、回転、ガンマ、余白処理、浮動・単ページ判定、SVG固定レイアウトページ化。
- 本文の空行除去 / 最大空行数、本文バイト数・空行数・章単位の強制改ページ、`底本：` の奥付分離、Kobo栞用 paragraph ID。
- 外字フォントの検出、glyph span出力、EPUB内フォント格納、動的 `font.css` 生成。

### 部分対応

- 注記資産の固定形式（1〜30字下げ、折り返し、字詰め、地付き等）は `AozoraConfig::load_tag_text` で変換できる。一方、資産内で `TODO Pattern` とされる正規表現形式の複合字下げはJava版と同じ汎用演算ではない。
- 注記フラグは `P` / `M` / `L` と `1` の動作を実装している。`K`（訓点）と `2` / `3`（ルビ排他）はJava版の専用状態管理とは一致しない。
- 割り注は `<span class="wrc">` へ変換し、改行も処理するが、Java版の自動改行・禁則計算との完全一致は未確認。後置注記も資産に定義された装飾規則が中心で、「〜のルビ」や「注記付き」の専用変換は未実装。
- 画像処理の主要機能は実装済みだが、Java `ImageUtils` との画素単位・エンコード単位の完全一致は未達。
- 目次はセクション先頭の h1〜h3 を階層化し、`底本：` の奥付ページを除外する。Java版の自動章名抽出・副題 / 原題 / シリーズ等は未実装。

### 優先して仕上げる項目

- 外字資産を含む `--config-dir` の CLI エンドツーエンドテスト。
- ローカル入力に限定した Java 版との XHTML / セクション差分の縮小。
- EPUB構造、ローカル画像、外字フォント、表紙、目次の回帰テスト拡充。

この監査では、軽量版の互換対象をローカル入力からEPUBを生成する経路に限定した。Web小説取得、HTTP / HTTPS、RAR、GUIは実装対象外として評価から除外する。


## 2026-09-14: Java 版との全面監査と修正

Java 版 (kyukyunyorituryo/AozoraEpub3) の 8 サブシステムを並列監査し、ローカル
変換経路の欠落・不一致を洗い出して修正した。**インストール済みの
`AozoraEpub3.jar` は参照にならない**点に注意。jar (2026-08-07 ビルド) は
リポジトリ `src/` (2026-09-11) より古く、`ImageInfoReader.correctExt` の
null ガード (1453e12) が未反映で `test_chapter.txt` の変換が
`NullPointerException` で落ちる。参照実装は `src/` を javac でビルドして使う。

```text
javac -encoding UTF-8 -proc:none -cp "AozoraEpub3.jar" -d <classes> <src/**/*.java>
java -cp "<classes>;AozoraEpub3.jar" AozoraEpub3 -i <ini> -ext .epub -d <out> <input>
```

`--config-dir` は注記資産の場所を指定するだけで、変換フラグを変えてはならない。
（以前は Java CLI パリティの上書きが `--config-dir` の有無で分岐していた。）

### 修正済み

- 表紙ページを `item/xhtml/cover.xhtml` に出力（manifest の href と一致せず
  参照切れだった）。`cover.vm` を再現（fixed-layout-jp.css / viewport /
  `epub:type="cover"` / SVG 画像）
- `-i`/`--preset` 指定時に `AutoYoko` / `DakutenType` / `IvsBMP` / `IvsSSP` が
  捨てられる問題。`AutoYokoEQ3` の既定を true に（Java は INI キーを持たず
  常に有効）。`replace.txt` を `replace_sample.txt` に改名して Java と同じ
  「未使用」状態に
- noBr 行の複合字下げ開きタグ欠落、字下げ省略 (前ブロックを同じ行で閉じる)、
  キャプション終わりでの画像ラッパー閉じ
- `※` エスケープの判定順（Java は外字変換が先）と連鎖（`ch[idx]='　'` 相当）
- package.vm の空行と表紙 itemref のインデント、toc.ncx の本文フォールバック、
  表紙 item の属性順
- スタイル設定 8 キー (`PageMargin` / `BodyMargin` / `*Unit` / `LineHeight` /
  `FontSize` / `BoldUseGothic` / `gothicUseBold`) を text.css に反映
- `CoverPage` / `CoverPageToc` / `TocVertical` / `NoIllust` を実装
- `JisConverter` の面区点テーブル全表を `src/jis.rs` に移植
  (`tools/gen_jis.py` で生成)。辞書に無い面区点コード付き外字注記の
  不一致 60/75 → 0/75
- 画像系: `scan_top` の代入漏れ (上余白が切り取られない)、
  `SinglePageWidth`/`SinglePageSizeW`/`SinglePageSizeH` と
  `AutoMarginWhiteLevel` の既定値、画像幅の `Double.toString` 表記、
  `.jpeg` → `.jpg` 正規化、画像のみ ZIP の `FileNameComparator` 並び替え、
  `RotateImage` の適用条件 (Java は単ページ画像とアーカイブ入力の本文画像
  のみ)、画像注記の `（`/`）` 解析 (最後の `（` 〜 最初の `、`/`）`)、
  画像指定外字 (`※［＃…（file）］` → 外字画像)

### 意図的に再現していない Java 側の挙動

- `［＃米印］` 等で内部エスケープマーカーが `※` から `\u0001` に変わった
  (444d66d) 影響で、`＜＜` / `＞＞` がルビとして解釈され行が欠落する
  (`test_chuki.txt` 0049)。Lite は文書化された意図 (`＜＜` → リテラルの `《`)
  に従う
- `dcterms:modified`: Java はローカル時刻に `Z` を付ける。Lite は UTC
- 章名中の `※` の並びで行が欠落する件 (kyukyunyorituryo/AozoraEpub3#34)

### 既知の残差

- `test_title.txt` 0001: 表題前の表紙画像（`preTitleBuf` 相当）で
  Java は `<p><br/></p>` + p なし `<span>`、Lite は `<p><span>`
- `test_ruby.txt`: タイトル抽出の `※` 圧縮が Java と異なり出力ファイル名が
  `ルビ※※※※《》` vs `ルビ※※《》`
- `test_png.zip`: 画像のみ EPUB の `standard.opf`
- `NoIllust=1` のセクション数 (Java は `isImageSectionLine` も無効化するため
  単ページ画像由来の改ページが消える)
- 画像の連番 (`NNNN.ext`) は Java と一致しない場合がある
- `IMAGE_PAGE_NOFIT` (FitImage=0 で画面内に収まる単ページ画像) の扱いと
  `ImageFitW/H` / `ImageHeight` 相当の単ページ画像 CSS
- 画像バイト列の一致: Java は色モデル (2 値 / インデックス / グレー) を保持し
  WebP を Lossy で書くため、リサイズが入る画像は byte 一致しない
- `ChukiRuby` (`［＃「○」に「△」のルビ］` / 注記付き → ルビ / 小書き)
- 章名の自動抽出キー (`ChapterExclude` / `ChapterUseNextLine` / `ChapterName` /
  `ChapterNum*` / `ChapterPattern` / `ChapterNameLength`) は未実装
- タイトル・章名の `※` 圧縮。`test_ruby.txt` で Java は `ルビ※※※※《》`、
  Lite は `ルビ※※《》` となる。Java は 444d66d で内部エスケープを `※` から
  `\u0001` に変えたため、`CharUtils.getChapterName` は `\u0001` の除去だけを
  行い、`※` は通常文字として残る (米印外字は `※` を 2 文字出力する)。
  Lite の `metadata.rs` は旧挙動 (`※` + 特殊文字のペア除去) を移植しており、
  単純に外すと逆に 1 文字多い `ルビ※※※※※《※》` になる。切り分けには
  `convert_gaiji_notes` と `remove_ruby` / `unescape_marks` の適用順の整理が
  必要 (未着手)。

フィクスチャ 21 件のうち 18 件が byte 一致。差分は `test_chuki.txt` 0049
(Java 側のエスケープ退行)、`test_title.txt` 0001 (表題前バッファ)、
`test_png.zip` (画像のみ EPUB の OPF) の 3 件。

2026-09-14 の追加修正 (続き):

- 米印外字の `※` を 2 文字出力 (Java の内部マーカーは 《》｜＃ では `\u0001`
  だが ※ では literal な ※ のため)。タイトル・目次ラベルが一致し
  `test_ruby.txt` が完全一致に
- ChukiRuby 周辺: `※［＃…のルビ］` を前方参照注記として扱わない
  (Java は外字変換を先に通す)、対象が行頭に無い場合の空基底ルビ、
  その際のインデックス外 panic を修正
- 章の自動抽出 (`ChapterName` / `ChapterNumOnly` / `ChapterNumTitle` /
  `ChapterNumParen` / `ChapterNumParenTitle` / `ChapterUseNextLine` /
  `ChapterExclude`) を実装。抽出 4 種 + 次行連結 + 除外の 6 構成で
  nav / toc.ncx が Java と一致
- `tools/parity_check.py` を追加 (21 フィクスチャの差分をカーネル外で測る)
- 注記表の網羅検証 `tools/note_coverage.py` を追加。`chuki_tag.txt` /
  `chuki_tag_suf.txt` の全行を 1 行ずつ Java / Rust で変換して比較する。
  開始注記と終了注記は表の意味論で対にして 1 ケースにし (`X前`↔`X後`、
  `X開始`↔`X終了`、`ここからX`↔`ここまでX`/`ここで…終わり`、段番号が
  落ちる綴りは接尾辞の最長一致)、利用者が本文に書けない 33 行
  (画像タグ 19 行 = `printImageChuki` の `String.format` テンプレート、
  折り返し1/2/3 等の断片・属性 14 行) を除外する。結果は **589 ケース中
  5 件が差分、Java が出すタグを Rust が出していない注記は 0 件**
- 残る 5 件は次の 2 挙動のみ
  - `ページ左下` / `ページの左下`: Java はページ注記と章の先頭行が同一行の
    ときのみ `id="kobo.N.M"` を注入する (Lite は注入しない)。注記を単独行に
    置く実用法では byte 一致する
  - `地付き` / `字下げ省略` / `行内地付き`: Java は閉じタグを二重出力する
    (`<div class="btm">本文</div></div>` 等)。Java 側が非整合なため再現しない
- ページ下付き注記 (chuki_tag.txt 4列目=L) を flag=1 と同じタグ登録に含め、
  改ページ後の行に開閉タグを出力するようにした。`tools/realistic_cases.py`
  (現実的な用法 17 ケース) は 16 件が Java と一致。残り 1 件は `［＃地付き］`
  の二重 `</div>` で、上記と同じ意図的非再現
- `ページの左右中央` はタグ列が空の注記なのに `block_single_tags` の既定に
  入っていたため noBr 扱いになり `<p>` が落ちていた。既定から外して Java と
  一致 (`block_single_tags` の既定はタグを持つ注記のみにする)
- 複合字下げのクラス付与が Java の else-if と違っていた。`破線枠囲み` は
  `枠囲み` を含むため `dashed_border` と `border` の両方が付いていた
  (Java は `dashed_border` のみ)。罫囲み / 枠囲み の各組で排他にする
- 回帰テストは `中寄せ` を合成していて修正前でも通る空振りだった。既定 config の
  `ページの左右中央` を使う形に直し、旧既定に戻すと落ちることを確認した

### 注記表カバレッジの最終値 (2026-09-14)

`tools/note_coverage.py` は開始注記と終了注記を表の意味論で対にして 1 ケースにし、
利用者が本文に書けない 33 行 (画像タグ 19 行 = `printImageChuki` の
`String.format` テンプレート、折り返し1/2/3 等の断片・属性 14 行) を除外する。
`chuki_tag_suf.txt` の行は開始/終了注記名から `chuki_tag.txt` のタグ列を引いて
期待タグにする (93 行分を解決) ので、gap 判定は両表を覆う。

```text
(内部生成タグ/断片として除外: 33)
(対になる終了注記なし: 77)
notes: 589
differs: 5/589
=== Java が出すタグを Rust が出していない注記: 0 ===
```

残り 5 件は「`ページ左下`/`ページの左下` の `id="kobo.N.M"` 注入 (注記と章の
先頭行が同一行のときのみ)」2 件と「Java が閉じタグを二重出力する
`地付き`/`字下げ省略`/`行内地付き`」3 件で、いずれも再現しない方針。

除外した 33 行が未検証のまま残るわけではない。画像タグ 19 行は
`main.rs` の画像処理 (`decorate_image_tags`、float / 単ページ / 外字画像) と
`applies_java_float_image_classes` などのテスト、`test_image.txt` /
`test_gaiji_image.txt` / `test_png.zip` フィクスチャで検証している。断片・属性
14 行は複合字下げとして `realistic_cases.py` の 6 形態で検証している。
`柱` は `chuki_ivs.txt` の IVS 外字エントリでタグ注記ではない (外字経路で検証)。
`tools/realistic_cases.py` は 22 ケース中 21 件が Java と一致 (残りは上記の
二重 `</div>`)。複合字下げのクラスは全形で一致する。

```text
［＃ここから３字下げ、罫囲みと中央揃え］ → mt3 border center
［＃ここから２字下げ、破線罫囲み］       → mt2 dashed_border
［＃ここから２字下げ、破線枠囲み］       → mt2 dashed_border
［＃ここから２字下げ、横書き］           → mt2 yoko
［＃ここから３字下げ、５字詰め］         → pt3 jzm5
［＃ここから３字下げ、折り返して２字下げ］ → pt2 idt1
```

### 2026-09-14: Narou カスタム注記は narou.rs が所有 (Lite 側は上流表のまま)

Narou.rb / Narou Bridge のカスタム注記 (`ここから柱` / 前書き / 後書き /
パラメーター / 一字〜三字下げ / 二分アキ / 濁点 / zws / `ｌｉｎｋ＿ｓ` 系) は
**narou.rs 側の資産**であり、Lite の `assets/aozora/chuki_tag.txt` には
取り込まない (上流 874 行のまま)。当初は配布物の表 (904 行) を正として
30 行を取り込んだが、次の理由で撤回した。

- 配布物の 30 行は narou.rs の `init` が書き込んだもの。`init.rs:286-305` が
  `preset/custom_chuki_tag.txt` を読み、インストール先 `chuki_tag.txt` の
  `### Narou.rb embedded custom chuki ###` マーカー間を置換 (無ければ追記) する。
  上流リポジトリの表には痕跡が無い (`git log -S "ここから柱"` が空)
- narou.rs は Lite 資産のスナップショット `assets/aozora_lite/*.txt` (874 行) を
  `include_str!` で持ち、`AozoraConfig::default()` に `load_tag_text` などで
  自前で重ねる (`src/epub_lite.rs:28-60` の `embedded_config()`)。注入配線は
  narou.rs 側にあり、Lite に焼き込んでもスナップショットを更新するまで届かない
- つまり焼き込みは二重管理。Lite 側の責務は「注記タグを外部から注入できる口」で、
  それは実装済み (下記)

検証 (上流表に戻した状態):

```text
tools/note_coverage.py  : 589 ケース中 5 件差分、タグ欠落 0 件
tools/realistic_cases.py: [aozora] 21/22 / [narou] 9/9
tools/parity_check.py   : 18/21 (既知 3 件のみ)
cargo test --release    : 全バイナリ green
```

`realistic_cases.py` の Narou グループは実経路で比較する: Java は
「上流表 + narou プリセット」の作業ディレクトリ、Rust は
`--config-dir` に narou.rs の `preset/custom_chuki_tag.txt` を
`custom_chuki_tag.txt` として渡す。プリセットの場所は環境変数 `NAROU_PRESET` で
差し替え可 (既定 `../narou.rs/preset/custom_chuki_tag.txt`)。無い場合はスキップ。

#### 注記タグ / CSS の外部注入 (配線状況)

- 注記タグ: ライブラリは `AozoraConfig::load_tag_text` ほかの公開ローダを持ち、
  `load_from_dirs` は `chuki_tag.txt` に加えて `custom_chuki_tag.txt` を上書き
  マージする (テスト `loads_standard_and_overlay_directories_in_order`)。CLI は
  `--config-dir <dir>` でこれを配線済み。実測: `custom_chuki_tag.txt` に
  `テスト強調<TAB><span class="test-em">` だけ置いたディレクトリを
  `--config-dir` で渡すと `<span class="test-em">強調</span>` が出力される
- 組み込み表は `include_str!` でコンパイル埋め込みのため、`--config-dir` を
  渡しても既定の注記は失われない。実測: `--config-dir` (追加 1 ファイルのみ) でも
  `［＃大見出し］` / `［＃ここから太字］` / `［＃ここから３字下げ、罫囲みと中央揃え］`
  がすべて期待どおり出力される
- `--config-dir` が置き換えるのは**ディスク上の資産解決** (`main.rs:70-73`)。
  失われるのは `<dir>/gaiji/*.ttf` のみ (加算化は未対応)
- CSS: ライブラリは `EpubBook::with_assets([EpubAsset::new("style/x.css",
  "text/css", bytes)])` で追加できる (manifest にも入る)。CLI に CSS を足す口は
  無い (既定で有効にする必要が出たときのみ検討)
- 配布物の `template/OPS/css_custom/vertical_font.css` には `/* 柱（もどき） */`
  として `.running_head` / `.half_em_space` / `.introduction` / `.postscript` /
  `.custom_parameter_block` の定義があるが、**変換では使われない** (Java ソースに
  `css_custom` 参照なし、生成 EPUB に `OPS/` も `css_custom/` も含まれない)。
  README_Changes 1.1.0b8 の名残で、リーダー向けの手動カスタム用サンプル
- `chuki_utf.txt` は Rust 資産だけ 1 行修正済み (`U+003AC` → `U+01F71`、
  JIS X 0213 1-11-39 の文字。6de28a6)。意図的に残す

#### 未対応: テンプレートの `_custom` 上書き

Java の `Epub3Writer.writeFile` (Epub3Writer.java:371-383) は、EPUB に格納する
テンプレートファイルごとに `template/<dir>_custom/<同名ファイル>` があれば
そちらを優先する (2012 年の README_Changes 1.1.0b8 の項目)。配布物の
`template/OPS/css_custom/vertical_font.css` と `template/item/css_custom/*.css`
はこの仕組み用のファイル。

Lite はテンプレートを `include_str!` でコンパイル埋め込みしているため、
この上書き機構を持たない。現状で出力差は出ない (配布物の `_custom` のファイル名が
実際に格納されるテンプレート名と一致せず、生成 EPUB に `css_custom` 由来の
ファイルが入らないことを実測)。ユーザーが格納済みテンプレートと同じ名前で
`_custom` を置いた場合のみ Java 側だけが差し替えるため、既知の残差として扱う。

## 作業ツリーとコミット状態## 作業ツリーとコミット状態
引き継ぎ後に完了した論理単位は、以下のコミットとして `develop` へ commit / push 済み。

- `bc3a29d`: Aozora 変換データ資産
- `0ddcc38`: EPUB テンプレート資産
- `d5746c0`: AozoraConfig の設定拡張
- `384e5a6`: 入力・メタデータ層
- `1fb68e9`: 本文変換・inline 注記処理
- `3886663`: EPUB レンダリングとナビゲーション
- `a366129`: Java 互換 CLI 統合
- `a0e0516`: 外字画像注記のローカル画像化
- `ed1d21b`: 画像寸法・回転・浮動・単ページ・SVG 処理
- `5bb092f`: 強制字下げと強制改ページ設定
- `d437e80`: Kobo 栞用 paragraph ID
- `6840d56`: `底本：` 奥付分離と目次除外
- `325c0a8`: コメントブロックの字下げ抑止
- `f740859`: Parity 残差分 64行→8行（19/21完全一致、画像・横組み・窓見出し・0049・目次章名・空行処理）

`develop` の HEAD は `origin/develop` と同期している (`master` は変更していない)。
2026-09-14 の修正は以下のコミット。

- `a7e0144`: 表紙ページと OPF/NCX を Java 版に一致させる
- `18894c8`: `-i`/`--preset` 指定時に INI の変換フラグが捨てられる問題を修正
- `1a72dbb`: noBr 行のブロック注記処理を Java 版に一致させる
- `e4db6e3`: `※` エスケープの判定順と連鎖を Java 版に一致させる
- `3c7c47c`: インライン字下げ注記でも字下げ省略を適用する
- `893784d`: スタイル設定 8 キーを text.css に反映する
- `db1bcd5`: `CoverPage` / `CoverPageToc` を実装する
- `6e773d0`: `NoIllust` を実装する
- `fb35008`: `TocVertical` を配線し目次ラベルに縦中横を適用する
- `1f7f5b7`: JIS X 0213 の面区点テーブルを Java から移植する
- `c2b2b83`: 余白除去・既定値・並び順・拡張子を Java 版に一致させる
- `660590b`: RotateImage を Java と同じ条件でのみ適用する
- `bfb82ad`: 画像注記のファイル名抽出を Java 版に一致させる
- `051ccb8`: 画像指定外字を外字画像として出力する

- `7433b7d`: Narou.rb / Narou Bridge のカスタム注記 26 行を取り込む
- `84b93c6`: README に Narou 注記の対応を追記する

再開時は既存差分を破棄せず、まず `git status --short --branch` で状態を確認すること。

```text
git status --short --branch
git log --oneline --decorate -8
```

作業ツリーが clean でない場合は、変更者と目的を確認してから続行すること。


## 主要ファイル

- `Cargo.toml`: GPL 設定、依存関係、lib 定義
- `src/lib.rs`: 公開 API
- `src/main.rs`: CLI と入力から EPUB までの統合処理
- `src/config.rs`: INI / 注記設定の読み込み
- `src/input.rs`: TXT / ZIP / TXTZ / CBZ と画像解決
- `src/metadata.rs`: タイトル・著者・出版社の推定
- `src/text.rs`: 本文・セクション変換
- `src/text_inline.rs`: ルビ、注記、縦中横、画像、リンクの inline 変換
- `src/epub.rs`: EPUB メタデータとアーカイブ生成
- `src/epub_render.rs`: XHTML、nav、title page のレンダリング
- `tests/epub_structure.rs`: EPUB 構造テスト
- `assets/aozora/`: AozoraEpub3 由来の注記・設定資産
- `sample/AozoraEpub3/`: Java 版の参照実装とテストデータ。`.gitignore` で除外
- `LICENSE.txt`, `gpl.txt`: GPL v3 と由来表示

## 再開時の推奨手順

1. `git status --short --branch` で未コミット変更を確認
2. `cargo test --all` を再実行
3. `test_chuki.txt` などローカルフィクスチャの Java / Rust XHTML 差分をセクション単位で比較する
4. 外字資産と画像・改ページ設定を使う CLI エンドツーエンドテストを追加する
5. 変更対象ごとに回帰テストを追加する
6. `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo build --all-targets --all-features` を実行する
7. 必要なら `ccc index` でコードインデックスを更新する
8. `develop` 上でレビュー・コミットする。`master` への merge / push はユーザーの明示依頼と十分な検証後のみ

Web小説取得、HTTP / HTTPS、RAR、GUIを調査・実装対象に戻してはならない。
