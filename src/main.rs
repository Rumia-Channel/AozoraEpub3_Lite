use aozora_epub3_lite::{
    AozoraConfig, BookMeta, EpubAsset, EpubBook, EpubMetadata, Input, TextEntry, TitleType,
    aozora_text_to_xhtml_sections_with_config, decode_text, detect_meta, escape_html,
    file_title_creator, image_references,
};
use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Command-line options in the Java reference layout (options before the
/// first positional; everything after is an input file).
#[derive(Clone, Debug, Default)]
struct CliOptions {
    inputs: Vec<String>,
    help: bool,
    ini: Option<String>,
    title_type: Option<usize>,
    use_file_name: bool,
    cover: Option<String>,
    out_ext: String,
    out_from_input_name: bool,
    dst: Option<String>,
    encoding: Option<String>,
    horizontal: Option<bool>,
    device: Option<String>,
    creator: Option<String>,
    language: Option<String>,
    config_dirs: Vec<String>,
    preset: Option<String>,
}

fn run() -> Result<(), Box<dyn Error>> {
    let options = match parse_args(env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("error: {message}");
            return Err(io::Error::new(io::ErrorKind::InvalidInput, usage()).into());
        }
    };
    if options.help || options.inputs.is_empty() {
        print_usage();
        return Ok(());
    }
    if let Some(dst) = options.dst.as_deref()
        && !Path::new(dst).is_dir()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("-d : dst path not exist. {dst}"),
        )
        .into());
    }
    let preset = options
        .preset
        .as_deref()
        .or(options.ini.as_deref())
        .map(Path::new);
    let config_dirs = options
        .config_dirs
        .iter()
        .map(|dir| Path::new(dir.as_str()))
        .collect::<Vec<_>>();
    let config = AozoraConfig::load_from_dirs(&config_dirs, preset)?;
    let vertical = options
        .horizontal
        .unwrap_or_else(|| config.ini.get_bool("Vertical").unwrap_or(true));
    let publisher_first = config.ini.get_bool("PubFirst").unwrap_or(false);
    let title_type = options
        .title_type
        .or_else(|| {
            config
                .ini
                .get("TitleType")
                .and_then(|value| value.parse::<usize>().ok())
        })
        .and_then(TitleType::from_index)
        .unwrap_or(TitleType::TitleAuthor);

    for input in &options.inputs {
        convert_input(
            input,
            &options,
            &config,
            title_type,
            publisher_first,
            vertical,
        )?;
    }
    Ok(())
}

