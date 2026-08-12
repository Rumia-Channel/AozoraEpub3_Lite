use std::fmt;
use std::io::{self, Seek, Write};

use zip::write::{SimpleFileOptions, ZipWriter};
use zip::{CompressionMethod, result::ZipError};
#[path = "epub_render.rs"]
mod render;

use render::{render_cover, render_nav, render_ncx, render_package, render_section};

const MIMETYPE: &str = "application/epub+zip";
const CONTAINER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="item/standard.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"#;
const BOOK_STYLE_CSS: &str = include_str!("../assets/aozora/template/item/style/book-style.css");
const TEXT_CSS: &str = r#"@charset "UTF-8";

@page {
  margin: .5em;
}

html.vrtl {
  writing-mode: vertical-rl;
  -webkit-writing-mode: vertical-rl;
  -epub-writing-mode: vertical-rl;
  line-break: strict;
  word-break: normal;
}

html.hltr {
  writing-mode: horizontal-tb;
  -webkit-writing-mode: horizontal-tb;
  -epub-writing-mode: horizontal-tb;
}

body {
  margin: 0;
  padding: 0;
}

.main {
  margin: 1em;
}

p {
  margin: 0;
  line-height: 1.8;
}

.p-titlepage {
  text-align: center;
}

.book-title {
  margin-top: 30vh;
}

.book-title-main {
  font-size: 2em;
  font-weight: bold;
}

.author {
  margin-top: 2em;
}

.p-image .main {
  margin: 0;
  text-align: center;
}

.p-image .fit {
  max-width: 100%;
  max-height: 100vh;
  object-fit: contain;
}

h1, h2, h3, h4, h5, h6 {
  margin: 0;
  font-weight: normal;
}

.vrtl h1, .vrtl h2, .vrtl h3, .vrtl h4, .vrtl h5, .vrtl h6 {
  font-family: serif;
}

.hltr h1, .hltr h2, .hltr h3, .hltr h4, .hltr h5, .hltr h6 {
  font-family: serif;
}

.font-1em50 { font-size: 1.5em; }
.font-1em30 { font-size: 1.3em; }
.font-1em10 { font-size: 1.1em; }
.bold { font-weight: bold; }
.italic { font-style: italic; }
.gfont { font-family: sans-serif; }
.em-sesame { text-emphasis: filled sesame; -webkit-text-emphasis: filled sesame; }
.wrc { font-size: .75em; }
.super { vertical-align: super; font-size: .75em; }
.sub { vertical-align: sub; font-size: .75em; }
.kogaki { font-size: .75em; }
.upr { text-orientation: upright; }
.swr, .yoko { writing-mode: horizontal-tb; }
.btm { margin-top: auto; }
.pt1 { padding-top: 1em; }
.pt2 { padding-top: 2em; }
.pt3 { padding-top: 3em; }

hr {
  border: 0;
  border-top: 1px solid currentColor;
  margin: .5em 0;
}

a {
  color: inherit;
}

.mt1 { margin-top: 1em; }
.mt2 { margin-top: 2em; }
.mt3 { margin-top: 3em; }
.introduction, .postscript { margin: 1em 0; }
.running_head { font-size: .8em; }
.clear { clear: both; }

.p-middle .main {
  min-height: calc(100vh - 2em);
  display: flex;
  flex-direction: column;
  justify-content: center;
}

.p-bottom .main {
  min-height: calc(100vh - 2em);
  display: flex;
  flex-direction: column;
  justify-content: flex-end;
}
.tcy { text-combine-upright: all; -epub-text-combine: horizontal; }
"#;

const TITLE_PAGE_MARKER: &str = "<!-- aozora-title-page -->";

