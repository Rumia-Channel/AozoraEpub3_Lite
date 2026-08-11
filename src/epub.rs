use std::fmt;
use std::io::{self, Seek, Write};

use zip::write::{SimpleFileOptions, ZipWriter};
use zip::{CompressionMethod, result::ZipError};

const MIMETYPE: &str = "application/epub+zip";
const CONTAINER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="item/standard.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"#;
const BOOK_STYLE_CSS: &str = r#"@charset "UTF-8";

html.vrtl {
  writing-mode: vertical-rl;
  -webkit-writing-mode: vertical-rl;
  -epub-writing-mode: vertical-rl;
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
"#;

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
    pub language: String,
    pub identifier: String,
    pub modified: String,
}

impl EpubMetadata {
    pub fn new(title: impl Into<String>, identifier: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            creator: None,
            language: "ja".to_owned(),
            identifier: identifier.into(),
            modified: "1970-01-01T00:00:00Z".to_owned(),
        }
    }

    pub fn with_creator(mut self, creator: impl Into<String>) -> Self {
        self.creator = Some(creator.into());
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
                self.sections.len(),
                &self.assets,
                self.cover_asset.as_deref(),
            )
            .as_bytes(),
            CompressionMethod::Deflated,
        )?;
        write_entry(
            &mut archive,
            "item/nav.xhtml",
            render_nav(&self.metadata, self.sections.len()).as_bytes(),
            CompressionMethod::Deflated,
        )?;
        if let Some(cover_asset) = &self.cover_asset {
            write_entry(
                &mut archive,
                "item/cover.xhtml",
                render_cover(&self.metadata, cover_asset).as_bytes(),
                CompressionMethod::Deflated,
            )?;
        }
        write_entry(
            &mut archive,
            "item/toc.ncx",
            render_ncx(&self.metadata, self.sections.len()).as_bytes(),
            CompressionMethod::Deflated,
        )?;
        write_entry(
            &mut archive,
            "item/style/book-style.css",
            BOOK_STYLE_CSS.as_bytes(),
            CompressionMethod::Deflated,
        )?;
        for (index, section) in self.sections.iter().enumerate() {
            let path = format!("item/xhtml/{:04}.xhtml", index + 1);
            write_entry(
                &mut archive,
                &path,
                render_section(&self.metadata, &section.body_fragment).as_bytes(),
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

fn render_package(
    metadata: &EpubMetadata,
    section_count: usize,
    assets: &[EpubAsset],
    cover_asset: Option<&str>,
) -> String {
    let creator = metadata
        .creator
        .as_deref()
        .map(|value| {
            format!(
                "\n    <dc:creator id=\"creator\">{}</dc:creator>",
                xml_escape(value)
            )
        })
        .unwrap_or_default();
    let mut manifest_sections = String::new();
    let mut spine_sections = String::new();
    let mut manifest_assets = String::new();
    for index in 0..section_count {
        let number = index + 1;
        manifest_sections.push_str(&format!(
            "    <item id=\"section-{number:04}\" href=\"xhtml/{number:04}.xhtml\" media-type=\"application/xhtml+xml\"/>\n"
        ));
        spine_sections.push_str(&format!("    <itemref idref=\"section-{number:04}\"/>\n"));
    }
    for (index, asset) in assets.iter().enumerate() {
        let properties = if cover_asset == Some(asset.path.as_str()) {
            " properties=\"cover-image\""
        } else {
            ""
        };
        manifest_assets.push_str(&format!(
            "    <item id=\"asset-{index:04}\" href=\"{}\" media-type=\"{}\"{properties}/>\n",
            xml_escape(&asset.path),
            xml_escape(&asset.media_type),
        ));
    }
    let cover_manifest = if cover_asset.is_some() {
        "    <item id=\"cover\" href=\"cover.xhtml\" media-type=\"application/xhtml+xml\"/>\n"
    } else {
        ""
    };
    let cover_spine = if cover_asset.is_some() {
        "    <itemref idref=\"cover\"/>\n"
    } else {
        ""
    };

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>{}</dc:title>{}
    <dc:language>{}</dc:language>
    <dc:identifier id="pub-id">{}</dc:identifier>
    <meta property="dcterms:modified">{}</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="style" href="style/book-style.css" media-type="text/css"/>
{}{}{}    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
  </manifest>
  <spine page-progression-direction="rtl" toc="ncx">
{}{}  </spine>
</package>
"#,
        xml_escape(&metadata.title),
        creator,
        xml_escape(&metadata.language),
        xml_escape(&metadata.identifier),
        xml_escape(&metadata.modified),
        manifest_sections,
        manifest_assets,
        cover_manifest,
        cover_spine,
        spine_sections,
    )
}

fn render_nav(metadata: &EpubMetadata, section_count: usize) -> String {
    let mut nav_items = String::new();
    for index in 0..section_count {
        let number = index + 1;
        let label = if section_count == 1 {
            "本文".to_owned()
        } else {
            format!("本文 {number}")
        };
        nav_items.push_str(&format!(
            "      <li><a href=\"xhtml/{number:04}.xhtml\">{}</a></li>\n",
            xml_escape(&label)
        ));
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="{language}">
<head>
  <meta charset="UTF-8"/>
  <title>{title}</title>
</head>
<body>
  <nav epub:type="toc" id="toc">
    <h1>目次</h1>
    <ol>
{items}    </ol>
  </nav>
</body>
</html>
"#,
        language = xml_escape(&metadata.language),
        title = xml_escape(&metadata.title),
        items = nav_items,
    )
}

fn render_ncx(metadata: &EpubMetadata, section_count: usize) -> String {
    let mut nav_points = String::new();
    for index in 0..section_count {
        let number = index + 1;
        let label = if section_count == 1 {
            "本文".to_owned()
        } else {
            format!("本文 {number}")
        };
        nav_points.push_str(&format!(
            "    <navPoint id=\"navpoint-{number}\" playOrder=\"{number}\">\n\
      <navLabel><text>{}</text></navLabel>\n\
      <content src=\"xhtml/{number:04}.xhtml\"/>\n\
    </navPoint>\n",
            xml_escape(&label)
        ));
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE ncx PUBLIC "-//NISO//DTD ncx 2005-1//EN" "http://www.daisy.org/z3986/2005/ncx-2005-1.dtd">
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head>
    <meta name="dtb:uid" content="{identifier}"/>
  </head>
  <docTitle><text>{title}</text></docTitle>
  <navMap>
{nav_points}  </navMap>
</ncx>
"#,
        identifier = xml_escape(&metadata.identifier),
        title = xml_escape(&metadata.title),
        nav_points = nav_points,
    )
}

fn render_cover(metadata: &EpubMetadata, asset_path: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xml:lang="{language}">
<head>
  <meta charset="UTF-8"/>
  <title>{title}</title>
</head>
<body>
  <div class="cover">
    <img src="{asset_path}" alt="{title}"/>
  </div>
</body>
</html>
"#,
        language = xml_escape(&metadata.language),
        title = xml_escape(&metadata.title),
        asset_path = xml_escape(asset_path),
    )
}

fn render_section(metadata: &EpubMetadata, body_fragment: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xml:lang="{language}" class="vrtl">
<head>
  <meta charset="UTF-8"/>
  <title>{title}</title>
  <link rel="stylesheet" type="text/css" href="../style/book-style.css"/>
</head>
<body>
  <div class="main">
{body}
  </div>
</body>
</html>
"#,
        language = xml_escape(&metadata.language),
        title = xml_escape(&metadata.title),
        body = body_fragment,
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{EpubBook, EpubError, EpubMetadata};

    #[test]
    fn rejects_empty_title() {
        let book = EpubBook::new(EpubMetadata::new("", "urn:uuid:test"), "<p>本文</p>");
        let error = book.write_to(Cursor::new(Vec::new())).unwrap_err();
        assert!(matches!(error, EpubError::InvalidMetadata("title")));
    }
}
