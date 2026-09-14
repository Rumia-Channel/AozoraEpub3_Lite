//! 入力から EPUB を組み立てるための画像・表紙・セクション整形。
//!
//! Java 版 `AozoraEpub3` の 1 入力ぶんの処理 (`AozoraEpub3.convertFile` →
//! `Epub3Writer`) のうち、本文テキスト変換の後段にあたる部分を CLI から
//! 切り出したもの。ライブラリ利用側 (narou.rs など) が CLI と同じ絵の扱いを
//! 再現できるように公開している。
//!
//! 呼び出し順 (CLI と同じ):
//!
//! 1. `collect_assets` — 挿絵注記の正規化 (`［＃挿絵（img/i1.png）入る］` →
//!    `image/0001.png`) と EPUB に格納する資産の収集。戻り値の
//!    [`CollectedAsset::references`] / [`CollectedAsset::resolved`] /
//!    [`CollectedAsset::source`] で「本文中の参照名 ↔ EPUB パス ↔ 実ファイル名」を
//!    引ける (表紙だけ実ファイル名を参照するため、この対応が必要)。
//! 2. `decorate_image_tags` — 画像タグへ寸法・回転・float / 単ページクラスを付与。
//!    `rewrite_image_source` / `remove_missing_image_sources` は参照名の
//!    書き換えと未解決画像の除去。
//! 3. `remove_image_sources` — 表紙ページへ移動した挿絵を本文から除去。
//! 4. `reflow_image_sections` — 大きい単ページ画像のセクション分割
//!    (`split_image_page_sections`)。
//! 5. `build_title_page_markup` / `append_gaiji_assets` — 表題ページの
//!    マークアップと、実際に使った外字フォントの格納。
//!
//! 表紙の決定は [`is_auto_cover`] / [`is_same_name_cover`] / [`is_no_cover`]
//! (`-c` オプション) と、`AozoraConfig::cover_page` / `cover_page_toc`
//! (`CoverPage` / `CoverPageToc` INI) の組み合わせで行う。

