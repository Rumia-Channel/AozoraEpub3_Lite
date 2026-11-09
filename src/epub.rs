use std::borrow::Cow;
use std::fmt;
use std::io::{self, Seek, Write};
use std::path::Path;

use zip::write::{SimpleFileOptions, StreamWriter, ZipWriter};
use zip::{CompressionMethod, result::ZipError};
#[path = "epub_render.rs"]
mod render;

use render::{is_image_only, render_cover, render_nav, render_ncx, render_package, render_section};

use crate::config::StyleSettings;

const MIMETYPE: &str = "application/epub+zip";
const CONTAINER_XML: &str = "<?xml version=\"1.0\"?>\r\n<container\r\n version=\"1.0\"\r\n xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"\r\n>\r\n<rootfiles>\r\n<rootfile\r\n full-path=\"item/standard.opf\"\r\n media-type=\"application/oebps-package+xml\"\r\n/>\r\n</rootfiles>\r\n</container>\r\n";
const BOOK_STYLE_CSS: &str = include_str!("../assets/aozora/template/item/style/book-style.css");
/// Java `text.vm` を再現する。スタイル値は INI から、外字フォントの
/// `@font-face` は使用したフォントから差し込む。
fn render_text_css(assets: &[EpubAsset], style: &StyleSettings) -> String {
    let mut font_css = String::new();
    for asset in assets {
        if asset.media_type != "application/font-sfnt" {
            continue;
        }
        let Some(file_name) = asset.path.strip_prefix("gaiji/") else {
            continue;
        };
        let Some(stem) = Path::new(file_name)
            .file_stem()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
        else {
            continue;
        };
        font_css.push_str(&format!(
            "@font-face {{font-family:\"{stem}\"; src:url(../gaiji/{file_name});}}\n\
             .{stem} {{font-family:\"{stem}\";}}\n"
        ));
    }
    let (head, tail) = text_css_parts(style);
    let mut css = head;
    css.push('\n');
    css.push_str(&font_css);
    css.push_str(&tail);
    css
}

/// `{fonts}` プレースホルダの前後を `text.vm` の展開結果として組み立てる。
/// `tail` は外字フォント節の後ろの空行から始まる。
fn text_css_parts(style: &StyleSettings) -> (String, String) {
    let page_margin = style.page_margin.join(" ");
    let body_margin = style.body_margin.join(" ");
    let line_height = &style.line_height;
    let font_size = style.font_size;
    // text.vm の `#if`: 有効なときだけセレクタ行が増える。
    let vrtl_bold = if style.bold_use_gothic {
        ".vrtl .b,\n"
    } else {
        ""
    };
    let hltr_bold = if style.bold_use_gothic {
        ".hltr .b,\n"
    } else {
        ""
    };
    let gtc_bold = if style.gothic_use_bold { ".gtc,\n" } else { "" };
    let head = format!(
        r#"@charset "utf-8";
@namespace "http://www.w3.org/1999/xhtml";

/** 共通 テキスト用スタイル */
@page {{
margin: {page_margin};
}}
body {{
margin: 0;
padding: 0;
display: block;
color: #000;
font-size: {font_size}%;
line-height: {line_height};
vertical-align: baseline;
}}
/** 縦書き テキスト用スタイル */
html.vrtl {{
margin: {body_margin};
padding: 0;
writing-mode: vertical-rl;
-webkit-writing-mode: vertical-rl;
-epub-writing-mode: vertical-rl;
-epub-line-break: strict;
line-break: strict;
-epub-word-break: normal;
word-break: normal;
}}


/** 太字、ゴシック */
{vrtl_bold}.vrtl .gtc {{
font-family: '@ＭＳ ゴシック','@MS Gothic',sans-serif;
}}
{gtc_bold}.b {{ font-weight: bold; }}
.i {{ font-style: italic; }}

/** 外字フォント */"#
    );
    let tail = format!(
        r#"
/** 横書き テキスト用スタイル */

html.hltr {{
margin: {body_margin};
padding: 0;
writing-mode: horizontal-tb;
-webkit-writing-mode: horizontal-tb;
-epub-writing-mode: horizontal-tb;
-epub-line-break: strict;
line-break: strict;
-epub-word-break: normal;
word-break: normal;
}}

/** 太字、ゴシック */
{hltr_bold}.hltr .gtc {{
font-family: 'ＭＳ ゴシック','MS Gothic',sans-serif;
}}
{gtc_bold}.hltr .b {{ font-weight: bold; }}
.hltr .i {{ font-style: italic; }}
"#
    );
    (head, tail)
}

const TITLE_PAGE_MARKER: &str = "<!-- aozora-title-page -->";

#[derive(Debug)]
pub enum EpubError {
    Io(io::Error),
    Zip(ZipError),
    InvalidMetadata(&'static str),
    MissingAsset(String),
}

impl fmt::Display for EpubError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::Zip(error) => write!(f, "ZIP error: {error}"),
            Self::InvalidMetadata(field) => write!(f, "metadata field is empty: {field}"),
            Self::MissingAsset(path) => write!(f, "asset data missing: {path}"),
        }
    }
}

