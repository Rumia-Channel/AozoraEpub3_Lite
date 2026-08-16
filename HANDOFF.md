# AozoraEpub3_Lite 引継ぎメモ

更新日: 2026-08-16
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

### 1. Java 版との完全一致は未達（2026-08-16 時点: 21件中19件がXHTML完全一致、残差分8行）

Java 参照（`target/java-run/out-all`、1ファイル1プロセス生成）と Rust 最新（`target/parity-rust-head`）を `\r\n→\n` 正規化して1行単位で比較。**19/21 完全一致**。

一致済み19件: IVS、ラテン文字、ルビ※※《》、傍点・傍線、割り注、外字⚽、外字画像、横書き横組み、正立☆∀、注記、濁点、画像回り込み、禁則処理、窓見出し、縦中横AAA、行内地付き、BOM付きUTF-8、電書協EPUBサンプル、test_png。

残差分（8行）:
- **出版社0001（3行）**: タイトル前の表紙画像（`［＃（img/表紙.jpg）］` + 直後改ページ）。Java は `isImageSectionLine`（画像単独行+直後改ページ→pなし）とタイトル前バッファ処理（preTitleBuf）で `<p><br/></p>` + pなし `<span>` を出力し、Rust は `<p><span>` で出力する。解消には Java のタイトル前バッファ処理の再現が必要。
- **目次0005（5行）**: 章名内の `※※［＃米印］※［＃始め二重山括弧］`（《の直前の※が偶数）で Java が《をルビ開始と誤認し、未閉じルビの行末破棄で章名と `</h2>` を欠落させるデータ欠落バグ。**再現しない方針**。Java 版へ issue 報告済み（`kyukyunyorituryo/AozoraEpub3#34`。公式バイナリ 1.1.1b33Q でも再現確認済み）。Java 側の修正が反映されれば自動的に解消する。

EPUBCheck（21件）: 19件 0 エラー。注記（4件）・外字画像（1件）は J 参照と同一のエラー（alt 内 `<span class="upr">` と `&times;` 未宣言）。

### 2. CLI と外部設定の残存事項

CLI の主要オプションと外部設定の基本経路は実装・テスト済みである。残る項目は次に限定する。

- INI の全画像・改ページキーについて、変換結果まで含む代表ケースの固定。
- ローカル入力に対する Java 版互換差分を、再現可能なフィクスチャとして管理する。

### 3. 検証環境

- 差分計測: `target/xhtml-diff-report*.txt`（旧）、最新は `target/parity-rust-head` と `target/java-run/out-all` の直接比較（Python + difflib）。
- `tests/epub_parity.rs` は比較用のJava/Rust生成ディレクトリが必要なため通常は ignored。新参照（out-all）に合わせて更新が必要。
- `target/parity-rust-head` に現行実装で生成した21件を EPUBCheck で検証し、19件 0 エラー。注記（4件）・外字画像（1件）は J 参照と同一のエラー（alt 内 `<span class="upr">`、`&times;` 未宣言、playOrder 重複、未定義フラグメント）。
- Java 参照の再生成・デバッグには、`target/java-build`（sample ソースの javac ビルド）または公式バイナリ（`C:/Users/rumia/Documents/AozoraEpub3/AozoraEpub3.jar`、CLI は `java -cp AozoraEpub3.jar AozoraEpub3` で起動）が使える。
- テスト: `cargo test --all` 163 passed / 1 ignored、`cargo clippy --all-targets --all-features -D warnings` 通過。

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


## 作業ツリーとコミット状態
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

`develop` の HEAD は `origin/develop` より先行しており、今回のパリティ修正と本 HANDOFF 更新は未 push。`master` は変更していない。

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