use crate::{
    AozoraConfig, BookMeta, ChapterRecord, EpubAsset, EpubMetadata, Input, TextEntry,
    apply_alt_upright, escape_html, image_reference_occurrences, inline_to_xhtml,
};
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn append_gaiji_assets(
    assets: &mut Vec<EpubAsset>,
    config: &AozoraConfig,
    sections: &[String],
    title_markup: &str,
    creator_markup: Option<&str>,
    title_page_markup: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let mut font_entries = config.gaiji_fonts.iter().collect::<Vec<_>>();
    let mut markup = sections.join("\n");
    markup.push_str(title_markup);
    if let Some(creator_markup) = creator_markup {
        markup.push_str(creator_markup);
    }
    if let Some(title_page_markup) = title_page_markup {
        markup.push_str(title_page_markup);
    }
    font_entries.sort_by_key(|(class_name, _)| {
        markup
            .find(&format!("class=\"glyph {class_name}\""))
            .unwrap_or(usize::MAX)
    });
    for (class_name, path) in font_entries {
        let marker = format!("class=\"glyph {class_name}\"");
        let used = sections.iter().any(|section| section.contains(&marker))
            || title_markup.contains(&marker)
            || creator_markup.is_some_and(|markup| markup.contains(&marker))
            || title_page_markup.is_some_and(|markup| markup.contains(&marker));
        if !used {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        let epub_path = format!("gaiji/{file_name}");
        if assets.iter().any(|asset| asset.path == epub_path) {
            continue;
        }
        assets.push(EpubAsset::new(
            epub_path,
            "application/font-sfnt",
            fs::read(path)?,
        ));
    }
    Ok(())
}
/// Converts an image-only archive (CBZ) into an EPUB with one page per
/// image, the first image (name-sorted) as the cover.
pub fn svg_image_fragment(path: &str, dimensions: ImageDimensions) -> String {
    let width = dimensions.width.max(1);
    let height = dimensions.height.max(1);
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\"\n\
xmlns:xlink=\"http://www.w3.org/1999/xlink\"\n\
width=\"100%\" height=\"100%\" viewBox=\"0 0 {} {}\">\n\
<image width=\"{}\" height=\"{}\" xlink:href=\"../image/{}\"/>\n\
</svg>",
        width,
        height,
        width,
        height,
        escape_html(path),
    )
}
pub fn build_metadata(
    title: &str,
    creator: Option<&str>,
    publisher: Option<&str>,
    language: Option<&str>,
) -> EpubMetadata {
    let identifier = format!("urn:uuid:{}", java_name_uuid(title, creator.unwrap_or("")));
    let mut metadata = EpubMetadata::new(title, identifier);
    if let Some(creator) = creator {
        metadata = metadata.with_creator(creator);
    }
    if let Some(publisher) = publisher {
        metadata = metadata.with_publisher(publisher);
    }
    if let Some(language) = language {
        metadata = metadata.with_language(language);
    }
    metadata
}
pub fn java_name_uuid(title: &str, creator: &str) -> String {
    let mut digest = md5::compute(format!("{title}-{creator}").as_bytes()).0;
    digest[6] = (digest[6] & 0x0f) | 0x30;
    digest[8] = (digest[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        digest[0],
        digest[1],
        digest[2],
        digest[3],
        digest[4],
        digest[5],
        digest[6],
        digest[7],
        digest[8],
        digest[9],
        digest[10],
        digest[11],
        digest[12],
        digest[13],
        digest[14],
        digest[15],
    )
}
pub fn build_title_page_markup(
    input: &str,
    metadata: &BookMeta,
    config: &AozoraConfig,
    _vertical: bool,
) -> Option<String> {
    let title_start = metadata.title_line?;
    let creator_start = metadata
        .creator_line
        .or_else(|| metadata.title_end_line.map(|line| line + 1))
        .unwrap_or(title_start + 1);
    let title_lines = input
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            (index >= title_start && index < creator_start)
                .then_some(line.trim())
                .filter(|line| !line.is_empty())
        })
        .collect::<Vec<_>>();
    if title_lines.is_empty() {
        return None;
    }

    let mut markup = String::from("<div class=\"book-title start-2em\">\n");
    for (index, line) in title_lines.iter().enumerate() {
        let converted = inline_to_xhtml(line, config);
        let element = match index {
            0 => format!("\t<div class=\"title book-title-main\"><p>{converted}</p></div>"),
            1 => format!("\t<div class=\"orgtitle pt1\">{converted}</div>"),
            2 => format!("\t<div class=\"subtitle pt1\">{converted}</div>"),
            _ => format!("\t<div class=\"suborgtitle pt2\">{converted}</div>"),
        };
        markup.push_str(&element);
        markup.push('\n');
    }
    markup.push_str("</div>");

    if let Some(creator_start) = metadata.creator_line {
        let creator_end = metadata
            .title_end_line
            .unwrap_or(creator_start)
            .max(creator_start);
        for (index, line) in input
            .lines()
            .enumerate()
            .filter_map(|(line_index, line)| {
                (line_index >= creator_start && line_index <= creator_end)
                    .then_some(line.trim())
                    .filter(|line| !line.is_empty())
                    .map(|line| (line_index, line))
            })
            .enumerate()
        {
            let converted = inline_to_xhtml(line.1, config);
            let class = if index == 0 { "creator" } else { "subcreator" };
            markup.push_str(&format!(
                "\n\t<div class=\"{class} btm pb2 author\">{converted}</div>"
            ));
        }
    }
    Some(markup)
}
/// 解決済みの EPUB 資産1件と、そこを指す本文中の参照。
///
/// 「本文中の参照名 → EPUB 内パス → 読み出す実ファイル」の対応は次の通り:
///
/// - `references[i]`: 本文・挿絵注記に書かれた参照名 (`"img/i1.png"` など)。
/// - `resolved`: EPUB 内の格納名 (`"0001.png"`、`asset.path` は `image/0001.png`)。
/// - `source`: 実際に読み出すファイル名 / アーカイブ内エントリ。表紙 (`-c`) は
///   連番化されずファイル名のまま格納され得るため、`resolved` と一致しない
///   ことがある (`references` も空になる)。
pub struct CollectedAsset {
    pub asset: EpubAsset,
    /// Paths as referenced in the text (e.g. `"fig.png"`).
    pub references: Vec<String>,
    /// `references` と同順: 各参照が元の名前のまま解決できたか。
    /// Java は `getImageWidthRatio(srcFilePath)` を元の参照名で引き、
    /// 拡張子違い（例: `img/x.jpg` → 実体 `img/x.png`）では null となり
    /// ratio=0 → `fit` テンプレートになる。
    pub available: Vec<bool>,
    /// 実際に読み出すファイル名 (TXT 入力) / アーカイブ内エントリ (ZIP 系入力)。
    pub source: String,
    /// EPUB 内の格納名 (`"0001.png"`、`asset.path` は `image/0001.png`)。
    pub resolved: String,
    /// Pre-processing dimensions (header read only), used for layout and
    /// decoration so image bytes never need to stay resident.
    pub dimensions: Option<ImageDimensions>,
    /// Whether this image is used as the cover; cover images are processed
    /// with the cover flag when the bytes are read at write time.
    pub is_cover: bool,
    /// Java `imageInfo.rotateAngle`: 回転角 (度)。Java は単ページ画像と
    /// 本文中の挿絵で条件を分けて設定する。
    pub rotate: i32,
}
/// Collects EPUB assets for all image references in the text (plus the
/// cover image), resolving against the filesystem for TXT inputs or against
/// the archive for ZIP/TXTZ/CBZ inputs. Only dimensions are kept; the image
/// bytes are read again, one image at a time, when the EPUB is written via
/// [`EpubBook::write_to_with`]. Returns the assets and the EPUB asset path
/// of the cover, if any.
/// `body` が `Some` のときは、変換後の本文に残っている参照だけを集める
/// (Java `NoIllust` は挿絵を出力しないため、EPUB にも格納されない)。
pub fn collect_assets(
    input: &Input,
    entry: &TextEntry,
    text: &str,
    cover: Option<&str>,
    body: Option<&str>,
) -> Result<(Vec<CollectedAsset>, Option<String>), Box<dyn Error>> {
    let base = input.path().parent().unwrap_or_else(|| Path::new("."));
    let mut assets: Vec<CollectedAsset> = Vec::new();
    let mut cover_asset = None;
    let mut image_index = 0usize;

    for reference in image_reference_occurrences(text) {
        if let Some(body) = body
            && !body.contains(&format!("../image/{}", escape_html(&reference)))
        {
            continue;
        }
        image_index += 1;
        // 画像の解決はアーカイブ (ZIP/TXTZ/CBZ) と FileSource 入力では Input
        // 経由、素の TXT 入力では入力ファイルの隣のファイルシステム経由。
        let from_input = input.is_archive() || input.has_source();
        let original_available = if from_input {
            input.resolve_image_path(entry, &reference).is_some()
        } else {
            base.join(reference.replace('\\', "/")).is_file()
        };
        let source_path = if from_input {
            input.resolve_image_path(entry, &reference)
        } else {
            resolve_fs_image_path(base, &reference)?
        };
        let Some(source_path) = source_path else {
            continue;
        };
        if let Some(existing) = assets.iter_mut().find(|asset| asset.source == source_path) {
            existing.references.push(reference);
            existing.available.push(original_available);
            continue;
        }
        let extension = source_path
            .rsplit_once('.')
            .map(|(_, extension)| extension)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let media_type = media_type_for_extension(&extension).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported image type: {reference}"),
            )
        })?;
        // 寸法と表紙判定のためだけに1枚だけ読み、バイトは保持しない
        let data = if from_input {
            input.read_image(&source_path)?.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("image entry not found: {source_path}"),
                )
            })?
        } else {
            fs::read(base.join(source_path.replace('\\', "/")))?
        };
        let dimensions = image_dimensions(&data, media_type);
        let output_name = format!("{:04}.{}", image_index, extension.replace("jpeg", "jpg"));
        let epub_path = format!("image/{output_name}");
        let is_cover = is_auto_cover(cover)
            && cover_asset.is_none()
            && dimensions.is_some_and(|dimensions| dimensions.width > 64 && dimensions.height > 64);
        if is_cover {
            cover_asset = Some(epub_path.clone());
        }
        assets.push(CollectedAsset {
            asset: EpubAsset::lazy(epub_path, media_type),
            references: vec![reference],
            available: vec![original_available],
            source: source_path,
            resolved: output_name,
            dimensions,
            is_cover,
            rotate: 0,
        });
    }

    match cover {
        Some(value) if is_auto_cover(Some(value)) || is_no_cover(value) => {}
        Some(value) if is_same_name_cover(value) => {
            let Some((source, extension)) = same_name_image(input.path()) else {
                eprintln!(
                    "[WARN] cover image not found next to: {}",
                    input.path().display()
                );
                return Ok((assets, cover_asset));
            };
            let extension = extension.to_ascii_lowercase();
            let media_type = media_type_for_extension(&extension).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unsupported cover image type: {extension}"),
                )
            })?;
            let data = fs::read(&source)?;
            let dimensions = image_dimensions(&data, media_type);
            let output_name = format!(
                "{:04}.{}",
                image_index + 1,
                extension.replace("jpeg", "jpg")
            );
            let epub_path = format!("image/{output_name}");
            assets.push(CollectedAsset {
                asset: EpubAsset::lazy(epub_path.clone(), media_type),
                references: Vec::new(),
                available: Vec::new(),
                source: source.to_string_lossy().replace('\\', "/"),
                resolved: output_name,
                dimensions,
                is_cover: true,
                rotate: 0,
            });
            cover_asset = Some(epub_path);
        }
        Some(path) if !is_external_reference(path) => {
            let normalized = normalize_relative_path(path)?;
            let source = base.join(&normalized);
            if source.is_file() {
                let extension = source
                    .extension()
                    .and_then(OsStr::to_str)
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let media_type = media_type_for_extension(&extension).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("unsupported cover image type: {extension}"),
                    )
                })?;
                let data = fs::read(&source)?;
                let dimensions = image_dimensions(&data, media_type);
                let output_name = format!(
                    "{:04}.{}",
                    image_index + 1,
                    extension.replace("jpeg", "jpg")
                );
                let epub_path = format!("image/{output_name}");
                assets.push(CollectedAsset {
                    asset: EpubAsset::lazy(epub_path.clone(), media_type),
                    references: Vec::new(),
                    available: Vec::new(),
                    source: normalized,
                    resolved: output_name,
                    dimensions,
                    is_cover: true,
                    rotate: 0,
                });
                cover_asset = Some(epub_path);
            } else {
                eprintln!("[WARN] cover image file not found: {path}");
            }
        }
        Some(_) => {
            eprintln!("[WARN] external cover references are not supported: {cover:?}");
        }
        None => {}
    }
    Ok((assets, cover_asset))
}
/// Resolves an image reference against the filesystem, mirroring the
/// previous behavior: exact file, then same-stem candidates with a
/// supported extension. Returns the input-relative path without reading
/// any bytes.
pub fn resolve_fs_image_path(
    base: &Path,
    reference: &str,
) -> Result<Option<String>, Box<dyn Error>> {
    let source = match resolve_image_source(base, reference) {
        Ok((source, _extension)) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(Some(
        source
            .strip_prefix(base)
            .unwrap_or(source.as_path())
            .to_string_lossy()
            .replace('\\', "/"),
    ))
}
/// Replaces image-only paragraphs whose source could not be resolved with an
/// empty paragraph, preserving the Java converter's pagination placeholder.
pub fn remove_missing_image_sources(
    sections: &mut [String],
    references: &[String],
    resolved_references: &[String],
) {
    let missing = references
        .iter()
        .filter(|reference| !resolved_references.contains(reference))
        .map(|reference| format!("image/{reference}"))
        .collect::<Vec<_>>();
    remove_image_sources(sections, &missing);
}
/// 指定した EPUB 内パス (`image/0001.jpg`) を参照する `<img>` を本文から取り除く。
/// Java `Epub3Writer.getImageFilePath` が null を返す経路（画像未解決・表紙ページへ
/// 移動した挿絵）と同じ後始末をする。
pub fn remove_image_sources(sections: &mut [String], sources: &[String]) {
    for source in sources {
        let source = format!("src=\"../{}\"", escape_html(source));
        for section in sections.iter_mut() {
            let mut cursor = 0;
            while let Some(offset) = section[cursor..].find("<img") {
                let start = cursor + offset;
                let Some(end_offset) = section[start..].find("/>") else {
                    break;
                };
                let end = start + end_offset + 2;
                let tag = &section[start..end];
                if !tag.contains(source.as_str()) {
                    cursor = end;
                    continue;
                }
                let line_start = section[..start].rfind('\n').map_or(0, |index| index + 1);
                let line_end = section[end..]
                    .find('\n')
                    .map_or(section.len(), |index| end + index);
                let line = &section[line_start..line_end];
                if is_image_only_paragraph(line, tag) {
                    let remove_end = if section.as_bytes().get(line_end) == Some(&b'\n') {
                        line_end + 1
                    } else {
                        line_end
                    };
                    let remainder = format!("{}{}", &section[..line_start], &section[remove_end..]);
                    let replacement = if is_empty_paragraph_fragment(&remainder) {
                        ""
                    } else if section.as_bytes().get(line_end) == Some(&b'\n') {
                        "<p><br/></p>\n"
                    } else {
                        "<p><br/></p>"
                    };
                    section.replace_range(line_start..remove_end, replacement);
                    cursor = line_start + replacement.len();
                    continue;
                }
                section.replace_range(start..end, "");
                cursor = start;
            }
        }
    }
    for section in sections {
        remove_empty_image_wrappers(section);
    }
}
pub fn remove_empty_image_wrappers(fragment: &mut String) {
    // Java: 画像取得失敗で <img> を除去した後、画像だけを包んでいた
    // <span> ラッパーを除去する。二分アキ (<span class="half_em_space"></span>)
    // や zws (&#8203;) など、画像と無関係な空 span は消してはいけない。
    // 画像ラッパーは class なしの素の <span> のみ (parse_raw_image / 画像注記が
    // <span><img .../></span> を生成する)。
    let mut cursor = 0;
    while let Some(offset) = fragment[cursor..].find("</span>") {
        let close = cursor + offset;
        let Some(open) = fragment[..close].rfind("<span") else {
            cursor = close + "</span>".len();
            continue;
        };
        let Some(content_start) = fragment[open..close].find('>') else {
            cursor = close + "</span>".len();
            continue;
        };
        let content_start = open + content_start + 1;
        // class 属性付きの span (half_em_space 等) は画像ラッパーではない
        let is_plain_span = fragment[open..content_start]
            .trim_end_matches('>')
            .trim_end()
            == "<span";
        if is_plain_span && fragment[content_start..close].trim().is_empty() {
            fragment.replace_range(open..close + "</span>".len(), "");
            cursor = open;
        } else {
            cursor = close + "</span>".len();
        }
    }
}
pub fn is_image_only_paragraph(line: &str, image_tag: &str) -> bool {
    let Some(inner) = line
        .trim()
        .strip_prefix("<p>")
        .and_then(|value| value.strip_suffix("</p>"))
    else {
        return false;
    };
    let mut remainder = inner.replace(image_tag, "");
    loop {
        let trimmed = remainder.trim();
        if let Some(end) = trimmed.find('>')
            && trimmed.starts_with("<span")
        {
            remainder = trimmed[end + 1..].to_owned();
            continue;
        }
        if let Some(stripped) = trimmed.strip_suffix("</span>") {
            remainder = stripped.to_owned();
            continue;
        }
        return trimmed.is_empty();
    }
}
pub fn tag_attribute<'a>(tag: &'a str, attribute: &str) -> Option<&'a str> {
    let marker = format!("{attribute}=\"");
    let start = tag.find(&marker)? + marker.len();
    let end = tag[start..].find('"')?;
    Some(&tag[start..start + end])
}
/// Makes local EPUB fragment links self-contained by removing references to
/// missing external documents. Named anchors are emitted as XHTML `id`s by
/// the inline converter, so fragment links remain valid across sections.
pub fn is_external_reference(value: &str) -> bool {
    let value = value.trim();
    if value.starts_with("//") {
        return true;
    }
    let Some((scheme, _)) = value.split_once(':') else {
        return false;
    };
    !scheme.is_empty()
        && scheme.chars().enumerate().all(|(index, character)| {
            if index == 0 {
                character.is_ascii_alphabetic()
            } else {
                character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
            }
        })
}
pub fn is_auto_cover(value: Option<&str>) -> bool {
    value.is_some_and(|value| matches!(value.trim(), "0" | "先頭の挿絵" | "[先頭の挿絵]"))
}
pub fn is_same_name_cover(value: &str) -> bool {
    matches!(
        value.trim(),
        "1" | "入力ファイル名と同じ画像(png,jpg,webp)" | "[入力ファイル名と同じ画像(png,jpg,webp)]"
    )
}
pub fn is_no_cover(value: &str) -> bool {
    matches!(value.trim(), "" | "表紙無し" | "[表紙無し]")
}
/// Rewrites `<img src="../image/REF">` to the resolved asset path in all
/// sections (needed when an archive stores the image under the text entry's
/// parent directory).
pub fn rewrite_image_source(sections: &mut [String], reference: &str, resolved: &str) {
    let from = format!("src=\"../image/{}\"", escape_html(reference));
    let to = format!("src=\"../image/{}\"", escape_html(resolved));
    for section in sections.iter_mut() {
        if section.contains(&from) {
            *section = section.replace(&from, &to);
        }
    }
}
/// `-c 1`: the input file name with a supported image extension, searched
/// next to the input file (png, jpg, jpeg, webp in the Java case order).
pub fn same_name_image(input_path: &Path) -> Option<(PathBuf, String)> {
    let base = input_path.with_extension("");
    for extension in [
        "png", "jpg", "jpeg", "webp", "PNG", "JPG", "JPEG", "WEBP", "Png", "Jpg", "Jpeg", "Webp",
    ] {
        let candidate = base.with_extension(extension);
        if candidate.is_file() {
            return Some((candidate, extension.to_owned()));
        }
    }
    None
}
#[derive(Clone, Copy)]
pub struct ImageDimensions {
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImagePageType {
    Inline,
    Page,
}
impl ImagePageType {
    fn is_page(self) -> bool {
        matches!(self, Self::Page)
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImagePageFit {
    None,
}
pub fn image_setting_f32(config: &AozoraConfig, key: &str, default: f32) -> f32 {
    config
        .ini
        .get(key)
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(default)
}
pub fn image_setting_usize(config: &AozoraConfig, key: &str, default: usize) -> usize {
    config
        .ini
        .get(key)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}
pub fn image_setting_bool(config: &AozoraConfig, key: &str, default: bool) -> bool {
    config.ini.get_bool(key).unwrap_or(default)
}
/// Java `Epub3Writer.getImagePageType` (単ページ画像) と `writeArchiveImage`
/// (本文中の挿絵) の条件で回転角を決める。表紙は Java が常に 0 にする。
pub fn rotate_for_image(
    config: &AozoraConfig,
    dimensions: ImageDimensions,
    page_type: ImagePageType,
    archive_images: bool,
) -> i32 {
    let Some(angle) = image_rotation(config) else {
        return 0;
    };
    let display_width = image_setting_f32(config, "DispW", 600.0);
    let display_height = image_setting_f32(config, "DispH", 800.0);
    if display_width <= 0.0
        || display_height <= 0.0
        || dimensions.width == 0
        || dimensions.height == 0
    {
        return 0;
    }
    let scale = image_setting_f32(config, "ImageScale", 1.0);
    let scaled_width = dimensions.width as f32 * scale;
    let scaled_height = dimensions.height as f32 * scale;
    // 単ページ画像: 画面より横長で 110% 以上 / 画面より縦長で 110% 以上
    if page_type.is_page() {
        if scaled_width / scaled_height > display_width / display_height {
            return if display_width < display_height && scaled_width > scaled_height * 1.1 {
                angle
            } else {
                0
            };
        }
        return if display_width > display_height && scaled_width * 1.1 < scaled_height {
            angle
        } else {
            0
        };
    }
    // 本文中の挿絵: Java writeArchiveImage はアーカイブ入力のときだけ回転する
    if !archive_images {
        return 0;
    }
    let ratio = f64::from(dimensions.width) / f64::from(dimensions.height);
    let display = f64::from(display_width) / f64::from(display_height);
    if ratio >= display {
        if display_width < display_height && 1.0 / ratio < display {
            angle
        } else {
            0
        }
    } else if display_width > display_height && 1.0 / ratio > display {
        angle
    } else {
        0
    }
}
pub fn image_rotation(config: &AozoraConfig) -> Option<i32> {
    match config.ini.get("RotateImage").map(str::trim) {
        Some("1") => Some(90),
        Some("2") => Some(-90),
        _ => None,
    }
}
pub fn image_page_type(
    dimensions: ImageDimensions,
    config: &AozoraConfig,
    has_caption: bool,
    tag_level: usize,
) -> ImagePageType {
    let display_width = image_setting_f32(config, "DispW", 600.0);
    let display_height = image_setting_f32(config, "DispH", 800.0);
    let scale = image_setting_f32(config, "ImageScale", 1.0);
    let image_width = dimensions.width as f32 * scale;
    let image_height = dimensions.height as f32 * scale;
    let float_type = image_setting_usize(config, "ImageFloatType", 0);
    let float_width = image_setting_usize(config, "ImageFloatW", 0) as u32;
    let float_height = image_setting_usize(config, "ImageFloatH", 0) as u32;

    if float_type != 0
        && (dimensions.width >= 64 || dimensions.height >= 64)
        && dimensions.width <= float_width
        && dimensions.height <= float_height
    {
        return ImagePageType::Inline;
    }

    // Java AozoraEpub3.java の既定値 (CLI)
    let single_page_width = image_setting_usize(config, "SinglePageWidth", 600) as u32;
    let single_page_size_width = image_setting_usize(config, "SinglePageSizeW", 480) as u32;
    let single_page_size_height = image_setting_usize(config, "SinglePageSizeH", 640) as u32;
    let eligible = dimensions.width >= single_page_width
        || (dimensions.width >= single_page_size_width
            && dimensions.height >= single_page_size_height);
    if eligible && tag_level == 0 && !has_caption {
        let fit_image = image_setting_bool(config, "FitImage", false);
        let image_size_type = image_setting_usize(config, "ImageSizeType", 2);
        if image_width <= display_width && image_height < display_height {
            if !fit_image {
                return ImagePageType::Inline;
            }
        } else if image_size_type == 1 {
            return ImagePageType::Page;
        }
        return ImagePageType::Page;
    }

    ImagePageType::Inline
}
pub fn rotated_dimensions(
    dimensions: ImageDimensions,
    config: &AozoraConfig,
    page_type: ImagePageType,
) -> ImageDimensions {
    if page_type.is_page()
        && image_rotation(config).is_some()
        && should_rotate(
            dimensions,
            image_setting_f32(config, "DispW", 600.0),
            image_setting_f32(config, "DispH", 800.0),
        )
    {
        ImageDimensions {
            width: dimensions.height,
            height: dimensions.width,
        }
    } else {
        dimensions
    }
}
pub fn image_page_fit(
    dimensions: ImageDimensions,
    config: &AozoraConfig,
    has_caption: bool,
    page_type: ImagePageType,
) -> ImagePageFit {
    if !page_type.is_page() || has_caption {
        return ImagePageFit::None;
    }
    let dimensions = rotated_dimensions(dimensions, config, page_type);
    let display_width = image_setting_f32(config, "DispW", 600.0);
    let display_height = image_setting_f32(config, "DispH", 800.0);
    if display_width <= 0.0 || display_height <= 0.0 {
        return ImagePageFit::None;
    }
    let image_width =
        dimensions.width as f32 * image_setting_f32(config, "ImageScale", 1.0).max(0.0);
    let image_height =
        dimensions.height as f32 * image_setting_f32(config, "ImageScale", 1.0).max(0.0);
    if image_width <= display_width && image_height < display_height {
        return ImagePageFit::None;
    }
    // Java: ImageHeight はヘッダ出力後に設定されるため単ページ img に style は付かない
    let _ = image_setting_usize(config, "ImageSizeType", 2);
    ImagePageFit::None
}
pub fn image_float_type(
    dimensions: ImageDimensions,
    config: &AozoraConfig,
) -> Option<(usize, bool)> {
    let float_type = image_setting_usize(config, "ImageFloatType", 0);
    let float_width = image_setting_usize(config, "ImageFloatW", 0) as u32;
    let float_height = image_setting_usize(config, "ImageFloatH", 0) as u32;
    if float_type == 0
        || (dimensions.width < 64 && dimensions.height < 64)
        || dimensions.width > float_width
        || dimensions.height > float_height
    {
        return None;
    }
    let display_width = image_setting_f32(config, "DispW", 600.0);
    let scaled_width =
        dimensions.width as f32 * image_setting_f32(config, "ImageScale", 1.0).max(0.0);
    Some((float_type, scaled_width > display_width))
}
pub fn image_tag_in_line(line: &str) -> Option<&str> {
    let start = line.find("<img")?;
    let end = start + line[start..].find('>')? + 1;
    Some(&line[start..end])
}
pub fn image_asset_for_line<'a>(
    line: &str,
    assets: &'a [CollectedAsset],
) -> Option<(&'a CollectedAsset, bool)> {
    let tag = image_tag_in_line(line)?;
    let source = tag_attribute(tag, "src")?;
    let source = source.strip_prefix("../")?;
    let asset = assets.iter().find(|asset| asset.asset.path == source)?;
    let has_caption = line.contains("キャプション") || line.contains("caption");
    Some((asset, has_caption))
}
pub fn is_standalone_image_line(line: &str) -> bool {
    let trimmed = line.trim();
    let Some(inner) = trimmed
        .strip_prefix("<p>")
        .and_then(|value| value.strip_suffix("</p>"))
        .map(str::trim)
    else {
        return false;
    };
    let inner = inner
        .strip_prefix("<span>")
        .and_then(|value| value.strip_suffix("</span>"))
        .map(str::trim)
        .unwrap_or(inner);
    // 外字画像は Java では printImageChuki を通らず単ページ化も改ページもされない
    if inner.contains("class=\"gaiji") {
        return false;
    }
    inner.starts_with("<img") && inner.ends_with("/>")
}
pub fn should_split_image_page(
    dimensions: ImageDimensions,
    config: &AozoraConfig,
    has_caption: bool,
) -> bool {
    image_page_type(dimensions, config, has_caption, 0).is_page()
        && !image_setting_bool(config, "ImageFloatPage", false)
}
/// 単ページ画像行の `<p>…</p>` ラッパーを除去して `<span>` のみにする。
/// Java は printImagePage → printLineBuffer(noBr=true) で p を付けずに出力する。
pub fn unwrap_image_paragraph(line: &str) -> String {
    match line
        .trim()
        .strip_prefix("<p>")
        .and_then(|value| value.strip_suffix("</p>"))
    {
        Some(inner) => format!("{inner}\n"),
        None => line.to_owned(),
    }
}
pub fn is_empty_paragraph_fragment(fragment: &str) -> bool {
    let fragment = fragment
        .replace('\r', "")
        .replace("<!-- aozora-page-chapter -->", "")
        .replace("<!-- aozora-page-middle -->", "")
        .replace("<!-- aozora-page-bottom -->", "")
        .replace("<!-- aozora-page-no-chapter -->", "");
    fragment
        .split("<p><br/></p>")
        .all(|part| part.trim().is_empty())
}
pub fn has_open_block_container(fragment: &str) -> bool {
    fragment.matches("<div").count() > fragment.matches("</div>").count()
}
pub fn is_page_marker_only_fragment(fragment: &str) -> bool {
    fragment
        .replace("<!-- aozora-page-chapter -->", "")
        .replace("<!-- aozora-page-middle -->", "")
        .replace("<!-- aozora-page-bottom -->", "")
        .replace("<!-- aozora-page-no-chapter -->", "")
        .trim()
        .is_empty()
}
pub fn split_image_page_sections(
    section: &str,
    assets: &[CollectedAsset],
    config: &AozoraConfig,
) -> Vec<String> {
    let mut output = Vec::new();
    let mut current = Vec::new();
    for line in section.split_inclusive('\n') {
        let prefix: String = current.concat();
        if is_standalone_image_line(line)
            && !has_open_block_container(&prefix)
            && let Some((asset, has_caption)) = image_asset_for_line(line, assets)
            && should_split_image_page(
                asset.dimensions.unwrap_or(ImageDimensions {
                    width: 0,
                    height: 0,
                }),
                config,
                has_caption,
            )
        {
            if !prefix.trim().is_empty() && !is_empty_paragraph_fragment(&prefix) {
                output.push(prefix);
                output.push(unwrap_image_paragraph(line));
            } else {
                // 直前が空段落だけなら単ページ画像セクションに持ち込まない。
                // Java は改ページ前の空行を出力しないため、ここで混ぜると
                // <body class="p-image"> の判定が崩れる。
                output.push(unwrap_image_paragraph(line));
            }
            current.clear();
        } else {
            current.push(line);
        }
    }
    let suffix = current.concat();
    if !suffix.trim().is_empty() {
        output.push(suffix);
    }
    if output.is_empty() {
        vec![section.to_owned()]
    } else {
        output
    }
}
pub fn standalone_image_page_type(
    section: &str,
    assets: &[CollectedAsset],
    config: &AozoraConfig,
) -> Option<ImagePageType> {
    if section.contains("aozora-page-") {
        return None;
    }
    if !is_standalone_image_line(section) {
        return None;
    }
    let (asset, has_caption) = image_asset_for_line(section, assets)?;
    let dimensions = asset.dimensions?;
    let page_type = image_page_type(dimensions, config, has_caption, 0);
    Some(
        if page_type.is_page() && image_setting_bool(config, "ImageFloatPage", false) {
            ImagePageType::Inline
        } else {
            page_type
        },
    )
}
pub fn reflow_image_sections(
    sections: &mut Vec<String>,
    chapters: &mut [ChapterRecord],
    assets: &[CollectedAsset],
    config: &AozoraConfig,
) {
    // 元セクション番号を添えて分割（後で章の section_index をリマップする）
    let mut split: Vec<(usize, String)> = Vec::new();
    for (index, section) in sections.drain(..).enumerate() {
        for piece in split_image_page_sections(&section, assets, config) {
            if !piece.trim().is_empty() && !is_page_marker_only_fragment(&piece) {
                split.push((index, piece));
            }
        }
    }
    let mut output: Vec<String> = Vec::with_capacity(split.len());
    let mut origins: Vec<usize> = Vec::with_capacity(split.len());
    let mut pending = String::new();
    let mut pending_origin = 0usize;
    for (origin, section) in split {
        if standalone_image_page_type(&section, assets, config) == Some(ImagePageType::Inline) {
            if pending.is_empty() && output.is_empty() {
                pending_origin = origin;
            }
            if !pending.is_empty() {
                pending.push_str(&section);
            } else if let Some(previous) = output.last_mut() {
                previous.push_str(&section);
            } else {
                pending = section;
            }
        } else {
            if !pending.is_empty() {
                output.push(std::mem::take(&mut pending));
                origins.push(pending_origin);
            }
            output.push(section);
            origins.push(origin);
        }
    }
    if !pending.is_empty() {
        if let Some(previous) = output.last_mut() {
            previous.push_str(&pending);
        } else {
            output.push(pending);
        }
    }
    *sections = output;
    // 分割されたセクションに応じて章の section_index を振り直す
    for chapter in chapters.iter_mut() {
        if let Some(new_index) = origins
            .iter()
            .position(|&origin| origin == chapter.section_index)
        {
            chapter.section_index = new_index;
        }
    }
}
pub fn image_wrapper_range(original: &str, image_start: usize) -> Option<(usize, usize)> {
    let start = original[..image_start].rfind("<span")?;
    let end = start + original[start..].find('>')? + 1;
    original[end..image_start]
        .trim()
        .is_empty()
        .then_some((start, end))
}
pub fn escape_image_alt(value: &str) -> String {
    let decoded = value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&times;", "×");
    escape_html(&decoded).replace('×', "&times;")
}
/// Java の画像タグ (`chuki_tag.txt` の 画像 / 画像幅 / 画像浮 等) は
/// width / height 属性を持たないため、ここでも出力しない。
pub fn render_image_tag(
    source: &str,
    alt: &str,
    class_name: Option<&str>,
    style: Option<&str>,
) -> String {
    let class = class_name
        .map(|value| format!(" class=\"{value}\""))
        .unwrap_or_default();
    let style = style
        .filter(|value| !value.is_empty())
        .map(|value| format!(" style=\"{value}\""))
        .unwrap_or_default();
    format!(
        "<img{class}{style} src=\"{}\" alt=\"{alt}\"/>",
        escape_html(source)
    )
}
pub fn decorate_image_tags(
    sections: &mut [String],
    assets: &mut [CollectedAsset],
    config: &AozoraConfig,
    // アーカイブ入力か。Java は writeArchiveImage でしか本文中の挿絵を
    // 回転させない (.txt 入力の画像はファイルシステムから無回転で書かれる)。
    archive_images: bool,
) {
    for section in sections.iter_mut() {
        let original = section.clone();
        let mut replacements: Vec<(usize, usize, String)> = Vec::new();
        let mut cursor = 0;
        while let Some(offset) = original[cursor..].find("<img") {
            let start = cursor + offset;
            let Some(end_offset) = original[start..].find('>') else {
                break;
            };
            let end = start + end_offset + 1;
            let tag = &original[start..end];
            let Some(source) = tag_attribute(tag, "src") else {
                cursor = end;
                continue;
            };
            let reference_name = source.strip_prefix("../image/").unwrap_or(source);
            let Some(collected) = assets
                .iter()
                .find(|collected| collected.references.iter().any(|r| r == reference_name))
            else {
                cursor = end;
                continue;
            };
            let reference_index = collected
                .references
                .iter()
                .position(|r| r == reference_name)
                .unwrap_or(0);
            let source_missing = !collected
                .available
                .get(reference_index)
                .copied()
                .unwrap_or(false);
            let Some(dimensions) = collected.dimensions else {
                cursor = end;
                continue;
            };
            let line_start = original[..start].rfind('\n').map_or(0, |index| index + 1);
            let line_end = original[end..]
                .find('\n')
                .map_or(original.len(), |index| end + index);
            let line = &original[line_start..line_end];
            let has_caption = line.contains("キャプション") || line.contains("caption");
            let has_open_block = has_open_block_container(&original[..start]);
            let page_type =
                image_page_type(dimensions, config, has_caption, usize::from(has_open_block));
            let rotate = if source_missing {
                0
            } else {
                rotate_for_image(config, dimensions, page_type, archive_images)
            };
            // Java は ImageInfo 単位で rotateAngle を持つ。同じ画像が複数回
            // 参照されていれば最後の判定が使われる。
            if let Some(asset) = assets
                .iter_mut()
                .find(|asset| asset.references.iter().any(|r| r == reference_name))
            {
                asset.rotate = rotate;
            }
            let ratio = if page_type.is_page() {
                0.0
            } else if source_missing {
                // Java: 元参照が無い/未知拡張子なら getImageInfo が null → ratio 0 → fit
                0.0
            } else {
                image_width_ratio(dimensions, config, has_caption, rotate)
            };
            // Java は `style="width:%s%%"` に double を渡すため Double.toString と同じ
            // 表記 (70 → "70.0") になる。Rust の `{:?}` が同じ書式。
            let ratio_text = format!("{ratio:?}");
            // Java: 行バッファ全体の事後変換で alt 内の正立文字も <span class="upr"> 化される
            let alt = apply_alt_upright(
                &escape_image_alt(tag_attribute(tag, "alt").unwrap_or_default().trim()),
                config,
            );
            let wrapper =
                image_wrapper_range(&original, start).filter(|(wrapper_start, wrapper_end)| {
                    &original[*wrapper_start..*wrapper_end] == "<span>"
                });

            if tag_attribute(tag, "class")
                .is_some_and(|class| class.split_whitespace().any(|name| name == "gaiji"))
            {
                // Java: getImageOrientation が -1 (行方向 64px 以下) のときは
                // switch に一致する case が無く、img タグを出力しない。
                // (画像ファイル自体は登録済みなので EPUB には格納される)
                match image_orientation(dimensions, config) {
                    -1 => replacements.push((start, end, String::new())),
                    orientation => {
                        let class_name = if orientation == 1 {
                            "gaiji-wide"
                        } else if orientation == 2 {
                            "gaiji-line"
                        } else {
                            "gaiji"
                        };
                        replacements.push((
                            start,
                            end,
                            render_image_tag(source, &alt, Some(class_name), None),
                        ));
                    }
                }
                cursor = end;
                continue;
            }

            let page_fit = image_page_fit(dimensions, config, has_caption, page_type);
            let _ = page_fit; // Java: ImageHeight はヘッダ出力後に設定されるため style 無し
            let float_type = image_float_type(dimensions, config);
            let page_float =
                page_type.is_page() && image_setting_bool(config, "ImageFloatPage", false);
            let block_float =
                !page_type.is_page() && image_setting_bool(config, "ImageFloatBlock", false);
            let (wrapper_replacement, image_class, image_style) = if page_float {
                (Some("<span class=\"img fpage\">".to_owned()), None, None)
            } else if page_type.is_page() {
                (Some("<span>".to_owned()), Some("fit"), None)
            } else if let Some((float_type, _)) = float_type {
                let class = if float_type == 1 { "ft" } else { "fb" };
                if ratio > 0.0 {
                    (
                        Some(format!(
                            "<span class=\"img {class}\" style=\"width:{ratio_text}%\">"
                        )),
                        None,
                        Some("width:100%".to_owned()),
                    )
                } else {
                    let class = if float_type == 1 {
                        "float-start m-end-1em"
                    } else {
                        "float-end m-start-1em"
                    };
                    (Some(format!("<span class=\"{class}\">")), Some("fit"), None)
                }
            } else if block_float {
                if ratio > 0.0 {
                    (
                        Some(format!(
                            "<span class=\"img fblk\" style=\"width:{ratio_text}%\">"
                        )),
                        None,
                        Some("width:100%".to_owned()),
                    )
                } else {
                    (
                        Some("<span class=\"img fblk\">".to_owned()),
                        Some("fit"),
                        None,
                    )
                }
            } else if ratio > 0.0 {
                (
                    Some(format!(
                        "<span class=\"img\" style=\"width:{ratio_text}%\">"
                    )),
                    None,
                    Some("width:100%".to_owned()),
                )
            } else {
                (None, Some("fit"), None)
            };

            if let Some((span_start, span_end)) = wrapper
                && let Some(wrapper_replacement) = wrapper_replacement
            {
                replacements.push((span_start, span_end, wrapper_replacement));
            }
            replacements.push((
                start,
                end,
                render_image_tag(source, &alt, image_class, image_style.as_deref()),
            ));
            cursor = end;
        }

        replacements.sort_unstable_by_key(|replacement| std::cmp::Reverse(replacement.0));
        for (start, end, replacement) in replacements {
            section.replace_range(start..end, &replacement);
        }
    }
}
pub fn image_orientation(dimensions: ImageDimensions, config: &AozoraConfig) -> i32 {
    if (config.vertical && dimensions.width <= 64) || (!config.vertical && dimensions.height <= 64)
    {
        return -1;
    }
    let dimensions = if image_rotation(config).is_some()
        && should_rotate(
            dimensions,
            image_setting_f32(config, "DispW", 600.0),
            image_setting_f32(config, "DispH", 800.0),
        ) {
        ImageDimensions {
            width: dimensions.height,
            height: dimensions.width,
        }
    } else {
        dimensions
    };
    if dimensions.width == dimensions.height {
        0
    } else if dimensions.width > dimensions.height {
        1
    } else {
        2
    }
}
pub fn image_width_ratio(
    dimensions: ImageDimensions,
    config: &AozoraConfig,
    has_caption: bool,
    rotate: i32,
) -> f64 {
    let scale = image_setting_f32(config, "ImageScale", 1.0);
    if scale == 0.0 {
        return 0.0;
    }
    if config.vertical && dimensions.width <= 64 || !config.vertical && dimensions.height <= 64 {
        return -1.0;
    }
    let display_width = image_setting_f32(config, "DispW", 600.0);
    let display_height = image_setting_f32(config, "DispH", 800.0);
    if display_width <= 0.0 || display_height <= 0.0 {
        return 0.0;
    }
    // Java getImageWidthRatio: 回転時は縦横を入れ替えて計算する
    let (image_width, image_height) = if rotate == 90 || rotate == -90 {
        (dimensions.height, dimensions.width)
    } else {
        (dimensions.width, dimensions.height)
    };
    // Java は double で (double)imgW/dispW*scale*100 の順に計算する
    let mut width_ratio = image_width as f64 / display_width as f64 * scale as f64 * 100.0;
    let height_ratio = image_height as f64 / display_height as f64 * scale as f64 * 100.0;
    if has_caption && height_ratio >= 90.0 {
        width_ratio *= 100.0 / height_ratio * 0.9;
    } else if height_ratio >= 100.0 {
        width_ratio *= 100.0 / height_ratio;
    }
    width_ratio.min(100.0)
}
pub fn should_rotate(dimensions: ImageDimensions, display_width: f32, display_height: f32) -> bool {
    if dimensions.width == 0 || dimensions.height == 0 {
        return false;
    }
    let image_ratio = dimensions.width as f32 / dimensions.height as f32;
    let display_ratio = display_width / display_height;
    if display_width < display_height {
        image_ratio > 1.1 && 1.0 / image_ratio < display_ratio
    } else {
        image_ratio < 1.0 / 1.1 && 1.0 / image_ratio > display_ratio
    }
}
pub fn image_dimensions(data: &[u8], media_type: &str) -> Option<ImageDimensions> {
    match media_type {
        "image/png" if data.len() >= 24 && data.starts_with(b"\x89PNG\r\n\x1a\n") => {
            Some(ImageDimensions {
                width: u32::from_be_bytes(data[16..20].try_into().ok()?),
                height: u32::from_be_bytes(data[20..24].try_into().ok()?),
            })
        }
        "image/gif"
            if data.len() >= 10 && (data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a")) =>
        {
            Some(ImageDimensions {
                width: u16::from_le_bytes(data[6..8].try_into().ok()?) as u32,
                height: u16::from_le_bytes(data[8..10].try_into().ok()?) as u32,
            })
        }
        "image/webp" => webp_dimensions(data),
        "image/jpeg" => jpeg_dimensions(data),
        _ => None,
    }
}
pub fn webp_dimensions(data: &[u8]) -> Option<ImageDimensions> {
    if data.len() < 30 || !data.starts_with(b"RIFF") || &data[8..12] != b"WEBP" {
        return None;
    }
    if &data[12..16] == b"VP8X" {
        return Some(ImageDimensions {
            width: 1 + u32::from_le_bytes([data[24], data[25], data[26], 0]),
            height: 1 + u32::from_le_bytes([data[27], data[28], data[29], 0]),
        });
    }
    None
}
pub fn jpeg_dimensions(data: &[u8]) -> Option<ImageDimensions> {
    if data.len() < 4 || data[..2] != [0xff, 0xd8] {
        return None;
    }
    let mut index = 2;
    while index + 9 < data.len() {
        while index < data.len() && data[index] != 0xff {
            index += 1;
        }
        while index < data.len() && data[index] == 0xff {
            index += 1;
        }
        let marker = *data.get(index)?;
        index += 1;
        if marker == 0xd8 || marker == 0xd9 {
            continue;
        }
        let length = u16::from_be_bytes([*data.get(index)?, *data.get(index + 1)?]) as usize;
        if length < 2 || index + length > data.len() {
            return None;
        }
        if matches!(
            marker,
            0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf
        ) {
            return Some(ImageDimensions {
                height: u16::from_be_bytes([data[index + 3], data[index + 4]]) as u32,
                width: u16::from_be_bytes([data[index + 5], data[index + 6]]) as u32,
            });
        }
        index += length;
    }
    None
}
/// Resolves a filesystem image by its path relative to the input text,
/// accepting a supported extension variant in the same directory.
pub fn resolve_image_source(base: &Path, image_path: &str) -> io::Result<(PathBuf, String)> {
    let normalized = image_path.replace('\\', "/");
    let requests = [base.join(&normalized)];

    for requested in requests {
        if requested.is_file() {
            let extension = requested
                .extension()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_owned();
            return Ok((requested, extension));
        }

        let parent = requested.parent().unwrap_or(base);
        let stem = requested
            .file_stem()
            .and_then(OsStr::to_str)
            .unwrap_or_default();
        let mut candidates = match fs::read_dir(parent) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .filter(|entry| entry.path().is_file())
                .filter_map(|entry| {
                    let path = entry.path();
                    let name = path.file_stem()?.to_str()?;
                    let extension = path.extension()?.to_str()?.to_owned();
                    (name.eq_ignore_ascii_case(stem)
                        && media_type_for_extension(&extension).is_some())
                    .then_some((path, extension))
                })
                .collect::<Vec<_>>(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        candidates.sort_by(|left, right| left.1.cmp(&right.1));
        if let Some((path, extension)) = candidates.into_iter().next() {
            return Ok((path, extension));
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("image file not found: {image_path}"),
    ))
}
pub fn normalize_relative_path(path: &str) -> Result<String, Box<dyn Error>> {
    let normalized = path.trim().replace('\\', "/");
    let mut parts = Vec::new();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "asset paths must stay below the input directory",
            )
            .into());
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "asset path is empty").into());
    }
    Ok(parts.join("/"))
}
/// Java `FileNameComparator`: 画像のみ ZIP の並び替え。`_` を `/` として扱い、
/// 漢数字と 上中下前後 を順序付けする (比較前に小文字化する)。
pub fn compare_image_names(left: &str, right: &str) -> std::cmp::Ordering {
    let left = left
        .to_lowercase()
        .chars()
        .map(ordering_char)
        .collect::<Vec<_>>();
    let right = right
        .to_lowercase()
        .chars()
        .map(ordering_char)
        .collect::<Vec<_>>();
    left.cmp(&right)
}
/// Java `FileNameComparator.replace`。
pub fn ordering_char(character: char) -> u32 {
    match character {
        '_' => '/' as u32,
        '一' => '一' as u32,
        '二' => '一' as u32 + 1,
        '三' => '一' as u32 + 2,
        '四' => '一' as u32 + 3,
        '五' => '一' as u32 + 4,
        '六' => '一' as u32 + 5,
        '七' => '一' as u32 + 6,
        '八' => '一' as u32 + 7,
        '九' => '一' as u32 + 8,
        '十' => '一' as u32 + 9,
        '上' => '上' as u32,
        '前' => '上' as u32 + 1,
        '中' => '上' as u32 + 2,
        '下' => '上' as u32 + 3,
        '後' => '上' as u32 + 4,
        _ => character as u32,
    }
}
pub fn media_type_for_extension(extension: &str) -> Option<&'static str> {
    match extension.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