impl std::error::Error for EpubError {}

impl From<io::Error> for EpubError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ZipError> for EpubError {
    fn from(error: ZipError) -> Self {
        Self::Zip(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpubMetadata {
    pub title: String,
    pub creator: Option<String>,
    pub publisher: Option<String>,
    pub language: String,
    pub identifier: String,
    pub modified: String,
}

impl EpubMetadata {
    pub fn new(title: impl Into<String>, identifier: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            creator: None,
            publisher: None,
            language: "ja".to_owned(),
            identifier: identifier.into(),
            modified: current_utc_timestamp(),
        }
    }

    pub fn with_creator(mut self, creator: impl Into<String>) -> Self {
        self.creator = Some(creator.into());
        self
    }

    pub fn with_publisher(mut self, publisher: impl Into<String>) -> Self {
        self.publisher = Some(publisher.into());
        self
    }

    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = language.into();
        self
    }

    pub fn with_modified(mut self, modified: impl Into<String>) -> Self {
        self.modified = modified.into();
        self
    }
}

/// UNIX epoch からの秒数。`std::time` は wasm32-unknown-unknown では
/// 実装が無く panic するため、そこでは JS の時計を使う。
#[cfg(not(target_arch = "wasm32"))]
fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(target_arch = "wasm32")]
fn unix_seconds() -> u64 {
    // `Date.now()` はミリ秒精度のエポック時刻を返す。
    (js_sys::Date::now() / 1000.0) as u64
}