/// Converts one input path (TXT or ZIP/TXTZ/CBZ). Archives produce one EPUB
/// per text entry, in archive order; image-only archives produce one EPUB
/// from all archive images.
fn convert_input(
    input_arg: &str,
    options: &CliOptions,
    config: &AozoraConfig,
    title_type: TitleType,
    publisher_first: bool,
    vertical: bool,
) -> Result<(), Box<dyn Error>> {
    let input_path = Path::new(input_arg);
    let input = Input::open(input_path)?;
    let dst = options.dst.as_deref().map(Path::new);

    if input.is_image_only() {
        return convert_image_only(&input, options, config, vertical);
    }

    let (file_title, file_creator) = file_title_creator(input.file_name().unwrap_or_default());
    let multi_entry = input.text_entries().len() > 1;
    for entry in input.text_entries() {
        let bytes = input.read_text(entry)?;
        let encoding_label = options
            .encoding
            .as_deref()
            .filter(|label| !label.eq_ignore_ascii_case("AUTO"));
        let text = decode_text(&bytes, encoding_label)?;

        let detected = detect_meta(&text, title_type, publisher_first);
        let body_text = remove_metadata_lines(&text, &detected);
        let publisher = detected.publisher;
        let (title, creator) = if options.use_file_name {
            (
                file_title.clone().or(detected.title),
                file_creator.clone().or(detected.creator),
            )
        } else {
            (
                detected.title.or(file_title.clone()),
                detected.creator.or(file_creator.clone()),
            )
        };
        let title = title.unwrap_or_else(|| title_from_path(input_path));
        let creator = options.creator.as_deref().or(creator.as_deref());

        let mut sections = aozora_text_to_xhtml_sections_with_config(&body_text, config)?;
        let (assets, cover) = collect_assets(&input, entry, &text, options.cover.as_deref())?;
        for collected in &assets {
            for reference in &collected.references {
                if collected.resolved != *reference {
                    rewrite_image_source(&mut sections, reference, &collected.resolved);
                }
            }
        }
        let resolved_references = assets
            .iter()
            .flat_map(|asset| asset.references.iter().map(String::as_str))
            .collect::<Vec<_>>();
        remove_missing_image_sources(
            &mut sections,
            &image_references(&text),
            &resolved_references,
        );
        let assets = assets
            .into_iter()
            .map(|item| item.asset)
            .collect::<Vec<_>>();
        decorate_image_tags(&mut sections, &assets, config);

        let metadata = build_metadata(
            &title,
            creator,
            publisher.as_deref(),
            options.language.as_deref(),
        );
        let suffix = multi_entry.then(|| entry_suffix(entry));
        let output = output_path(
            input_path,
            dst,
            Some(&title),
            creator,
            !options.out_from_input_name,
            &options.out_ext,
            suffix.as_deref(),
        );
        let mut book = EpubBook::from_sections(metadata, sections)
            .with_title_page_if(config.title_page_write)
            .with_vertical(vertical)
            .with_kindle(is_kindle(options))
            .with_assets(assets);
        if let Some(cover) = cover {
            book = book.with_cover_asset(cover);
        }
        let file = File::create(&output)?;
        book.write_to(file)?;
    }
    Ok(())
}
fn remove_metadata_lines(input: &str, metadata: &BookMeta) -> String {
    let Some(start) = metadata.meta_line_start else {
        return input.to_owned();
    };
    let Some(end) = metadata.title_end_line else {
        return input.to_owned();
    };
    let lines = input.lines().collect::<Vec<_>>();
    if start >= lines.len() || end >= lines.len() || start > end {
        return input.to_owned();
    }

    let mut remove_start = start;
    while remove_start > 0 && is_metadata_wrapper(lines[remove_start - 1]) {
        remove_start -= 1;
    }
    let mut remove_end = end;
    while remove_end + 1 < lines.len() && is_metadata_wrapper(lines[remove_end + 1]) {
        remove_end += 1;
    }

    lines
        .into_iter()
        .enumerate()
        .filter_map(|(index, line)| (index < remove_start || index > remove_end).then_some(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_metadata_wrapper(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("［＃ここから") || line.starts_with("［＃ここで")
}

/// Builds a disambiguating output-name suffix for archives with several
/// text entries (the Java reference would overwrite the same output file).
fn entry_suffix(entry: &TextEntry) -> String {
    let stem = entry
        .file_name()
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or_else(|| entry.file_name())
        .to_owned();
    if entry.parent.is_empty() {
        stem
    } else {
        format!("{}-{stem}", entry.parent.replace(['/', '\\'], "-"))
    }
}

/// Converts an image-only archive (CBZ) into an EPUB with one page per
/// image, the first image (name-sorted) as the cover.
fn convert_image_only(
    input: &Input,
    options: &CliOptions,
    config: &AozoraConfig,
    vertical: bool,
) -> Result<(), Box<dyn Error>> {
    let input_path = input.path();
    let mut sections = Vec::new();
    let mut assets = Vec::new();
    for (path, data) in input.images() {
        let extension = path
            .rsplit_once('.')
            .map(|(_, extension)| extension)
            .unwrap_or_default();
        let media_type = media_type_for_extension(extension).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported image type: {path}"),
            )
        })?;
        sections.push(format!(
            "<p><img class=\"fit\" src=\"../image/{}\" alt=\"\"/></p>",
            escape_html(path)
        ));
        assets.push(EpubAsset::new(
            format!("image/{path}"),
            media_type,
            data.clone(),
        ));
    }
    if assets.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("no images found in: {}", input_path.display()),
        )
        .into());
    }
    let title = title_from_path(input_path);
    let metadata = build_metadata(
        &title,
        options.creator.as_deref(),
        None,
        options.language.as_deref(),
    );
    decorate_image_tags(&mut sections, &assets, config);
    let cover = format!("image/{}", input.images().keys().next().unwrap());
    let output = output_path(
        input_path,
        options.dst.as_deref().map(Path::new),
        Some(&title),
        options.creator.as_deref(),
        !options.out_from_input_name,
        &options.out_ext,
        None,
    );
    let book = EpubBook::from_sections(metadata, sections)
        .with_vertical(vertical)
        .with_kindle(is_kindle(options))
        .with_assets(assets)
        .with_cover_asset(cover);
    let file = File::create(&output)?;
    book.write_to(file)?;
    Ok(())
}

