# AozoraEpub3_Lite

## 謝辞

本ツールは、[急急如律令](https://github.com/kyukyunyorituryo) さんが開発された改造版 [AozoraEpub3](https://github.com/kyukyunyorituryo/AozoraEpub3)、および [hmdev](https://github.com/hmdev) さんが開発された [AozoraEpub3](https://github.com/hmdev/AozoraEpub3) の成果に基づいています。両開発者に深く感謝いたします。

青空文庫のテキストを EPUB 3 に変換する、Rust 製の軽量 CLI ツールです。

Java 版 [AozoraEpub3](https://github.com/kyukyunyorituryo/AozoraEpub3) のローカル変換機能を Rust で再実装しており、GUI やネットワーク機能を持たない代わりに、単一バイナリで動作します。

## 特徴

- **ローカル入力のみ**: TXT / ZIP / TXTZ / CBZ から EPUB 3 を生成
- **青空文庫注記に対応**: ルビ、縦中横、傍点、傍線、割り注、外字、字下げ、見出し、改ページ、画像注記など
- **縦書き・横書き**: 既定は縦書き、`--horizontal` で横書き
- **画像処理**: 回り込み、単ページの SVG 固定レイアウト化、表紙、リサイズ・回転
- **外字フォント埋め込み**: GlyphWiki の 1 文字フォントを EPUB 内に格納
- **Java 版との互換性**: ローカル変換の XHTML 出力を、21 件のテストフィクスチャ中 19 件で完全一致（2026-08-16 時点）
- **追加ランタイム不要**: Java は不要

## 動作環境とビルド

- Windows / macOS / Linux
- Rust 1.85 以降（edition 2024）

```sh
cargo build --release
```

生成物は `target/release/AozoraEpub3_Lite`（Windows では `AozoraEpub3_Lite.exe`）です。

## 使い方

```text
AozoraEpub3_Lite [options] input_files(txt, zip, txtz, cbz)
```

例:

```sh
# 作品.txt を縦書き EPUB に変換して out/ に出力
AozoraEpub3_Lite -d out 作品.txt

# 横書きで複数ファイルを一括変換
AozoraEpub3_Lite --horizontal -d out 作品1.txt 作品2.zip

# 設定ファイルと端末プリセットを指定
AozoraEpub3_Lite -i reader.ini --preset kobo_touch.ini -d out 作品.txt
```

出力ファイル名は、抽出したメタデータがある場合は `[著者] タイトル.epub` のような形で生成されます。`-of` を指定すると入力ファイル名（例: `作品.txt` → `作品.epub`）が使われます。`-d` を省略すると、入力ファイルと同じディレクトリに出力します。入力が複数ある場合も、それぞれ独立した EPUB が生成されます。

### オプション一覧

| オプション | 説明 |
|---|---|
| `-h`, `--help` | 使い方を表示 |
| `-i <file>`, `--ini <file>` | 外部 INI 設定を読み込む |
| `-t <index>` | タイトル種別。`0`: タイトル→著者（既定）/ `1`: 著者→タイトル / `2`: タイトル→著者（副題優先）/ `3`: タイトルのみ / `4`: タイトル＋著者のみ / `5`: なし |
| `-tf` | 入力ファイル名をタイトル・著者として使う |
| `-c <value>`, `--cover <value>` | 表紙。`0`: 最初の挿絵 / `1`: 入力ファイルと同名の画像 / 画像ファイル名。省略時は INI の `Cover` を参照 |
| `-ext <ext>`, `--ext <ext>` | 出力拡張子（既定 `.epub`、INI の `Ext` を参照） |
| `-of` | 出力ファイル名に入力ファイル名を使う |
| `-d <dir>`, `--dst <dir>` | 出力ディレクトリ（事前に存在している必要がある） |
| `-enc <name>`, `--encoding <name>` | 入力エンコーディング。`AUTO`（既定・自動判定）/ `MS932` / `UTF-8` |
| `-hor`, `--horizontal` | 横書き（既定は縦書き） |
| `--vertical` | 縦書きを明示指定 |
| `-device <name>`, `--device <name>` | 端末別の出力処理（例: `kindle`）。端末プリセット INI と併用 |
| `--language <lang>` | EPUB の言語（既定 `ja`） |
| `--creator <name>` | 著者名を上書き |
| `--config-dir <dir>` | 注記資産などの設定ディレクトリ（繰り返し指定可） |
| `--preset <file>` | 外部プリセット INI を読み込む |

## 入力形式

| 拡張子 | 内容 |
|---|---|
| `.txt` | 青空文庫形式のプレーンテキスト。Shift_JIS / UTF-8（BOM 付き含む）を自動判定 |
| `.zip`, `.txtz` | テキストと画像を同梱した ZIP |
| `.cbz` | 画像のみの ZIP。画像 1 枚を 1 ページとする画像専用 EPUB を生成 |

拡張子が未知のファイルは、ZIP マジックバイトで判定します。

## 出力

EPUB 3 準拠のファイルが生成されます。

```text
作品.epub
├── mimetype                  # 非圧縮・先頭配置
├── META-INF/container.xml
└── item/
    ├── standard.opf          # manifest / spine / metadata
    ├── nav.xhtml             # EPUB 3 ナビゲーション
    ├── toc.ncx               # EPUB 2 互換の目次
    ├── xhtml/0001.xhtml …    # セクション本文
    ├── image/                # 本文・表紙画像
    ├── style/                # CSS
    └── fonts/                # 外字フォント（利用時のみ）
```

## 設定

### INI / プリセット

`-i` で外部 INI、`--preset` でプリセット INI を読み込めます。`assets/aozora/` には既定の `reader.ini` と、端末別プリセット（`kindle_pw.ini`、`kobo_touch.ini`、`kobo_glo.ini` など）を同梱しています。主なキー:

- `Vertical`: 縦書き / 横書き
- `TitleType`: タイトル種別
- `PageBreak*`: 改ページ判定のしきい値
- `CoverW` / `CoverH`: 表紙サイズ
- `FitImage` / `ImageFloatPage` / `ImageFloatBlock`: 画像の配置方法
- `SvgImage`: 単ページ画像の SVG 固定レイアウト化

### 注記資産

`assets/aozora/` の `chuki_*.txt` に、字下げ・傍点・割り注などの注記定義を同梱しています。`--config-dir` で別ディレクトリの資産に置き換えられます。

### 外字資産（GlyphWiki 1 文字フォント）

`assets/aozora/gaiji/` に GlyphWiki 形式の 1 文字フォント（例: `u4e35.ttf`）を配置すると、対応する外字（`※［＃U+4E35］` など）がそのフォントで表示されます。本文の外字は `〓` に置換され、フォントは EPUB 内の `fonts/` に埋め込まれます。

フォントは <http://glyphwiki.org/wiki/> から入手できます。ファイル名規則とフォント調整手順は `assets/aozora/gaiji/README.txt` を参照してください。

## ライブラリとして使う

`aozora_epub3_lite` クレートとして、Rust プログラムから変換機能を呼び出せます。

```sh
cargo add aozora_epub3_lite --git https://github.com/Rumia-Channel/AozoraEpub3_Lite
```

または Cargo.toml に直接:

```toml
[dependencies]
aozora_epub3_lite = { git = "https://github.com/Rumia-Channel/AozoraEpub3_Lite" }
```

使用例（設定のロード → テキスト変換 → 画像収集 → EPUB 書き出し）:

```rust
use std::fs::File;
use std::path::Path;

use aozora_epub3_lite::{
    AozoraConfig, EpubAsset, EpubBook, EpubMetadata, Input,
    aozora_text_to_xhtml_sections_with_config, decode_input, image_references,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. 設定（INI・注記資産・外字資産）を読み込む
    let config = AozoraConfig::load_from_dirs(&[Path::new("assets/aozora")], None)?;

    // 2. 入力（TXT / ZIP / TXTZ / CBZ）を開く
    let input = Input::open("作品.txt")?;

    for entry in input.text_entries() {
        // 3. テキストをデコードして XHTML セクションへ変換
        let text = decode_input(&input.read_text(entry)?, None)?;
        let sections = aozora_text_to_xhtml_sections_with_config(&text, &config)?;

        // 4. 本文が参照する画像をアセット化
        let assets = image_references(&text)
            .iter()
            .filter_map(|reference| {
                input.resolve_image(entry, reference).map(|(path, bytes)| {
                    let media_type = if path.ends_with(".png") {
                        "image/png"
                    } else if path.ends_with(".gif") {
                        "image/gif"
                    } else {
                        "image/jpeg"
                    };
                    EpubAsset::new(format!("image/{path}"), media_type, bytes.clone())
                })
            })
            .collect::<Vec<_>>();

        // 5. EPUB を組み立てて書き出す
        let metadata = EpubMetadata::new("作品タイトル", "urn:uuid:example");
        let book = EpubBook::from_sections(metadata, sections)
            .with_vertical(true)
            .with_assets(assets);
        book.write_to(File::create("作品.epub")?)?;
    }
    Ok(())
}
```

主な公開 API:

- `aozora_text_to_xhtml_sections*`: 青空文庫テキストを XHTML セクションに変換
- `AozoraConfig`: INI・注記資産・外字資産の設定
- `EpubBook` / `EpubAsset`: EPUB の組み立て
- `Input` / `decode_text`: 入力の読込とエンコーディング判定
- `BookMeta` / `TitleType`: タイトル・著者などのメタデータ推定

## Java 版との関係

- 移植元: [AozoraEpub3](https://github.com/kyukyunyorituryo/AozoraEpub3)（GPL v3）
- ローカル変換の XHTML 出力を 21 件のフィクスチャで比較し、19 件が完全一致。残り 8 行は原因特定済みです（表紙画像の特殊ケース 3 行、Java 側のデータ欠落バグ 5 行）。
- Java 側のバグ（章名の `※` が偶数個続くと行ごと欠落する問題）は再現せず、正しい出力を生成します。詳細は [kyukyunyorituryo/AozoraEpub3#34](https://github.com/kyukyunyorituryo/AozoraEpub3/issues/34)。

### 対象外（軽量版の設計判断）

以下は実装しません。対象外であっても欠陥とは扱いません。

- GUI
- Web 小説の取得・変換（ネットワーク通信）
- RAR 入力

## ライセンス

GPL-3.0-only。AozoraEpub3 に合わせて GPL v3 で配布します。
