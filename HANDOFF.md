# AozoraEpub3_Lite 引継ぎメモ

更新日: 2026-08-12
作業ディレクトリ: `C:/Users/rumia/Desktop/APP/Rust/AozoraEpub3_Lite`
作業ブランチ: `develop`

## 目的

Java 版 AozoraEpub3 を Rust で再構築するプロジェクト。GUI は対象外とし、次の2形態を目標にしている。

- `aozora_epub3_lite` Rust ライブラリ
- `AozoraEpub3_Lite` 単体 CLI

ライセンスは AozoraEpub3 に合わせて GPL v3。

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

## 検証済みコマンド

以下は 2026-08-12 時点で成功済み。

```text
cargo fmt --all -- --check
cargo test --all
cargo clippy --all-targets --all-features -- -D warnings
cargo build --all-targets --all-features
cargo test --test epub_structure
ccc index
```

結果:

- `cargo test --all`: 83 passed
- `cargo test --test epub_structure`: 5 passed
- clippy: 警告なし
- build: 成功
- `ccc index`: 35 files / 1371 chunks / error 0

代表入力による CLI 変換も確認済み。

```text
cargo run --quiet -- -d target/final-cli-verified \
  sample/AozoraEpub3/test_data/test_title.txt \
  sample/AozoraEpub3/test_data/test_chuki.txt \
  sample/AozoraEpub3/test_data/test_png.zip
```

生成された EPUB は3件。ZIP 整合性エラーはなく、画像・XHTML・manifest を確認済み。

## 既知の残存事項

### 1. `test_chuki.txt` の EPUBCheck エラー

EPUBCheck では次のエラーが残る。

- `href="#link"` の参照先 ID がない
- `href="aaa"` の参照先リソースがない

これは入力フィクスチャ内の raw `<a>` タグが原因。Java 版も同様のリンクを出力するため、現時点では互換性を優先して自動修正していない。

### 2. Java 版との完全一致は未達

`test_chuki.txt` の主要な縦中横ルビは一致したが、Java 版と Rust 版で以下の差が残る。

- 一部の空セクション数・XHTML ファイル数
- ブロック注記やレイアウト注記の細部
- Java 版の冗長な二重 `<span class="tcy">` と Rust 版の正規化差
- 画像ページ、タイトルページ、リンク処理の細部

完全互換を目指す場合は、まず Java / Rust の XHTML をセクション単位で比較し、差分を次の単位で分離すること。

1. セクション分割
2. 注記変換
3. 画像解決・資産配置
4. EPUB レンダリング

## 作業ツリーとコミット状態

引き継ぎ時点の未コミット実装は、次の論理単位に分割して `develop` へ commit / push 済み。

- `bc3a29d`: Aozora 変換データ資産
- `0ddcc38`: EPUB テンプレート資産
- `d5746c0`: AozoraConfig の設定拡張
- `384e5a6`: 入力・メタデータ層
- `1fb68e9`: 本文変換・inline 注記処理
- `3886663`: EPUB レンダリングとナビゲーション
- `a366129`: Java 互換 CLI 統合

`develop` は `origin/develop` と同期済み。`master` は変更していない。

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
3. `test_chuki.txt` の Java / Rust XHTML 差分をセクション単位で比較
4. 変更対象ごとに回帰テストを追加
5. `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo build --all-targets --all-features` を実行
6. 必要なら `ccc index` でコードインデックスを更新
7. `develop` 上でレビュー・コミットする。`master` への merge / push はユーザーの明示依頼と十分な検証後のみ