fn build_metadata(
    title: &str,
    creator: Option<&str>,
    publisher: Option<&str>,
    language: Option<&str>,
) -> EpubMetadata {
    let identifier = format!("urn:aozoraepub3-lite:{}", percent_encode(title));
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

fn is_kindle(options: &CliOptions) -> bool {
    options
        .device
        .as_deref()
        .is_some_and(|device| device.eq_ignore_ascii_case("kindle"))
}

/// A resolved EPUB asset plus every text reference that points to it.
struct CollectedAsset {
    asset: EpubAsset,
    /// Paths as referenced in the text (e.g. `"fig.png"`).
    references: Vec<String>,
    /// Path the asset is stored at inside the EPUB (without the `image/`
    /// prefix); may include the archive parent directory.
    resolved: String,
}

/// Collects EPUB assets for all image references in the text (plus the
/// cover image), resolving against the filesystem for TXT inputs or against
/// the archive for ZIP/TXTZ/CBZ inputs. Returns the assets and the EPUB
/// asset path of the cover, if any.
fn collect_assets(
    input: &Input,
    entry: &TextEntry,
    text: &str,
    cover: Option<&str>,
) -> Result<(Vec<CollectedAsset>, Option<String>), Box<dyn Error>> {
    let base = input.path().parent().unwrap_or_else(|| Path::new("."));
    let mut assets: Vec<CollectedAsset> = Vec::new();
    let mut cover_asset = None;

    for reference in image_references(text) {
        let resolved = if input.is_archive() {
            input
                .resolve_image(entry, &reference)
                .map(|(path, data)| (path.to_owned(), data.clone()))
        } else {
            match resolve_fs_image(base, &reference) {
                Ok(Some(resolved)) => Some(resolved),
                Ok(None) => None,
                Err(error) => return Err(error),
            }
        };
        let Some((source_path, data)) = resolved else {
            continue;
        };
        let media_type = media_type_for_extension(
            source_path
                .rsplit_once('.')
                .map(|(_, extension)| extension)
                .unwrap_or_default(),
        )
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported image type: {reference}"),
            )
        })?;
        let epub_path = format!("image/{source_path}");
        if cover == Some("0")
            && cover_asset.is_none()
            && image_dimensions(&data, media_type)
                .is_some_and(|dimensions| dimensions.width > 64 && dimensions.height > 64)
        {
            cover_asset = Some(epub_path.clone());
        }
        if let Some(existing) = assets.iter_mut().find(|item| item.asset.path == epub_path) {
            existing.references.push(reference);
        } else {
            assets.push(CollectedAsset {
                asset: EpubAsset::new(epub_path, media_type, data),
                references: vec![reference],
                resolved: source_path,
            });
        }
    }

    match cover {
        Some("0") => {}
        Some("1") => {
            let Some((source, extension)) = same_name_image(input.path()) else {
                eprintln!(
                    "[WARN] cover image not found next to: {}",
                    input.path().display()
                );
                return Ok((assets, cover_asset));
            };
            let data = fs::read(&source)?;
            let media_type = media_type_for_extension(&extension).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unsupported cover image type: {extension}"),
                )
            })?;
            let epub_path = format!(
                "image/{}",
                source
                    .file_name()
                    .and_then(OsStr::to_str)
                    .unwrap_or("cover")
            );
            if !assets.iter().any(|item| item.asset.path == epub_path) {
                assets.push(CollectedAsset {
                    asset: EpubAsset::new(epub_path.clone(), media_type, data),
                    references: Vec::new(),
                    resolved: String::new(),
                });
            }
            cover_asset = Some(epub_path);
        }
        Some(path) if !path.starts_with("http://") && !path.starts_with("https://") => {
            let normalized = normalize_relative_path(path)?;
            let source = base.join(&normalized);
            if source.is_file() {
                let extension = source
                    .extension()
                    .and_then(OsStr::to_str)
                    .unwrap_or_default()
                    .to_owned();
                let media_type = media_type_for_extension(&extension).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("unsupported cover image type: {extension}"),
                    )
                })?;
                let data = fs::read(&source)?;
                let epub_path = format!("image/{normalized}");
                if !assets.iter().any(|item| item.asset.path == epub_path) {
                    assets.push(CollectedAsset {
                        asset: EpubAsset::new(epub_path.clone(), media_type, data),
                        references: Vec::new(),
                        resolved: String::new(),
                    });
                }
                cover_asset = Some(epub_path);
            } else {
                eprintln!("[WARN] cover image file not found: {path}");
            }
        }
        Some(_) => {
            eprintln!("[WARN] URL covers are not supported: {cover:?}");
        }
        None => {}
    }
    Ok((assets, cover_asset))
}