#[derive(Debug)]
pub enum EpubError {
    Io(io::Error),
    Zip(ZipError),
    InvalidMetadata(&'static str),
}

impl fmt::Display for EpubError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::Zip(error) => write!(f, "ZIP error: {error}"),
            Self::InvalidMetadata(field) => write!(f, "metadata field is empty: {field}"),
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
    pub data: Vec<u8>,
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
            data: data.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpubBook {
    pub metadata: EpubMetadata,
    pub sections: Vec<EpubSection>,
    pub assets: Vec<EpubAsset>,
    pub cover_asset: Option<String>,
    vertical: bool,
    kindle: bool,
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
            vertical: true,
            kindle: false,
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

    pub fn with_kindle(mut self, kindle: bool) -> Self {
        self.kindle = kindle;
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
        validate_metadata(&self.metadata)?;
        for asset in &self.assets {
            validate_asset(asset)?;
        }
        if let Some(cover_asset) = &self.cover_asset
            && !self.assets.iter().any(|asset| &asset.path == cover_asset)
        {
            return Err(EpubError::InvalidMetadata("cover asset"));
        }

        let mut archive = ZipWriter::new(output);
        write_entry(
            &mut archive,
            "mimetype",
            MIMETYPE.as_bytes(),
            CompressionMethod::Stored,
        )?;
        write_entry(
            &mut archive,
            "META-INF/container.xml",
            CONTAINER_XML.as_bytes(),
            CompressionMethod::Deflated,
        )?;
        write_entry(
            &mut archive,
            "item/standard.opf",
            render_package(
                &self.metadata,
                &self.sections,
                &self.assets,
                self.cover_asset.as_deref(),
                self.vertical,
            )
            .as_bytes(),
            CompressionMethod::Deflated,
        )?;
        write_entry(
            &mut archive,
            "item/nav.xhtml",
            render_nav(&self.metadata, &self.sections).as_bytes(),
            CompressionMethod::Deflated,
        )?;
        if let Some(cover_asset) = &self.cover_asset {
            write_entry(
                &mut archive,
                "item/cover.xhtml",
                render_cover(&self.metadata, cover_asset, self.kindle).as_bytes(),
                CompressionMethod::Deflated,
            )?;
        }
        write_entry(
            &mut archive,
            "item/toc.ncx",
            render_ncx(&self.metadata, &self.sections).as_bytes(),
            CompressionMethod::Deflated,
        )?;
        write_entry(
            &mut archive,
            "item/style/book-style.css",
            BOOK_STYLE_CSS.as_bytes(),
            CompressionMethod::Deflated,
        )?;
        for (name, content) in [
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
            (
                "item/style/aozora.css",
                include_str!("../assets/aozora/template/item/style/aozora.css"),
            ),
            (
                "item/style/font.css",
                include_str!("../assets/aozora/template/item/style/font.css"),
            ),
            ("item/style/text.css", TEXT_CSS),
            (
                "item/style/fixed-layout-jp.css",
                include_str!("../assets/aozora/template/item/style/fixed-layout-jp.css"),
            ),
        ] {
            write_entry(
                &mut archive,
                name,
                content.as_bytes(),
                CompressionMethod::Deflated,
            )?;
        }
        let mut body_index = 0;
        for section in &self.sections {
            let path = if is_title_page(section) {
                "item/xhtml/title.xhtml".to_owned()
            } else {
                body_index += 1;
                format!("item/xhtml/{body_index:04}.xhtml")
            };
            write_entry(
                &mut archive,
                &path,
                render_section(
                    &self.metadata,
                    &section.body_fragment,
                    self.vertical,
                    self.kindle,
                )
                .as_bytes(),
                CompressionMethod::Deflated,
            )?;
        }
        for asset in &self.assets {
            let path = format!("item/{}", asset.path);
            write_entry(
                &mut archive,
                &path,
                &asset.data,
                CompressionMethod::Deflated,
            )?;
        }

        Ok(archive.finish()?)
    }
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

    use super::{EpubBook, EpubError, EpubMetadata};

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

        let mut package = String::new();
        archive
            .by_name("item/standard.opf")
            .unwrap()
            .read_to_string(&mut package)
            .unwrap();
        assert!(package.contains("href=\"xhtml/title.xhtml\""));
        assert!(package.contains("href=\"xhtml/0001.xhtml\""));
        assert!(package.contains("</manifest>\n  <spine"));

        let mut nav = String::new();
        archive
            .by_name("item/nav.xhtml")
            .unwrap()
            .read_to_string(&mut nav)
            .unwrap();
        assert!(nav.contains(">タイトル</a>"));
        assert!(nav.contains(">第一章</a>"));
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
        assert!(section.contains(
            "<html xmlns=\"http://www.w3.org/1999/xhtml\" \
             xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"ja\" class=\"hltr\">"
        ));
        assert!(section.contains("<body class=\"p-image\">"));
        assert!(section.contains("<img class=\"fit\" src=\"../image/fig.png\""));
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
        assert!(middle.contains("<body class=\"p-middle\">"));
        assert!(!middle.contains("aozora-page-middle"));
    }
}