/// Java: dcterms:modified は変換時刻 (new Date()) を ISO 8601 UTC で出力する。
/// chrono 等を持たないため UNIX epoch から civil 日付を自前計算する。
fn current_utc_timestamp() -> String {
    let secs = unix_seconds();
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Howard Hinnant's civil-from-days algorithm
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpubSection {
    pub body_fragment: String,
    deferred: bool,
    image_only: bool,
}

impl EpubSection {
    pub fn new(body_fragment: impl Into<String>) -> Self {
        Self {
            body_fragment: body_fragment.into(),
            deferred: false,
            image_only: false,
        }
    }

    /// A body whose XHTML is replayed at write time. No chapter text is retained.
    /// `image_only` records the fixed-layout SVG classification from the first pass.
    pub fn deferred(image_only: bool) -> Self {
        Self {
            body_fragment: String::new(),
            deferred: true,
            image_only,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpubAsset {
    pub path: String,
    pub media_type: String,
    /// Processed image bytes; `None` defers the bytes to the provider passed
    /// to [`EpubBook::write_to_with`] so large images never stay resident.
    pub data: Option<Vec<u8>>,
}

impl EpubAsset {
    pub fn new(
        path: impl Into<String>,
        media_type: impl Into<String>,
        data: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            path: path.into(),
            media_type: media_type.into(),
            data: Some(data.into()),
        }
    }

    /// Creates an asset whose bytes are supplied lazily at write time via
    /// [`EpubBook::write_to_with`]. Keeps large images out of memory.
    pub fn lazy(path: impl Into<String>, media_type: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            media_type: media_type.into(),
            data: None,
        }
    }
}

/// A navigation chapter for the nav/NCX pages.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NavChapter {
    pub label: String,
    pub path: String,
    pub anchor: Option<String>,
    /// Raw heading level before TOC nesting: 1 for page-break chapters and
    /// 大見出し, 2 for 中見出し, 3 for 小見出し (mirrors
    /// `ChapterLineInfo.getLevel`). The nav/NCX renderers convert this into
    /// the actual nesting depth.
    pub level: u8,
    /// ラベルが既に XHTML（エスケープ済み・縦中横タグ入り）なら true。
    pub markup: bool,
}

impl NavChapter {
    pub fn new(label: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            path: path.into(),
            anchor: None,
            level: 1,
            markup: false,
        }
    }

    pub fn with_anchor(mut self, anchor: impl Into<String>) -> Self {
        self.anchor = Some(anchor.into());
        self
    }

    /// 目次ラベルを XHTML としてそのまま出力する (Java `TocVertical` の
    /// `convertTcyText` 適用済みラベル用)。
    pub fn with_markup(mut self, markup: bool) -> Self {
        self.markup = markup;
        self
    }

    pub fn with_level(mut self, level: u8) -> Self {
        self.level = level;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpubBook {
    pub metadata: EpubMetadata,
    pub sections: Vec<EpubSection>,
    pub assets: Vec<EpubAsset>,
    pub cover_asset: Option<String>,
    /// 表紙画像の元寸法。Java `cover.vm` の viewport / viewBox に使う。
    pub cover_dimensions: Option<(u32, u32)>,
    /// `text.css` に展開するスタイル設定 (Java `text.vm` の変数)。
    pub style: StyleSettings,
    /// `CoverPage` INI key: 表紙ページを xhtml/cover.xhtml に出力する。
    insert_cover_page: bool,
    /// `CoverPageToc` INI key: 目次に表紙への項目を追加する。
    cover_page_toc: bool,
    pub chapters: Vec<NavChapter>,
    vertical: bool,
    toc_vertical: bool,
    kindle: bool,
    /// `TocPage` INI key: insert `nav.xhtml` into the spine after the title
    /// page so the TOC is a visible page (Java `bookInfo.insertTocPage`).
    toc_page: bool,
    /// `NavNest` INI key: nest the nav.xhtml `<ol>` by chapter level.
    nav_nest: bool,
    /// `NcxNest` INI key: nest toc.ncx navPoints by chapter level.
    ncx_nest: bool,
    /// `TitleToc` INI key (Java default true): include the title in the TOC.
    title_toc: bool,
    title_markup: Option<String>,
    creator_markup: Option<String>,
    title_page_markup: Option<String>,
    /// `TitlePage` INI key: 1=TITLE_MIDDLE (title_middle.vm), 2=TITLE_HORIZONTAL
    /// (title_horizontal.vm)。Java `bookInfo.titlePageType` 相当。
    title_page_type: usize,
}
impl EpubBook {
    pub fn new(metadata: EpubMetadata, body_fragment: impl Into<String>) -> Self {
        Self::from_sections(metadata, [body_fragment.into()])
    }

    pub fn from_sections<I, S>(metadata: EpubMetadata, sections: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let sections = sections
            .into_iter()
            .map(EpubSection::new)
            .collect::<Vec<_>>();
        let sections = if sections.is_empty() {
            vec![EpubSection::new("")]
        } else {
            sections
        };
        Self {
            metadata,
            sections,
            assets: Vec::new(),
            cover_asset: None,
            cover_dimensions: None,
            style: StyleSettings::default(),
            insert_cover_page: false,
            cover_page_toc: false,
            chapters: Vec::new(),
            vertical: true,
            toc_vertical: false,
            kindle: false,
            toc_page: false,
            nav_nest: false,
            ncx_nest: false,
            title_toc: true,
            title_markup: None,
            creator_markup: None,
            title_page_markup: None,
            title_page_type: 0,
        }
    }

    pub fn with_assets<I>(mut self, assets: I) -> Self
    where
        I: IntoIterator<Item = EpubAsset>,
    {
        self.assets = assets.into_iter().collect();
        self
    }

    pub fn with_cover_asset(mut self, path: impl Into<String>) -> Self {
        self.cover_asset = Some(path.into());
        self
    }

    /// 表紙画像の元寸法。Java `cover.vm` の `<meta name="viewport">` と
    /// `<svg viewBox>` に出力される。
    pub fn with_cover_dimensions(mut self, dimensions: Option<(u32, u32)>) -> Self {
        self.cover_dimensions = dimensions;
        self
    }

    /// `text.css` のスタイル設定 (Java `text.vm` の変数) を差し替える。
    pub fn with_style(mut self, style: StyleSettings) -> Self {
        self.style = style;
        self
    }

    /// `CoverPage` / `CoverPageToc` INI keys。表紙ページを出さない場合でも
    /// 表紙画像は OPF の `cover-image` として残る (Java と同じ)。
    pub fn with_cover_page(mut self, insert_cover_page: bool, cover_page_toc: bool) -> Self {
        self.insert_cover_page = insert_cover_page;
        self.cover_page_toc = cover_page_toc;
        self
    }

    /// Java `bookInfo.insertCoverPage`: 表紙画像が無ければ表紙ページは出ない。
    fn cover_page_enabled(&self) -> bool {
        self.insert_cover_page && self.cover_asset.is_some()
    }

    pub fn with_vertical(mut self, vertical: bool) -> Self {
        self.vertical = vertical;
        self
    }

    pub fn with_toc_vertical(mut self, toc_vertical: bool) -> Self {
        self.toc_vertical = toc_vertical;
        self
    }

    /// `TocPage` INI key: add `nav.xhtml` to the spine after the title page.
    pub fn with_toc_page(mut self, toc_page: bool) -> Self {
        self.toc_page = toc_page;
        self
    }

    /// `NavNest`/`NcxNest` INI keys: nest nav.xhtml `<ol>` and toc.ncx
    /// navPoints by chapter level.
    pub fn with_toc_nest(mut self, nav_nest: bool, ncx_nest: bool) -> Self {
        self.nav_nest = nav_nest;
        self.ncx_nest = ncx_nest;
        self
    }

    /// `TitleToc` INI key (Java default true): include the title in the TOC.
    pub fn with_title_toc(mut self, title_toc: bool) -> Self {
        self.title_toc = title_toc;
        self
    }

    pub fn with_chapters<I>(mut self, chapters: I) -> Self
    where
        I: IntoIterator<Item = NavChapter>,
    {
        self.chapters = chapters.into_iter().collect();
        self
    }

    pub fn with_kindle(mut self, kindle: bool) -> Self {
        self.kindle = kindle;
        self
    }
    pub fn with_metadata_markup(
        mut self,
        title_markup: impl Into<String>,
        creator_markup: Option<String>,
    ) -> Self {
        self.title_markup = Some(title_markup.into());
        self.creator_markup = creator_markup;
        self
    }
    pub fn with_title_page_markup(mut self, markup: impl Into<String>) -> Self {
        self.title_page_markup = Some(markup.into());
        self
    }
    pub fn with_title_page_type(mut self, title_page_type: usize) -> Self {
        self.title_page_type = title_page_type;
        self
    }

    pub fn with_title_page(mut self) -> Self {
        if !self.sections.iter().any(is_title_page) {
            self.sections.insert(0, EpubSection::new(TITLE_PAGE_MARKER));
        }
        self
    }

    pub fn with_title_page_if(mut self, enabled: bool) -> Self {
        if enabled {
            self = self.with_title_page();
        }
        self
    }

    pub fn write_to<W: Write + Seek>(&self, output: W) -> Result<W, EpubError> {
        self.write_to_with(output, |_| None)
    }

    /// Writes the EPUB, resolving `EpubAsset`s whose `data` is `None` through
    /// `provider` (keyed by the asset's EPUB path, e.g. `"image/0001.jpg"`)
    /// one asset at a time. Keeps large image sets out of memory.
    pub fn write_to_with<W: Write + Seek>(
        &self,
        output: W,
        provider: impl Fn(&str) -> Option<Vec<u8>>,
    ) -> Result<W, EpubError> {
        self.validate()?;
        let mut archive = ZipWriter::new(output);
        write_epub_body(&mut archive, self, &provider)?;
        Ok(archive.finish()?)
    }

    /// Writes the EPUB to a `Write`-only sink without ever seeking: every
    /// entry is streamed with a ZIP data descriptor, so the whole book can be
    /// produced end-to-end without buffering it in memory. Suitable for
    /// response bodies (Cloudflare Workers, HTTP servers, ...) where the
    /// output cannot be seeked.
    pub fn write_to_stream<W: Write>(&self, output: W) -> Result<W, EpubError> {
        self.write_to_stream_with(output, |_| None)
    }

    /// Like [`EpubBook::write_to_stream`], resolving deferred assets
    /// (`data: None`) through `provider` one at a time.
    pub fn write_to_stream_with<W: Write>(
        &self,
        output: W,
        provider: impl Fn(&str) -> Option<Vec<u8>>,
    ) -> Result<W, EpubError> {
        self.validate()?;
        let mut archive = ZipWriter::new_stream(output);
        write_epub_body(&mut archive, self, &provider)?;
        Ok(archive.finish()?.into_inner())
    }

    fn validate(&self) -> Result<(), EpubError> {
        validate_metadata(&self.metadata)?;
        for asset in &self.assets {
            validate_asset(asset)?;
        }
        if let Some(cover_asset) = &self.cover_asset
            && !self.assets.iter().any(|asset| &asset.path == cover_asset)
        {
            return Err(EpubError::InvalidMetadata("cover asset"));
        }
        Ok(())
    }
}

/// An entry plan contains only names and references, never rendered book bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EpubEntrySource {
    Static(&'static [u8]),
    Asset(usize),
    EmbeddedAsset(usize),
    Section(usize),
    Cover,
    TextStyle,
    Package,
    Navigation,
    Ncx,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpubEntry {
    name: String,
    method: CompressionMethod,
    source: EpubEntrySource,
}

impl EpubEntry {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn asset_path(&self) -> Option<&str> {
        match &self.source {
            EpubEntrySource::Asset(_) => self.name.strip_prefix("item/"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpubEntryInfo {
    pub name: String,
    /// Present only when asset bytes must be supplied by the caller.
    pub asset_path: Option<String>,
    /// The book section index when its body must be replayed by the caller.
    pub section_index: Option<usize>,
}

/// A pull-driven ZIP writer. Start an entry, supply bounded chunks, then end it.
/// The sink is called synchronously for each chunk; callers using an asynchronous
/// transport must drain that sink before supplying another chunk.
pub struct EpubStreamWriter<W: Write> {
    archive: ZipWriter<StreamWriter<W>>,
    book: EpubBook,
    entries: Vec<EpubEntry>,
    index: usize,
    open: bool,
    failed: bool,
}

impl EpubBook {
    pub fn entries(&self) -> Result<Vec<EpubEntry>, EpubError> {
        self.validate()?;
        Ok(entries_for(self))
    }

    /// Takes ownership so neither chapter fragments nor embedded assets are cloned.
    pub fn stream_writer<W: Write>(self, sink: W) -> Result<EpubStreamWriter<W>, EpubError> {
        let entries = self.entries()?;
        Ok(EpubStreamWriter {
            archive: ZipWriter::new_stream(sink),
            book: self,
            entries,
            index: 0,
            open: false,
            failed: false,
        })
    }
}

impl<W: Write> EpubStreamWriter<W> {
    pub fn next_entry(&self) -> Option<EpubEntryInfo> {
        self.entries.get(self.index).map(|entry| EpubEntryInfo {
            name: entry.name.clone(),
            asset_path: entry.asset_path().map(str::to_owned),
            section_index: match entry.source {
                EpubEntrySource::Section(index) if self.book.sections[index].deferred => Some(index),
                _ => None,
            },
        })
    }

    /// Renders only the current generated entry. Deferred sections require the
    /// replayed fragment; external assets are written directly with `write_chunk`.
    /// Embedded asset buffers are moved out, so each current entry is rendered once.
    pub fn render_current(&mut self, section: Option<&str>) -> Result<Vec<u8>, EpubError> {
        self.check_active()?;
        let entry = self.entries.get(self.index)
            .ok_or(EpubError::InvalidMetadata("write past the last EPUB entry"))?;
        if let EpubEntrySource::EmbeddedAsset(index) = entry.source {
            let asset = &mut self.book.assets[index];
            return asset.data.take().ok_or_else(|| EpubError::MissingAsset(asset.path.clone()));
        }
        Ok(render_entry(&self.book, entry, section)?.into_owned())
    }

    pub fn begin_current(&mut self) -> Result<(), EpubError> {
        self.check_active()?;
        if self.open {
            return Err(EpubError::InvalidMetadata("EPUB entry already open"));
        }
        let entry = self.entries.get(self.index)
            .ok_or(EpubError::InvalidMetadata("write past the last EPUB entry"))?;
        let options = SimpleFileOptions::default().compression_method(entry.method);
        if let Err(error) = self.archive.start_file(entry.name.as_str(), options) {
            self.failed = true;
            return Err(error.into());
        }
        self.open = true;
        Ok(())
    }

    pub fn write_chunk(&mut self, bytes: &[u8]) -> Result<(), EpubError> {
        self.check_active()?;
        if !self.open {
            return Err(EpubError::InvalidMetadata("EPUB entry is not open"));
        }
        for chunk in bytes.chunks(64 * 1024) {
            if let Err(error) = self.archive.write_all(chunk) {
                self.failed = true;
                return Err(error.into());
            }
        }
        Ok(())
    }

    pub fn end_current(&mut self) -> Result<(), EpubError> {
        self.check_active()?;
        if !self.open {
            return Err(EpubError::InvalidMetadata("EPUB entry is not open"));
        }
        // zip finalizes the descriptor at the next start_file (or finish).
        // Flushing here delivers compressed data before the producer reads ahead.
        if let Err(error) = self.archive.flush() {
            self.failed = true;
            return Err(error.into());
        }
        self.open = false;
        self.index += 1;
        Ok(())
    }

    /// Native whole-entry convenience boundary. Streaming consumers use the
    /// begin/write_chunk/end methods instead.
    pub fn write_current(&mut self, asset: Option<&[u8]>) -> Result<(), EpubError> {
        let rendered;
        let bytes = if let Some(path) = self.next_entry().and_then(|info| info.asset_path) {
            asset.ok_or(EpubError::MissingAsset(path))?
        } else {
            rendered = self.render_current(None)?;
            &rendered
        };
        self.begin_current()?;
        self.write_chunk(bytes)?;
        self.end_current()
    }

    fn check_active(&self) -> Result<(), EpubError> {
        if self.failed {
            return Err(EpubError::InvalidMetadata("EPUB writer failed"));
        }
        Ok(())
    }

    /// Incomplete or failed archives are errors, never successful partial books.
    pub fn finish(self) -> Result<W, EpubError> {
        self.check_active()?;
        if self.open || self.index != self.entries.len() {
            return Err(EpubError::InvalidMetadata("EPUB entries are incomplete"));
        }
        Ok(self.archive.finish()?.into_inner())
    }
}

fn push_static(entries: &mut Vec<EpubEntry>, name: &str, content: &'static [u8], method: CompressionMethod) {
    entries.push(EpubEntry {
        name: name.to_owned(),
        method,
        source: EpubEntrySource::Static(content),
    });
}

fn push_generated(entries: &mut Vec<EpubEntry>, name: String, source: EpubEntrySource) {
    entries.push(EpubEntry { name, method: CompressionMethod::Deflated, source });
}

fn push_asset(entries: &mut Vec<EpubEntry>, index: usize, asset: &EpubAsset) {
    let source = if asset.data.is_some() {
        EpubEntrySource::EmbeddedAsset(index)
    } else {
        EpubEntrySource::Asset(index)
    };
    push_generated(entries, format!("item/{}", asset.path), source);
}

fn entries_for(book: &EpubBook) -> Vec<EpubEntry> {
    let mut entries = Vec::new();
    let image_only = is_image_only(&book.sections);
    push_static(&mut entries, "mimetype", MIMETYPE.as_bytes(), CompressionMethod::Stored);
    push_static(&mut entries, "META-INF/container.xml", CONTAINER_XML.as_bytes(), CompressionMethod::Deflated);
    if image_only {
        push_static(&mut entries, "item/style/fixed-layout-jp.css",
            include_bytes!("../assets/aozora/template/item/style/fixed-layout-jp.css"), CompressionMethod::Deflated);
    } else {
        for (name, content) in [
            ("item/style/font.css", include_str!("../assets/aozora/template/item/style/font.css")),
            ("item/style/aozora.css", include_str!("../assets/aozora/template/item/style/aozora.css")),
            ("item/style/fixed-layout-jp.css", include_str!("../assets/aozora/template/item/style/fixed-layout-jp.css")),
            ("item/style/book-style.css", BOOK_STYLE_CSS),
            ("item/style/style-reset.css", include_str!("../assets/aozora/template/item/style/style-reset.css")),
            ("item/style/style-standard.css", include_str!("../assets/aozora/template/item/style/style-standard.css")),
            ("item/style/style-advance.css", include_str!("../assets/aozora/template/item/style/style-advance.css")),
        ] {
            push_static(&mut entries, name, content.as_bytes(), CompressionMethod::Deflated);
        }
    }
    if image_only {
        for (index, asset) in book.assets.iter().enumerate() {
            push_asset(&mut entries, index, asset);
        }
    }
    if !image_only && book.cover_page_enabled() {
        push_generated(&mut entries, "item/xhtml/cover.xhtml".to_owned(), EpubEntrySource::Cover);
    }
    let mut body_number = 0;
    for (index, section) in book.sections.iter().enumerate() {
        let name = if is_title_page(section) {
            "item/xhtml/title.xhtml".to_owned()
        } else {
            body_number += 1;
            format!("item/xhtml/{body_number:04}.xhtml")
        };
        push_generated(&mut entries, name, EpubEntrySource::Section(index));
    }
    if !image_only {
        push_generated(&mut entries, "item/style/text.css".to_owned(), EpubEntrySource::TextStyle);
    }
    push_generated(&mut entries, "item/standard.opf".to_owned(), EpubEntrySource::Package);
    push_generated(&mut entries, "item/nav.xhtml".to_owned(), EpubEntrySource::Navigation);
    push_generated(&mut entries, "item/toc.ncx".to_owned(), EpubEntrySource::Ncx);
    if !image_only {
        for (index, asset) in book.assets.iter().enumerate() {
            push_asset(&mut entries, index, asset);
        }
    }
    entries
}

fn render_entry<'a>(book: &'a EpubBook, entry: &EpubEntry, body: Option<&str>) -> Result<Cow<'a, [u8]>, EpubError> {
    let text = match &entry.source {
        EpubEntrySource::Static(bytes) => return Ok(Cow::Borrowed(bytes)),
        EpubEntrySource::Asset(index) | EpubEntrySource::EmbeddedAsset(index) => {
            let asset = &book.assets[*index];
            let data = asset.data.as_deref()
                .ok_or_else(|| EpubError::MissingAsset(asset.path.clone()))?;
            return Ok(Cow::Borrowed(data));
        }
        EpubEntrySource::Section(index) => {
            let section = &book.sections[*index];
            let body = if section.deferred {
                body.ok_or(EpubError::InvalidMetadata("deferred EPUB section body missing"))?
            } else {
                &section.body_fragment
            };
            let extra_css = book.assets.iter().map(|asset| &asset.path)
                .filter(|path| path.starts_with("style/") && path.ends_with(".css"))
                .cloned().collect::<Vec<_>>();
            render_section(&book.metadata, body, book.vertical, book.kindle,
                book.title_markup.as_deref(), book.creator_markup.as_deref(),
                book.title_page_markup.as_deref(), book.title_page_type, &extra_css)
        }
        EpubEntrySource::Cover => render_cover(&book.metadata,
            book.cover_asset.as_deref().ok_or(EpubError::InvalidMetadata("cover asset"))?,
            book.cover_dimensions, book.kindle),
        EpubEntrySource::TextStyle => render_text_css(&book.assets, &book.style),
        EpubEntrySource::Package => render_package(&book.metadata, &book.sections, &book.assets,
            book.cover_asset.as_deref(), book.vertical, book.toc_page, book.cover_page_enabled()),
        EpubEntrySource::Navigation => render_nav(&book.metadata, &book.sections, book.vertical,
            &book.metadata.title, &book.chapters, book.toc_vertical, book.toc_page, book.nav_nest,
            book.title_toc, book.cover_page_enabled(), book.cover_page_enabled() && book.cover_page_toc),
        EpubEntrySource::Ncx => render_ncx(&book.metadata, &book.sections, &book.metadata.title,
            &book.chapters, book.ncx_nest, book.title_toc, book.cover_page_enabled() && book.cover_page_toc),
    };
    Ok(Cow::Owned(text.into_bytes()))
}

fn write_epub_body<W: Write + Seek>(
    archive: &mut ZipWriter<W>,
    book: &EpubBook,
    provider: &impl Fn(&str) -> Option<Vec<u8>>,
) -> Result<(), EpubError> {
    for entry in entries_for(book) {
        let options = SimpleFileOptions::default().compression_method(entry.method);
        let bytes = match render_entry(book, &entry, None) {
            Err(EpubError::MissingAsset(path)) => Cow::Owned(
                provider(&path).ok_or(EpubError::MissingAsset(path))?),
            result => result?,
        };
        archive.start_file(entry.name.as_str(), options)?;
        archive.write_all(&bytes)?;
        archive.flush()?;
    }
    Ok(())
}

fn validate_asset(asset: &EpubAsset) -> Result<(), EpubError> {
    if asset.path.trim().is_empty() {
        return Err(EpubError::InvalidMetadata("asset path"));
    }
    if asset.media_type.trim().is_empty() {
        return Err(EpubError::InvalidMetadata("asset media type"));
    }
    if asset
        .path
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(EpubError::InvalidMetadata("asset path"));
    }
    Ok(())
}

fn validate_metadata(metadata: &EpubMetadata) -> Result<(), EpubError> {
    if metadata.title.trim().is_empty() {
        return Err(EpubError::InvalidMetadata("title"));
    }
    if metadata.language.trim().is_empty() {
        return Err(EpubError::InvalidMetadata("language"));
    }
    if metadata.identifier.trim().is_empty() {
        return Err(EpubError::InvalidMetadata("identifier"));
    }
    if metadata.modified.trim().is_empty() {
        return Err(EpubError::InvalidMetadata("modified"));
    }
    Ok(())
}

fn is_title_page(section: &EpubSection) -> bool {
    section.body_fragment.trim() == TITLE_PAGE_MARKER
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use zip::ZipArchive;

    use super::{EpubAsset, EpubBook, EpubError, EpubMetadata};

    #[test]
    fn rejects_empty_title() {
        let book = EpubBook::new(EpubMetadata::new("", "urn:uuid:test"), "<p>本文</p>");
        let error = book.write_to(Cursor::new(Vec::new())).unwrap_err();
        assert!(matches!(error, EpubError::InvalidMetadata("title")));
    }

    #[test]
    fn writes_title_page_and_heading_based_navigation() {
        let metadata = EpubMetadata::new("題名", "urn:uuid:test").with_creator("著者");
        let book =
            EpubBook::new(metadata, "<h1 class=\"title\">第一章</h1><p>本文</p>").with_title_page();
        let cursor = book.write_to(Cursor::new(Vec::new())).unwrap();
        let mut archive = ZipArchive::new(cursor).unwrap();

        let mut title_page = String::new();
        archive
            .by_name("item/xhtml/title.xhtml")
            .unwrap()
            .read_to_string(&mut title_page)
            .unwrap();
        assert!(title_page.contains("<body class=\"p-titlepage\">"));
        assert!(title_page.contains("<div class=\"author\"><p>著者</p></div>"));
        assert!(title_page.contains("<div class=\"book-title"));

        let mut package = String::new();
        archive
            .by_name("item/standard.opf")
            .unwrap()
            .read_to_string(&mut package)
            .unwrap();
        assert!(package.contains("href=\"xhtml/title.xhtml\""));
        assert!(package.contains("href=\"xhtml/0001.xhtml\""));
        assert!(package.contains("<spine page-progression-direction=\"rtl\""));

        let mut nav = String::new();
        archive
            .by_name("item/nav.xhtml")
            .unwrap()
            .read_to_string(&mut nav)
            .unwrap();
        // EpubBook-level nav: title page landmark + first-body fallback
        assert!(nav.contains("epub:type=\"titlepage\""));
        assert!(nav.contains(">本文</a>"));
    }
    #[test]
    fn renders_optional_metadata_markup_in_title_and_navigation() {
        let book = EpubBook::new(EpubMetadata::new("題名", "urn:uuid:test"), "<p>本文</p>")
            .with_title_page()
            .with_metadata_markup(
                "<ruby>題<rt>だい</rt>名</ruby>",
                Some("<ruby>著者<rt>ちょしゃ</rt></ruby>".to_owned()),
            );
        let mut archive = ZipArchive::new(book.write_to(Cursor::new(Vec::new())).unwrap()).unwrap();
        let mut title_page = String::new();
        archive
            .by_name("item/xhtml/title.xhtml")
            .unwrap()
            .read_to_string(&mut title_page)
            .unwrap();
        assert!(title_page.contains("<ruby>題<rt>だい</rt>名</ruby>"));
        let mut nav = String::new();
        archive
            .by_name("item/nav.xhtml")
            .unwrap()
            .read_to_string(&mut nav)
            .unwrap();
        assert!(nav.contains("epub:type=\"titlepage\""));
        assert!(!nav.contains("<ruby>題<rt>だい</rt>名</ruby></a>"));
    }

    #[test]
    fn renders_image_section_as_horizontal_page() {
        let book = EpubBook::new(
            EpubMetadata::new("題名", "urn:uuid:test"),
            "<p><img class=\"fit\" src=\"../image/fig.png\" alt=\"挿絵\"/></p>",
        );
        let cursor = book.write_to(Cursor::new(Vec::new())).unwrap();
        let mut archive = ZipArchive::new(cursor).unwrap();
        let mut section = String::new();
        archive
            .by_name("item/xhtml/0001.xhtml")
            .unwrap()
            .read_to_string(&mut section)
            .unwrap();
        // 単ページ画像セクションは横書き固定レイアウト (Java: <html ... class="hltr">)。
        // セクション xhtml の改行は Java と同じ LF。
        assert!(section.contains("\n class=\"hltr\"\n>"), "{section}");
        assert!(section.contains("<body class=\"p-image\">"));
        assert!(section.contains("<img class=\"fit\" src=\"../image/fig.png\""));
    }

    #[test]
    fn renders_svg_image_section_with_fixed_layout_viewport() {
        let body = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 480"><image width="640" height="480" xlink:href="../image/0001.png"/></svg>"#;
        let book = EpubBook::new(EpubMetadata::new("画像", "urn:uuid:test"), body);
        let cursor = book.write_to(Cursor::new(Vec::new())).unwrap();
        let mut archive = ZipArchive::new(cursor).unwrap();
        let mut section = String::new();
        archive
            .by_name("item/xhtml/0001.xhtml")
            .unwrap()
            .read_to_string(&mut section)
            .unwrap();
        assert!(section.contains("fixed-layout-jp.css"));
        assert!(section.contains("name=\"viewport\" content=\"width=640, height=480\""));
        assert!(section.contains("<svg xmlns=\"http://www.w3.org/2000/svg\""));
    }

    #[test]
    fn renders_page_alignment_and_horizontal_layout() {
        let book = EpubBook::from_sections(
            EpubMetadata::new("題名", "urn:uuid:test"),
            [
                "<!-- aozora-page-middle -->\n<p>中央</p>",
                "<!-- aozora-page-bottom -->\n<p>下</p>",
            ],
        )
        .with_vertical(false);
        let cursor = book.write_to(Cursor::new(Vec::new())).unwrap();
        let mut archive = ZipArchive::new(cursor).unwrap();

        let mut package = String::new();
        archive
            .by_name("item/standard.opf")
            .unwrap()
            .read_to_string(&mut package)
            .unwrap();
        assert!(package.contains("page-progression-direction=\"ltr\""));

        let mut middle = String::new();
        archive
            .by_name("item/xhtml/0001.xhtml")
            .unwrap()
            .read_to_string(&mut middle)
            .unwrap();
        assert!(middle.contains("class=\"hltr\""));
        assert!(middle.contains("<body class=\"p-text\">"));
        assert!(middle.contains("block-align-center"));
    }

    #[test]
    fn omits_colophon_page_from_navigation() {
        let book = EpubBook::from_sections(
            EpubMetadata::new("題名", "urn:uuid:test"),
            [
                "<p>本文</p>",
                "<!-- aozora-page-no-chapter -->\n<p>底本：青空文庫</p>",
            ],
        );
        let cursor = book.write_to(Cursor::new(Vec::new())).unwrap();
        let mut archive = ZipArchive::new(cursor).unwrap();
        let mut nav = String::new();
        archive
            .by_name("item/nav.xhtml")
            .unwrap()
            .read_to_string(&mut nav)
            .unwrap();
        assert!(nav.contains("xhtml/0001.xhtml"));
        assert!(!nav.contains("xhtml/0002.xhtml"));
    }

    #[test]
    fn streams_the_same_entries_as_seekable_write() {
        let book = EpubBook::new(
            EpubMetadata::new("題名", "urn:uuid:test").with_creator("著者"),
            "<p>本文</p>",
        )
        .with_vertical(true);
        let mut seekable = Cursor::new(Vec::new());
        book.write_to(&mut seekable).unwrap();
        let mut streamed = Cursor::new(Vec::new());
        book.write_to_stream(&mut streamed).unwrap();

        let mut z1 = ZipArchive::new(Cursor::new(seekable.into_inner())).unwrap();
        let mut z2 = ZipArchive::new(Cursor::new(streamed.into_inner())).unwrap();
        assert_eq!(z1.len(), z2.len());
        let names = (0..z1.len())
            .map(|index| z1.by_index(index).unwrap().name().to_owned())
            .collect::<Vec<_>>();
        let stream_names = (0..z2.len())
            .map(|index| z2.by_index(index).unwrap().name().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, stream_names);
        for name in names {
            let mut expected = Vec::new();
            z1.by_name(&name)
                .unwrap()
                .read_to_end(&mut expected)
                .unwrap();
            let mut actual = Vec::new();
            z2.by_name(&name).unwrap().read_to_end(&mut actual).unwrap();
            assert_eq!(expected, actual, "entry {name} differs");
        }
    }

    #[test]
    fn streams_deferred_assets_through_the_provider() {
        let book = EpubBook::new(EpubMetadata::new("題名", "urn:uuid:test"), "<p>本文</p>")
            .with_assets([EpubAsset::lazy("image/0001.png", "image/png")]);
        let mut streamed = Cursor::new(Vec::new());
        book.write_to_stream_with(&mut streamed, |path| {
            (path == "image/0001.png").then(|| b"png-data".to_vec())
        })
        .unwrap();
        let mut archive = ZipArchive::new(Cursor::new(streamed.into_inner())).unwrap();
        let mut data = Vec::new();
        archive
            .by_name("item/image/0001.png")
            .unwrap()
            .read_to_end(&mut data)
            .unwrap();
        assert_eq!(data, b"png-data");
    }
}