/// Resolves an image reference against the filesystem, mirroring the
/// previous behavior: exact file, then same-stem candidates with a
/// supported extension. Returns the input-relative path and bytes.
type ResolvedImage = (String, Vec<u8>);

fn resolve_fs_image(base: &Path, reference: &str) -> Result<Option<ResolvedImage>, Box<dyn Error>> {
    let source = match resolve_image_source(base, reference) {
        Ok((source, _extension)) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let data = fs::read(&source)?;
    let relative = source
        .strip_prefix(base)
        .unwrap_or(source.as_path())
        .to_string_lossy()
        .replace('\\', "/");
    Ok(Some((relative, data)))
}

/// Removes image tags whose source could not be resolved into an EPUB asset.
/// Keeping an unresolved `src` would make the generated EPUB invalid; the
/// already escaped `alt` text remains as a readable fallback.
fn remove_missing_image_sources(
    sections: &mut [String],
    references: &[String],
    resolved_references: &[&str],
) {
    for reference in references {
        if resolved_references.contains(&reference.as_str()) {
            continue;
        }
        let source = format!("../image/{}", escape_html(reference));
        for section in sections.iter_mut() {
            let mut cursor = 0;
            while let Some(offset) = section[cursor..].find("<img") {
                let start = cursor + offset;
                let Some(end_offset) = section[start..].find("/>") else {
                    break;
                };
                let end = start + end_offset + 2;
                let tag = &section[start..end];
                if tag.contains(&format!("src=\"{source}\"")) {
                    let replacement = tag_attribute(tag, "alt").unwrap_or_default().to_owned();
                    let replacement_len = replacement.len();
                    section.replace_range(start..end, &replacement);
                    cursor = start + replacement_len;
                } else {
                    cursor = end;
                }
            }
        }
    }
}

fn tag_attribute<'a>(tag: &'a str, attribute: &str) -> Option<&'a str> {
    let marker = format!("{attribute}=\"");
    let start = tag.find(&marker)? + marker.len();
    let end = tag[start..].find('"')?;
    Some(&tag[start..start + end])
}
/// Rewrites `<img src="../image/REF">` to the resolved asset path in all
/// sections (needed when an archive stores the image under the text entry's
/// parent directory).
fn rewrite_image_source(sections: &mut [String], reference: &str, resolved: &str) {
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
fn same_name_image(input_path: &Path) -> Option<(PathBuf, String)> {
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

/// Derives the output file path, mirroring `AozoraEpub3.getOutFile`:
/// `[creator] title.ext` in the destination directory (default: next to
/// the input) unless `-of` was given, in which case the input file name is
/// used. Sanitizes file-name-hostile characters and truncates to 250 chars.
fn output_path(
    input_path: &Path,
    dst: Option<&Path>,
    title: Option<&str>,
    creator: Option<&str>,
    auto_file_name: bool,
    out_ext: &str,
    suffix: Option<&str>,
) -> PathBuf {
    let extension = if out_ext.is_empty() { ".epub" } else { out_ext };
    let dst = dst.map(Path::to_path_buf).unwrap_or_else(|| {
        input_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_owned()
    });
    let stem = input_path
        .file_stem()
        .and_then(OsStr::to_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("book");
    let mut name = if auto_file_name
        && (creator.is_some_and(|value| !value.is_empty())
            || title.is_some_and(|value| !value.is_empty()))
    {
        let mut name = String::new();
        if let Some(creator) = creator.filter(|value| !value.is_empty()) {
            let cleaned = creator
                .chars()
                .filter(|character| {
                    !matches!(
                        character,
                        '\\' | '/' | ':' | '*' | '?' | '<' | '>' | '|' | '"' | '\t'
                    )
                })
                .take(64)
                .collect::<String>();
            name.push('[');
            name.push_str(&cleaned);
            name.push_str("] ");
        }
        if let Some(title) = title {
            name.push_str(
                &title
                    .chars()
                    .filter(|character| {
                        !matches!(
                            character,
                            '\\' | '/' | ':' | '*' | '!' | '?' | '<' | '>' | '|' | '"' | '\t'
                        )
                    })
                    .collect::<String>(),
            );
        }
        name
    } else {
        stem.to_owned()
    };
    if let Some(suffix) = suffix {
        name.push_str(" (");
        name.push_str(suffix);
        name.push(')');
    }
    let mut full = format!("{}/{}", dst.display(), name);
    if full.chars().count() > 250 {
        full = full.chars().take(250).collect();
    }
    full.push_str(extension);
    PathBuf::from(full)
}

#[derive(Clone, Copy)]
struct ImageDimensions {
    width: u32,
    height: u32,
}

fn decorate_image_tags(sections: &mut [String], assets: &[EpubAsset], config: &AozoraConfig) {
    let display_width = config
        .ini
        .get("DispW")
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(658.0);
    let display_height = config
        .ini
        .get("DispH")
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(905.0);
    let rotation = match config.ini.get("RotateImage") {
        Some("1") => Some(90),
        Some("2") => Some(-90),
        _ => None,
    };

    for asset in assets {
        let Some(dimensions) = image_dimensions(&asset.data, &asset.media_type) else {
            continue;
        };
        let rotate = rotation.filter(|_| should_rotate(dimensions, display_width, display_height));
        let source = format!("src=\"../{}\"", escape_html(&asset.path));
        let attributes = match rotate {
            Some(angle) => format!(
                "width=\"{}\" height=\"{}\" style=\"transform: rotate({angle}deg); transform-origin: center;\" {source}",
                dimensions.width, dimensions.height
            ),
            None => format!(
                "width=\"{}\" height=\"{}\" {source}",
                dimensions.width, dimensions.height
            ),
        };
        for section in sections.iter_mut() {
            if section.contains(&source) {
                *section = section.replace(&source, &attributes);
            }
        }
    }
}

fn should_rotate(dimensions: ImageDimensions, display_width: f32, display_height: f32) -> bool {
    if dimensions.width == 0 || dimensions.height == 0 {
        return false;
    }
    let image_ratio = dimensions.width as f32 / dimensions.height as f32;
    let display_ratio = display_width / display_height;
    if display_width < display_height {
        image_ratio > 1.0 && 1.0 / image_ratio < display_ratio
    } else {
        image_ratio < 1.0 && 1.0 / image_ratio > display_ratio
    }
}

fn image_dimensions(data: &[u8], media_type: &str) -> Option<ImageDimensions> {
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

fn webp_dimensions(data: &[u8]) -> Option<ImageDimensions> {
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

fn jpeg_dimensions(data: &[u8]) -> Option<ImageDimensions> {
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

/// Resolves a filesystem image by exact path, then by supported extension.
/// Aozora test data conventionally keeps referenced images in an `img/`
/// sibling directory, so that directory is searched when the direct path is
/// absent.
fn resolve_image_source(base: &Path, image_path: &str) -> io::Result<(PathBuf, String)> {
    let normalized = image_path.replace('\\', "/");
    let mut requests = vec![base.join(&normalized)];
    if !normalized.starts_with("img/") {
        requests.push(base.join("img").join(&normalized));
    }

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

fn normalize_relative_path(path: &str) -> Result<String, Box<dyn Error>> {
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

fn media_type_for_extension(extension: &str) -> Option<&'static str> {
    match extension.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

fn title_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(OsStr::to_str)
        .filter(|title| !title.is_empty())
        .unwrap_or("book")
        .to_owned()
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(hex_digit(byte >> 4));
            encoded.push(hex_digit(byte & 0x0f));
        }
    }
    encoded
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'A' + value - 10) as char,
        _ => unreachable!(),
    }
}

/// Parses command-line arguments with Java-compatible semantics: option
/// parsing stops at the first positional argument, everything after is an
/// input file.
fn parse_args(args: impl IntoIterator<Item = String>) -> Result<CliOptions, String> {
    let mut options = CliOptions::default();
    let mut iter = args.into_iter();
    while let Some(argument) = iter.next() {
        if !argument.starts_with('-') || argument == "-" {
            options.inputs.push(argument);
            options.inputs.extend(iter);
            break;
        }
        let (name, inline_value) = match argument.strip_prefix("--") {
            Some(rest) => match rest.split_once('=') {
                Some((name, value)) => (name.to_owned(), Some(value.to_owned())),
                None => (rest.to_owned(), None),
            },
            None => (argument[1..].to_owned(), None),
        };
        match name.as_str() {
            "h" | "help" => options.help = true,
            "tf" => options.use_file_name = true,
            "of" => options.out_from_input_name = true,
            "hor" | "horizontal" => options.horizontal = Some(false),
            "vertical" => options.horizontal = Some(true),
            "t" => {
                let value = inline_value
                    .or_else(|| iter.next())
                    .ok_or_else(|| "-t requires a value".to_owned())?;
                let index = value
                    .parse::<usize>()
                    .map_err(|_| format!("-t expects an index 0..5, got: {value}"))?;
                if index > 5 {
                    return Err(format!("-t expects an index 0..5, got: {index}"));
                }
                options.title_type = Some(index);
            }
            "i" | "ini" => {
                options.ini = Some(
                    inline_value
                        .or_else(|| iter.next())
                        .ok_or_else(|| "-i requires a file".to_owned())?,
                );
            }
            "c" | "cover" => {
                options.cover = Some(
                    inline_value
                        .or_else(|| iter.next())
                        .ok_or_else(|| "--cover requires a value".to_owned())?,
                );
            }
            "ext" => {
                options.out_ext = inline_value
                    .or_else(|| iter.next())
                    .ok_or_else(|| "-ext requires a value".to_owned())?;
            }
            "d" | "dst" => {
                options.dst = Some(
                    inline_value
                        .or_else(|| iter.next())
                        .ok_or_else(|| "-d requires a path".to_owned())?,
                );
            }
            "enc" | "encoding" => {
                options.encoding = Some(
                    inline_value
                        .or_else(|| iter.next())
                        .ok_or_else(|| "-enc requires a label".to_owned())?,
                );
            }
            "device" => {
                options.device = Some(
                    inline_value
                        .or_else(|| iter.next())
                        .ok_or_else(|| "-device requires a value".to_owned())?,
                );
            }
            "creator" => {
                options.creator = Some(
                    inline_value
                        .or_else(|| iter.next())
                        .ok_or_else(|| "--creator requires a value".to_owned())?,
                );
            }
            "language" => {
                options.language = Some(
                    inline_value
                        .or_else(|| iter.next())
                        .ok_or_else(|| "--language requires a value".to_owned())?,
                );
            }
            "preset" => {
                options.preset = Some(
                    inline_value
                        .or_else(|| iter.next())
                        .ok_or_else(|| "--preset requires a path".to_owned())?,
                );
            }
            "config-dir" => {
                options.config_dirs.push(
                    inline_value
                        .or_else(|| iter.next())
                        .ok_or_else(|| "--config-dir requires a path".to_owned())?,
                );
            }
            _ => return Err(format!("unknown option: {argument}")),
        }
    }
    Ok(options)
}

fn print_usage() {
    println!("{}", usage());
}
fn usage() -> &'static str {
    "Usage: AozoraEpub3_Lite [options] input_files(txt, zip, txtz, cbz)\n\
     options:\n\
     \x20 -h, --help             show usage\n\
     \x20 -i, --ini <file>       load settings from an ini file\n\
     \x20 -t <index>             title type: 0:title->author (default) 1:author->title\n\
     \x20                         2:title->author(subtitle first) 3:title only\n\
     \x20                         4:title+author only 5:none\n\
     \x20 -tf                    use the input file name for title/creator\n\
     \x20 -c, --cover <value>    0:first illustration 1:same-name image <file name>\n\
     \x20 -ext <extension>       output extension (default .epub)\n\
     \x20 -d, --dst <dir>        output directory\n\
     \x20 -enc <encoding>        input encoding: AUTO (default), MS932, UTF-8\n\
     \x20 -hor                   horizontal writing (default vertical)\n\
     \x20 -device <kindle>       target device\n\
     \x20 --creator <name>       override creator\n\
     \x20 --language <lang>      EPUB language (default ja)\n\
     \x20 --config-dir <dir>     configuration directory (repeatable)\n\
     \x20 --preset <file>        preset ini file\n\
     \x20 --vertical             vertical writing"
}

#[cfg(test)]
mod tests {
    use super::{
        AozoraConfig, CliOptions, EpubAsset, ImageDimensions, TitleType, decorate_image_tags,
        detect_meta, image_dimensions, output_path, parse_args, remove_metadata_lines,
        should_rotate,
    };
    use aozora_epub3_lite::IniSettings;
    use std::path::Path;

    fn parse(args: &[&str]) -> Result<CliOptions, String> {
        parse_args(args.iter().map(|value| value.to_string()))
    }

    #[test]
    fn parses_java_compatible_options() {
        let options = parse(&[
            "-t",
            "2",
            "-tf",
            "-c",
            "1",
            "-ext",
            ".kepub.epub",
            "-of",
            "-d",
            "out",
            "-enc",
            "MS932",
            "-hor",
            "-device",
            "kindle",
            "a.txt",
            "b.zip",
        ])
        .unwrap();
        assert_eq!(options.title_type, Some(2));
        assert!(options.use_file_name);
        assert_eq!(options.cover.as_deref(), Some("1"));
        assert_eq!(options.out_ext, ".kepub.epub");
        assert!(options.out_from_input_name);
        assert_eq!(options.dst.as_deref(), Some("out"));
        assert_eq!(options.encoding.as_deref(), Some("MS932"));
        assert_eq!(options.horizontal, Some(false));
        assert_eq!(options.device.as_deref(), Some("kindle"));
        assert_eq!(options.inputs, vec!["a.txt".to_owned(), "b.zip".to_owned()]);
    }

    #[test]
    fn retains_long_options_and_equals_values() {
        let options = parse(&[
            "--creator",
            "夏目漱石",
            "--language=ja",
            "--config-dir",
            "cfg",
            "--preset",
            "p.ini",
            "--vertical",
            "--encoding",
            "utf-8",
            "--cover=0",
            "book.txt",
        ])
        .unwrap();
        assert_eq!(options.creator.as_deref(), Some("夏目漱石"));
        assert_eq!(options.language.as_deref(), Some("ja"));
        assert_eq!(options.config_dirs, vec!["cfg".to_owned()]);
        assert_eq!(options.preset.as_deref(), Some("p.ini"));
        assert_eq!(options.horizontal, Some(true));
        assert_eq!(options.encoding.as_deref(), Some("utf-8"));
        assert_eq!(options.cover.as_deref(), Some("0"));
        assert_eq!(options.inputs, vec!["book.txt".to_owned()]);
    }

    #[test]
    fn treats_options_after_positionals_as_inputs() {
        let options = parse(&["a.txt", "-t", "2"]).unwrap();
        assert_eq!(
            options.inputs,
            vec!["a.txt".to_owned(), "-t".to_owned(), "2".to_owned()]
        );
        assert_eq!(options.title_type, None);
    }

    #[test]
    fn help_and_parse_errors() {
        assert!(parse(&["-h"]).unwrap().help);
        assert!(parse(&["--help"]).unwrap().help);
        assert!(parse(&["-t", "9", "x.txt"]).is_err());
        assert!(parse(&["-t", "abc", "x.txt"]).is_err());
        assert!(parse(&["-t"]).is_err());
        assert!(parse(&["--nope", "x.txt"]).is_err());
        assert!(parse(&["-i"]).is_err());
    }

    #[test]
    fn derives_output_file_names_like_java() {
        let input = Path::new("C:/books/吾輩は猫である.txt");
        let out = output_path(
            input,
            None,
            Some("吾輩は猫である"),
            Some("夏目漱石"),
            true,
            ".epub",
            None,
        );
        assert_eq!(out, Path::new("C:/books/[夏目漱石] 吾輩は猫である.epub"));

        // -of: the input file name is used
        let out = output_path(input, None, Some("表題"), None, false, ".epub", None);
        assert_eq!(out, Path::new("C:/books/吾輩は猫である.epub"));

        // no title/creator falls back to the input file name
        let out = output_path(input, None, None, None, true, ".epub", None);
        assert_eq!(out, Path::new("C:/books/吾輩は猫である.epub"));

        // destination directory overrides the input directory
        let out = output_path(
            input,
            Some(Path::new("C:/out")),
            Some("表題"),
            None,
            true,
            ".kepub.epub",
            None,
        );
        assert_eq!(out, Path::new("C:/out/表題.kepub.epub"));

        // multi-entry archive suffix
        let out = output_path(
            input,
            None,
            Some("表題"),
            None,
            true,
            ".epub",
            Some("novel-01"),
        );
        assert_eq!(out, Path::new("C:/books/表題 (novel-01).epub"));
    }

    #[test]
    fn removes_detected_title_lines_before_body_conversion() {
        let input = "表題\n著者名\n\n本文";
        let metadata = detect_meta(input, TitleType::TitleAuthor, false);
        assert_eq!(remove_metadata_lines(input, &metadata), "\n本文");
    }
    #[test]
    fn sanitizes_file_name_hostile_characters() {
        let input = Path::new("C:/books/in.txt");
        let out = output_path(
            input,
            None,
            Some("タイトル: 第1話!?"),
            Some("作\\者"),
            true,
            ".epub",
            None,
        );
        let name = out.file_name().unwrap().to_str().unwrap();
        assert_eq!(name, "[作者] タイトル 第1話.epub");
    }

    #[test]
    fn reads_common_image_dimensions_without_decoding_pixels() {
        let mut png = vec![0; 24];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[16..20].copy_from_slice(&640u32.to_be_bytes());
        png[20..24].copy_from_slice(&480u32.to_be_bytes());
        assert_eq!(
            image_dimensions(&png, "image/png").map(|value| (value.width, value.height)),
            Some((640, 480))
        );

        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&320u16.to_le_bytes());
        gif.extend_from_slice(&200u16.to_le_bytes());
        assert_eq!(
            image_dimensions(&gif, "image/gif").map(|value| (value.width, value.height)),
            Some((320, 200))
        );
    }

    #[test]
    fn rotates_only_images_with_the_wrong_orientation() {
        assert!(should_rotate(
            ImageDimensions {
                width: 1600,
                height: 900,
            },
            658.0,
            905.0
        ));
        assert!(!should_rotate(
            ImageDimensions {
                width: 900,
                height: 1600,
            },
            658.0,
            905.0
        ));
    }

    #[test]
    fn decorates_image_tags_with_dimensions_and_rotation() {
        let mut png = vec![0; 24];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[16..20].copy_from_slice(&1600u32.to_be_bytes());
        png[20..24].copy_from_slice(&900u32.to_be_bytes());
        let asset = EpubAsset::new("image/fig.png", "image/png", png);
        let config = AozoraConfig::from_ini(
            IniSettings::parse("DispW=658\nDispH=905\nRotateImage=1\n").unwrap(),
        );
        let mut sections =
            vec!["<p><img class=\"fit\" src=\"../image/fig.png\" alt=\"図\"/></p>".to_owned()];
        decorate_image_tags(&mut sections, &[asset], &config);
        assert!(sections[0].contains("width=\"1600\" height=\"900\""));
        assert!(sections[0].contains("transform: rotate(90deg)"));
    }
}
