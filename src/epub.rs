use std::fmt;
use std::io::{self, Seek, Write};
use std::path::Path;

use zip::write::{SimpleFileOptions, ZipWriter};
use zip::{CompressionMethod, result::ZipError};
#[path = "epub_render.rs"]
mod render;

use render::{is_image_only, render_cover, render_nav, render_ncx, render_package, render_section};

const MIMETYPE: &str = "application/epub+zip";
const CONTAINER_XML: &str = "<?xml version=\"1.0\"?>\r\n<container\r\n version=\"1.0\"\r\n xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"\r\n>\r\n<rootfiles>\r\n<rootfile\r\n full-path=\"item/standard.opf\"\r\n media-type=\"application/oebps-package+xml\"\r\n/>\r\n</rootfiles>\r\n</container>\r\n";
const BOOK_STYLE_CSS: &str = include_str!("../assets/aozora/template/item/style/book-style.css");
const TEXT_CSS: &str = r#"@charset "utf-8";
@namespace "http://www.w3.org/1999/xhtml";

/** 共通 テキスト用スタイル */
@page {
margin: 0.5em 0.5em 0.5em 0.5em;
}
body {
margin: 0;
padding: 0;
display: block;
color: #000;
font-size: 100%;
line-height: 1.8;
vertical-align: baseline;
}
/** 縦書き テキスト用スタイル */
html.vrtl {
margin: 0em 0em 0em 0em;
padding: 0;
writing-mode: vertical-rl;
-webkit-writing-mode: vertical-rl;
-epub-writing-mode: vertical-rl;
-epub-line-break: strict;
line-break: strict;
-epub-word-break: normal;
word-break: normal;
}


/** 太字、ゴシック */
.vrtl .gtc {
font-family: '@ＭＳ ゴシック','@MS Gothic',sans-serif;
}
.b { font-weight: bold; }
.i { font-style: italic; }

/** 外字フォント */

/** 横書き テキスト用スタイル */

html.hltr {
margin: 0em 0em 0em 0em;
padding: 0;
writing-mode: horizontal-tb;
-webkit-writing-mode: horizontal-tb;
-epub-writing-mode: horizontal-tb;
-epub-line-break: strict;
line-break: strict;
-epub-word-break: normal;
word-break: normal;
}

/** 太字、ゴシック */
.hltr .gtc {
font-family: 'ＭＳ ゴシック','MS Gothic',sans-serif;
}
.hltr .b { font-weight: bold; }
.hltr .i { font-style: italic; }
"#;

