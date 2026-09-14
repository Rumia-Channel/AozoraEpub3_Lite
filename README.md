# AozoraEpub3_Lite

## 謝辞

本ツールは、[hmdev](https://github.com/hmdev) さんが開発した AozoraEpub3 と、[急急如律令](https://github.com/kyukyunyorituryo) さんによる改造版 AozoraEpub3 の成果に基づいています。

両開発者、および青空文庫・GlyphWiki など関連プロジェクトの関係者に感謝します。

青空文庫形式のテキストを EPUB 3 に変換する、Rust 製のコマンドラインツールです。

Java 版 [AozoraEpub3](https://github.com/kyukyunyorituryo/AozoraEpub3) のうち、ローカルファイルの変換に必要な機能を Rust で再実装しています。GUI や Web 小説の取得機能は持たず、TXT / ZIP / TXTZ / CBZ から EPUB を生成する処理に絞っています。

## 主な特徴

- TXT / ZIP / TXTZ / CBZ から EPUB 3 を生成
- 青空文庫の主な注記に対応（ルビ、縦中横、傍点、傍線、割り注、字下げ、見出し、改ページ、画像注記など）
- Narou.rb / Narou Bridge のカスタム注記に対応（柱、前書き、後書き、パラメーター、二分アキ、濁点、zws、ｌｉｎｋ系）
- 縦書き・横書きに対応
- 表紙、画像の回り込み、リサイズ・回転、単ページ画像の SVG 固定レイアウト化に対応
- GlyphWiki の 1 文字フォントを EPUB に埋め込み可能
- Java ランタイム不要
- Windows / macOS / Linux の x64 / ARM64 向けリリースを用意

GUI、Web 小説の取得、RAR 入力は対象外です。

## ダウンロード

ビルド済みの実行ファイルは [Releases](https://github.com/Rumia-Channel/AozoraEpub3_Lite/releases) から入手できます。

配布 ZIP には実行ファイルだけでなく、変換に必要な注記定義ファイル、外字フォント用ディレクトリ、プリセット、EPUB テンプレートも含まれています。ZIP を展開し、中のファイル構成を保ったまま使用してください。

macOS / Linux で実行権限が付いていない場合は、次のように設定します。

```sh
chmod +x AozoraEpub3_Lite
```

ソースからビルドする場合は Rust 1.85 以降が必要です（edition 2024）。

```sh
cargo build --release
```

生成される実行ファイルは `target/release/AozoraEpub3_Lite`、Windows では `target/release/AozoraEpub3_Lite.exe` です。

## まず使う

```text
AozoraEpub3_Lite [options] input_files(txt, zip, txtz, cbz)
```

最小の例:

```sh
AozoraEpub3_Lite 作品.txt
```

出力先を指定する場合:

```sh
AozoraEpub3_Lite -d out 作品.txt
```

`out` ディレクトリは事前に作成しておく必要があります。

横書きにする場合:

```sh
AozoraEpub3_Lite --horizontal 作品.txt
```

Kobo 向けのプリセットを使う場合:

```sh
AozoraEpub3_Lite --preset presets/kobo_touch.ini 作品.txt
```

外部 INI を使う場合は `-i` でも指定できます。

```sh
AozoraEpub3_Lite -i presets/reader.ini 作品.txt
```

`-i` / `--ini` と `--preset` は同時には指定できません。

### オプションは入力ファイルより前に指定する

Java 版との互換性のため、オプションの解釈は最初の入力ファイルで終了します。その後に続く引数はすべて入力ファイルとして扱われます。

```sh
# 正しい
AozoraEpub3_Lite --horizontal -d out 作品.txt

# --horizontal はオプションではなく入力ファイルとして扱われる
AozoraEpub3_Lite 作品.txt --horizontal
```

## 入力形式

| 拡張子 | 内容 |
|---|---|
| `.txt` | 青空文庫形式のプレーンテキスト。Shift_JIS / MS932 / UTF-8 を自動判定 |
| `.zip`, `.txtz` | テキストと画像をまとめた ZIP。複数のテキストが入っている場合は、テキストごとに EPUB を生成 |
| `.cbz` | 画像のみの ZIP。画像 1 枚を 1 ページとする EPUB を生成 |

拡張子が不明な場合でも、ZIP のマジックバイトを持つファイルは ZIP として判定します。

## 出力ファイル名

本文からタイトル・著者を取得できた場合、通常は次のような名前で出力します。

```text
[著者] タイトル.epub
```

`-of` を指定すると入力ファイル名をそのまま使います。

```sh
AozoraEpub3_Lite -of 作品.txt
# -> 作品.epub
```

`-d` を省略した場合は、入力ファイルと同じディレクトリに出力します。

## オプション

| オプション | 説明 |
|---|---|
| `-h`, `--help` | ヘルプを表示 |
| `-i <file>`, `--ini <file>` | 外部 INI を読み込む |
| `-t <index>` | タイトル・著者の取得方法を指定。`0`: タイトル→著者（既定）、`1`: 著者→タイトル、`2`: タイトル→著者（副題優先）、`3`: タイトルのみ、`4`: タイトル＋著者のみ、`5`: 取得しない |
| `-tf` | 入力ファイル名からタイトル・著者を決める |
| `-c <value>`, `--cover <value>` | 表紙を指定。`0`: 最初の挿絵、`1`: 入力ファイルと同名の画像、または画像ファイル名 |
| `-ext <ext>`, `--ext <ext>` | 出力拡張子を指定。既定は `.epub` |
| `-of`, `--of` | 出力ファイル名に入力ファイル名を使う |
| `-d <dir>`, `--dst <dir>` | 出力先ディレクトリを指定。ディレクトリは事前に作成しておく必要がある |
| `-enc <name>`, `--encoding <name>` | 入力文字コードを指定。`AUTO`（既定）、`MS932`、`UTF-8` など |
| `-hor`, `--horizontal` | 横書きにする |
| `--vertical` | 縦書きにする |
| `-device <name>`, `--device <name>` | 端末固有の出力処理を有効にする。現在は `kindle` を想定 |
| `--language <lang>` | EPUB の言語を指定。既定は `ja` |
| `--creator <name>` | 著者名を上書き |
| `--config-dir <dir>` | 注記定義ファイルなどを読むディレクトリを指定。複数回指定可能 |
| `--preset <file>` | プリセット INI を読み込む |

`-i` / `--ini` と `--preset` は排他的です。

## 配布 ZIP のファイル構成

配布 ZIP は、Java 版 AozoraEpub3 と近い構成になっています。

```text
AozoraEpub3_Lite(.exe)
chuki_*.txt        # 青空文庫注記の変換定義
replace.txt        # 文字置換規則
gaiji/             # 外字用の 1 文字フォント
presets/*.ini      # 変換設定・端末別プリセット
template/          # EPUB テンプレート
README.md
LICENSE.txt
```

通常はこの構成のまま使えば、追加設定は不要です。

実行時には、まず実行ファイルと同じディレクトリにある `chuki_*.txt` や `gaiji/` などを参照します。ソースツリーから開発中に実行した場合は、必要に応じて `assets/aozora/` を使用します。

別の定義ファイル一式を使いたい場合は `--config-dir` で明示できます。

## 設定ファイルとプリセット

配布 ZIP の `presets/` には `reader.ini` のほか、`kindle_pw.ini`、`kindle_fire.ini`、`kobo_touch.ini`、`kobo_glo.ini` などが入っています。

プリセットを使う場合:

```sh
AozoraEpub3_Lite --preset presets/kobo_touch.ini 作品.txt
```

主な設定項目:

- `Vertical`: 縦書き / 横書き
- `TitleType`: タイトル・著者の取得方法
- `PageBreak*`: 改ページ判定のしきい値
- `CoverPage` / `CoverPageToc`: 表紙ページの出力と目次への追加
- `TocPage` / `TocVertical`: 目次ページの出力と縦書き指定
- `NoIllust`: 挿絵を出力しない（表紙と外字画像は残ります）
- `PageMargin` / `BodyMargin` / `LineHeight` / `FontSize` / `BoldUseGothic` / `gothicUseBold`: 本文 CSS
- `CoverW` / `CoverH`: 表紙サイズ
- `FitImage`: 画像を表示領域に収めるかどうか
- `ImageFloatPage` / `ImageFloatBlock`: 画像の回り込み設定
- `SvgImage`: 単ページ画像を SVG 固定レイアウトにするかどうか

`-i` / `--preset` で読み込んだ INI はコマンドラインオプションより優先度が低く、
同じ項目を両方で指定した場合はコマンドラインが優先されます。

## 注記定義ファイル

`chuki_*.txt` には、青空文庫注記を XHTML に変換するための定義が入っています。字下げ、傍点、割り注、外字などの処理で使用します。

ソースツリーでは `assets/aozora/`、配布 ZIP では実行ファイルと同じディレクトリに置かれています。

## 外字フォント

`gaiji/` に GlyphWiki 形式の 1 文字フォントを置くと、対応する外字を EPUB に埋め込めます。

たとえば `u4e35.ttf` を用意すると、`※［＃U+4E35］` のような外字指定に対応できます。フォントは EPUB 内の `fonts/` に格納されます。

フォントは [GlyphWiki](http://glyphwiki.org/wiki/) から入手できます。ファイル名の規則や調整方法は `gaiji/README.txt` を参照してください。

## 生成される EPUB

生成する EPUB の内部構成はおおむね次のようになります。

```text
作品.epub
├── mimetype
├── META-INF/container.xml
└── item/
    ├── standard.opf
    ├── nav.xhtml
    ├── toc.ncx
    ├── xhtml/0001.xhtml ...
    ├── image/
    ├── style/
    └── fonts/          # 外字フォントを使った場合のみ
```

`nav.xhtml` は EPUB 3 のナビゲーション、`toc.ncx` は EPUB 2 系リーダーとの互換用です。

## Rust ライブラリとして使う

変換処理は `aozora_epub3_lite` クレートとしても利用できます。

```sh
cargo add aozora_epub3_lite --git https://github.com/Rumia-Channel/AozoraEpub3_Lite
```

または `Cargo.toml` に直接記述します。

```toml
[dependencies]
aozora_epub3_lite = { git = "https://github.com/Rumia-Channel/AozoraEpub3_Lite" }
```

単純な TXT → EPUB の例:

```rust
use std::fs::{self, File};
use std::path::Path;

use aozora_epub3_lite::{
    AozoraConfig, EpubBook, EpubMetadata,
    aozora_text_to_xhtml_sections_with_config, decode_text,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = AozoraConfig::load_from_dirs(&[Path::new("assets/aozora")], None)?;

    let bytes = fs::read("作品.txt")?;
    let text = decode_text(&bytes, None)?;
    let sections = aozora_text_to_xhtml_sections_with_config(&text, &config)?;

    let metadata = EpubMetadata::new("作品タイトル", "urn:uuid:example");
    let book = EpubBook::from_sections(metadata, sections).with_vertical(true);

    book.write_to(File::create("作品.epub")?)?;
    Ok(())
}
```

この例は本文だけを扱う最小構成です。画像を含む入力では `Input`、`EpubAsset`、`image_references` などを組み合わせます。

主な公開 API:

- `AozoraConfig`: INI、注記定義、外字フォントなどの設定を読み込む
- `aozora_text_to_xhtml_sections*`: 青空文庫形式の本文を XHTML セクションへ変換する
- `EpubBook` / `EpubAsset`: EPUB を組み立てて書き出す
- `Input` / `FileSource`: TXT / ZIP / TXTZ / CBZ や独自の入力元を扱う
- `decode_text`: 入力文字コードを判定して `String` に変換する
- `BookMeta` / `TitleType`: タイトル・著者などのメタデータを推定する

### ストリーミング出力

`EpubBook::write_to_stream` / `write_to_stream_with` は `Seek` を要求せず、`Write` のみで EPUB を出力できます。HTTP レスポンスや Cloudflare Workers など、シークできない出力先に向いています。

`EpubAsset::lazy` と `write_to_stream_with` を組み合わせると、画像をすべてメモリに保持せず、書き出し時に 1 枚ずつ取得できます。

```rust
book.write_to_stream_with(response_body, |epub_path| {
    let name = epub_path.strip_prefix("image/")?;
    input.read_image(name).ok().flatten()
})?;
```

入力側もローカル ZIP ではなくオブジェクトストレージなどから読みたい場合は、`FileSource` を実装して `Input::from_source` に渡せます。

## Java 版 AozoraEpub3 との関係

このプロジェクトは、次の実装を参照・移植しています。

- [hmdev/AozoraEpub3](https://github.com/hmdev/AozoraEpub3)
- [kyukyunyorituryo/AozoraEpub3](https://github.com/kyukyunyorituryo/AozoraEpub3)

ローカル変換の出力について、21 件のテストフィクスチャのうち 18 件が Java 版と byte 一致しています。残る 3 件は、表題前の表紙画像のバッファ処理、注記を含む行のエスケープ挙動（Java 側の退行）、画像のみ EPUB の OPF です。

Java 版で、章名中の `※` の並びによって行が欠落するケースがあります。また `＜＜` / `＞＞` がルビとして解釈され行が欠落するケースがあります。これらの挙動は AozoraEpub3_Lite では意図的に再現していません。前者の詳細は [kyukyunyorituryo/AozoraEpub3#34](https://github.com/kyukyunyorituryo/AozoraEpub3/issues/34) を参照してください。

なお、Java 版 AozoraEpub3 の同梱 jar はビルド時点がリポジトリより古い場合があり、`test_chapter.txt` の変換が例外で終了することがあります。差分を取る場合はリポジトリの `src/` をビルドして参照実装にしてください。

## 対象外

AozoraEpub3_Lite では、次の機能は実装対象としていません。

- GUI
- Web 小説の取得・変換
- RAR 入力

## ライセンス

GNU General Public License v3.0 only（GPL-3.0-only）。詳細は [LICENSE.txt](LICENSE.txt) と [gpl.txt](gpl.txt) を参照してください。