fn render_text_css(assets: &[EpubAsset]) -> String {
    let marker = "/** 外字フォント */";
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
    if font_css.is_empty() {
        return TEXT_CSS.to_owned();
    }
    let Some(marker_end) = TEXT_CSS.find(marker).map(|index| index + marker.len()) else {
        return TEXT_CSS.to_owned();
    };
    let mut css = String::with_capacity(TEXT_CSS.len() + font_css.len());
    css.push_str(&TEXT_CSS[..marker_end]);
    css.push('\n');
    css.push_str(&font_css);
    css.push_str(&TEXT_CSS[marker_end..]);
    css
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
            modified: "1970-01-01T00:00:00Z".to_owned(),
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpubSection {
    pub body_fragment: String,
}

impl EpubSection {
    pub fn new(body_fragment: impl Into<String>) -> Self {
        Self {
            body_fragment: body_fragment.into(),
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
}

impl NavChapter {
    pub fn new(label: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            path: path.into(),
            anchor: None,
        }
    }

    pub fn with_anchor(mut self, anchor: impl Into<String>) -> Self {
        self.anchor = Some(anchor.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpubBook {
    pub metadata: EpubMetadata,
    pub sections: Vec<EpubSection>,
    pub assets: Vec<EpubAsset>,
    pub cover_asset: Option<String>,
    pub chapters: Vec<NavChapter>,
    vertical: bool,
    toc_vertical: bool,
    kindle: bool,
    title_markup: Option<String>,
    creator_markup: Option<String>,
    title_page_markup: Option<String>,
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
            chapters: Vec::new(),
            vertical: true,
            toc_vertical: false,
            kindle: false,
            title_markup: None,
            creator_markup: None,
            title_page_markup: None,
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

    pub fn with_vertical(mut self, vertical: bool) -> Self {
        self.vertical = vertical;
        self
    }

    pub fn with_toc_vertical(mut self, toc_vertical: bool) -> Self {
        self.toc_vertical = toc_vertical;
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

/// Writes every EPUB entry into `archive`. Shared by the seekable and
/// streaming writers.
fn write_epub_body<W: Write + Seek>(
    archive: &mut ZipWriter<W>,
    book: &EpubBook,
    provider: &impl Fn(&str) -> Option<Vec<u8>>,
) -> Result<(), EpubError> {
    let image_only = is_image_only(&book.sections);
    write_entry(
        archive,
        "mimetype",
        MIMETYPE.as_bytes(),
        CompressionMethod::Stored,
    )?;
    write_entry(
        archive,
        "META-INF/container.xml",
        CONTAINER_XML.as_bytes(),
        CompressionMethod::Deflated,
    )?;

    if image_only {
        write_entry(
            archive,
            "item/style/fixed-layout-jp.css",
            include_str!("../assets/aozora/template/item/style/fixed-layout-jp.css").as_bytes(),
            CompressionMethod::Deflated,
        )?;
    } else {
        for (name, content) in [
            (
                "item/style/font.css",
                include_str!("../assets/aozora/template/item/style/font.css"),
            ),
            (
                "item/style/aozora.css",
                include_str!("../assets/aozora/template/item/style/aozora.css"),
            ),
            (
                "item/style/fixed-layout-jp.css",
                include_str!("../assets/aozora/template/item/style/fixed-layout-jp.css"),
            ),
            ("item/style/book-style.css", BOOK_STYLE_CSS),
            (
                "item/style/style-reset.css",
                include_str!("../assets/aozora/template/item/style/style-reset.css"),
            ),
            (
                "item/style/style-standard.css",
                include_str!("../assets/aozora/template/item/style/style-standard.css"),
            ),
            (
                "item/style/style-advance.css",
                include_str!("../assets/aozora/template/item/style/style-advance.css"),
            ),
        ] {
            write_entry(
                archive,
                name,
                content.as_bytes(),
                CompressionMethod::Deflated,
            )?;
        }
    }
    if image_only {
        for asset in &book.assets {
            write_asset(archive, asset, provider)?;
        }
    }

    if let Some(cover_asset) = &book.cover_asset
        && !image_only
    {
        write_entry(
            archive,
            "item/cover.xhtml",
            render_cover(&book.metadata, cover_asset, book.kindle).as_bytes(),
            CompressionMethod::Deflated,
        )?;
    }
    for (index, section) in book.sections.iter().enumerate() {
        let path = if is_title_page(section) {
            "item/xhtml/title.xhtml".to_owned()
        } else {
            format!(
                "item/xhtml/{:04}.xhtml",
                book.sections[..=index]
                    .iter()
                    .filter(|section| !is_title_page(section))
                    .count()
            )
        };
        write_entry(
            archive,
            &path,
            render_section(
                &book.metadata,
                &section.body_fragment,
                book.vertical,
                book.kindle,
                book.title_markup.as_deref(),
                book.creator_markup.as_deref(),
                book.title_page_markup.as_deref(),
            )
            .as_bytes(),
            CompressionMethod::Deflated,
        )?;
    }
    if !image_only {
        let text_css = render_text_css(&book.assets);
        write_entry(
            archive,
            "item/style/text.css",
            text_css.as_bytes(),
            CompressionMethod::Deflated,
        )?;
    }
    write_entry(
        archive,
        "item/standard.opf",
        render_package(
            &book.metadata,
            &book.sections,
            &book.assets,
            book.cover_asset.as_deref(),
            book.vertical,
        )
        .as_bytes(),
        CompressionMethod::Deflated,
    )?;
    write_entry(
        archive,
        "item/nav.xhtml",
        render_nav(
            &book.metadata,
            &book.sections,
            book.vertical,
            book.title_markup.as_deref(),
            &book.chapters,
            book.toc_vertical,
        )
        .as_bytes(),
        CompressionMethod::Deflated,
    )?;
    write_entry(
        archive,
        "item/toc.ncx",
        render_ncx(
            &book.metadata,
            &book.sections,
            book.title_markup.as_deref(),
            &book.chapters,
        )
        .as_bytes(),
        CompressionMethod::Deflated,
    )?;
    if !image_only {
        for asset in &book.assets {
            write_asset(archive, asset, provider)?;
        }
    }
    Ok(())
}

/// Writes one asset entry, loading deferred (`data: None`) bytes from
/// `provider` just before writing.
fn write_asset<W: Write + Seek>(
    archive: &mut ZipWriter<W>,
    asset: &EpubAsset,
    provider: &impl Fn(&str) -> Option<Vec<u8>>,
) -> Result<(), EpubError> {
    let path = format!("item/{}", asset.path);
    let data = match &asset.data {
        Some(data) => std::borrow::Cow::Borrowed(data.as_slice()),
        None => std::borrow::Cow::Owned(
            provider(&asset.path).ok_or_else(|| EpubError::MissingAsset(asset.path.clone()))?,
        ),
    };
    write_entry(archive, &path, &data, CompressionMethod::Deflated)
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

fn write_entry<W: Write + Seek>(
    archive: &mut ZipWriter<W>,
    name: &str,
    content: &[u8],
    method: CompressionMethod,
) -> Result<(), EpubError> {
    let options = SimpleFileOptions::default().compression_method(method);
    archive.start_file(name, options)?;
    archive.write_all(content)?;
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
        assert!(section.contains("\r\n class=\"hltr\"\r\n>"));
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
