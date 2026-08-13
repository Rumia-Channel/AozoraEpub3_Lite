use aozora_epub3_lite::{
    AozoraConfig, BookMeta, EpubAsset, EpubBook, EpubMetadata, Input, TextEntry, TitleType,
    aozora_text_to_xhtml_sections_with_config, decode_text, detect_meta_with_gaiji, escape_html,
    file_title_creator, image::process as process_image, image_reference_occurrences,
    image_references, inline_to_xhtml,
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
    let uses_builtin_config = options.config_dirs.is_empty();
    let config_dirs = if uses_builtin_config {
        vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/aozora")]
    } else {
        options.config_dirs.iter().map(PathBuf::from).collect()
    };
    let config_dir_refs = config_dirs.iter().map(PathBuf::as_path).collect::<Vec<_>>();
    let mut config = AozoraConfig::load_from_dirs(&config_dir_refs, preset)?;
    if uses_builtin_config {
        config.character_replacements.clear();
    }
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
        let preserve_utf8_bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF])
            && encoding_label.is_none_or(|label| label.eq_ignore_ascii_case("UTF-8"));
        let mut text = decode_text(&bytes, encoding_label)?;
        if preserve_utf8_bom {
            text.insert(0, '\u{feff}');
        }

        let detected = detect_meta_with_gaiji(&text, title_type, publisher_first, &config.gaiji);
        let detected_title_source = detected
            .title_line
            .and_then(|line| text.lines().nth(line))
            .map(str::trim)
            .map(str::to_owned);
        let detected_creator_source = detected
            .creator_line
            .and_then(|line| text.lines().nth(line))
            .map(str::trim)
            .map(str::to_owned);
        let publisher = detected.publisher.clone();
        let body_text = remove_metadata_lines(&text, &detected);
        let (title, creator) = if options.use_file_name {
            (
                file_title.clone().or(detected.title.clone()),
                file_creator.clone().or(detected.creator.clone()),
            )
        } else {
            (
                detected.title.clone().or(file_title.clone()),
                detected.creator.clone().or(file_creator.clone()),
            )
        };
        let title = title.unwrap_or_else(|| title_from_path(input_path));
        let creator = options.creator.as_deref().or(creator.as_deref());

        let mut sections = aozora_text_to_xhtml_sections_with_config(&body_text, config)?;
        let cover_setting = options.cover.as_deref();
        let (assets, cover) = collect_assets(&input, entry, &text, cover_setting, config)?;
        for collected in &assets {
            for reference in &collected.references {
                if collected.resolved != *reference {
                    rewrite_image_source(&mut sections, reference, &collected.resolved);
                }
            }
        }
        let resolved_references = assets
            .iter()
            .flat_map(|asset| asset.references.iter().cloned())
            .collect::<Vec<_>>();
        remove_missing_image_sources(
            &mut sections,
            &image_references(&text),
            &resolved_references,
        );
        let mut assets = assets
            .into_iter()
            .map(|item| item.asset)
            .collect::<Vec<_>>();
        sanitize_anchor_links(&mut sections);
        decorate_image_tags(&mut sections, &assets, config);
        reflow_image_sections(&mut sections, &assets, config);

        let title_markup_input = if options.use_file_name {
            title.as_str()
        } else {
            detected_title_source.as_deref().unwrap_or(title.as_str())
        };
        let title_markup = inline_to_xhtml(title_markup_input, config);
        let creator_markup = creator.map(|value| {
            let source = if options.use_file_name || options.creator.is_some() {
                value
            } else {
                detected_creator_source.as_deref().unwrap_or(value)
            };
            inline_to_xhtml(source, config)
        });
        let title_page_markup = if options.use_file_name {
            None
        } else {
            build_title_page_markup(&text, &detected, config, vertical)
        }
        .map(|markup| {
            let mut fragments = vec![markup];
            remove_missing_image_sources(
                &mut fragments,
                &image_references(&text),
                &resolved_references,
            );
            fragments.pop().unwrap_or_default()
        });
        append_gaiji_assets(
            &mut assets,
            config,
            &sections,
            &title_markup,
            creator_markup.as_deref(),
            title_page_markup.as_deref(),
        )?;
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
            .with_title_page_if(config.title_page_write && matches!(config.title_page_type, 1 | 2))
            .with_vertical(vertical)
            .with_kindle(is_kindle(options))
            .with_assets(assets)
            .with_metadata_markup(title_markup, creator_markup);
        if let Some(title_page_markup) = title_page_markup {
            book = book.with_title_page_markup(title_page_markup);
        }
        if let Some(cover) = cover {
            book = book.with_cover_asset(cover);
        }
        let file = File::create(&output)?;
        book.write_to(file)?;
    }
    Ok(())
}

fn append_gaiji_assets(
    assets: &mut Vec<EpubAsset>,
    config: &AozoraConfig,
    sections: &[String],
    title_markup: &str,
    creator_markup: Option<&str>,
    title_page_markup: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    for (class_name, path) in &config.gaiji_fonts {
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

    let mut retained = lines
        .into_iter()
        .enumerate()
        .filter_map(|(index, line)| (index < remove_start || index > remove_end).then_some(line))
        .collect::<Vec<_>>();
    if remove_start == 0
        && retained
            .iter()
            .find(|line| !line.trim().is_empty())
            .is_some_and(|line| line.starts_with("-----"))
    {
        while retained.first().is_some_and(|line| line.trim().is_empty()) {
            retained.remove(0);
        }
    }
    retained.join("\n")
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
fn svg_image_fragment(path: &str, dimensions: ImageDimensions) -> String {
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

fn convert_image_only(
    input: &Input,
    options: &CliOptions,
    config: &AozoraConfig,
    vertical: bool,
) -> Result<(), Box<dyn Error>> {
    let input_path = input.path();
    let mut sections = Vec::new();
    let mut assets = Vec::new();
    for (index, (path, data)) in input.images().iter().enumerate() {
        let extension = path
            .rsplit_once('.')
            .map(|(_, extension)| extension)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let media_type = media_type_for_extension(&extension).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported image type: {path}"),
            )
        })?;
        let output_name = format!("{:04}.{extension}", index + 1);
        let processed_data = process_image(data, media_type, &config.ini, index == 0)?;
        let fragment = image_dimensions(&processed_data, media_type)
            .map(|dimensions| svg_image_fragment(&output_name, dimensions))
            .unwrap_or_else(|| {
                format!(
                    "<p><img class=\"fit\" src=\"../image/{}\" alt=\"\"/></p>",
                    escape_html(&output_name)
                )
            });
        sections.push(fragment);
        assets.push(EpubAsset::new(
            format!("image/{output_name}"),
            media_type,
            processed_data,
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
        .with_assets(assets);
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

fn java_name_uuid(title: &str, creator: &str) -> String {
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

fn build_title_page_markup(
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
    /// Resolved filesystem or archive path used to deduplicate references.
    source: String,
    /// Path the asset is stored at inside the EPUB (without the `image/`
    /// prefix).
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
    config: &AozoraConfig,
) -> Result<(Vec<CollectedAsset>, Option<String>), Box<dyn Error>> {
    let base = input.path().parent().unwrap_or_else(|| Path::new("."));
    let mut assets: Vec<CollectedAsset> = Vec::new();
    let mut cover_asset = None;
    let mut image_index = 0usize;

    for reference in image_reference_occurrences(text) {
        image_index += 1;
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
        if let Some(existing) = assets.iter_mut().find(|asset| asset.source == source_path) {
            existing.references.push(reference);
            continue;
        }
        let output_name = format!("{:04}.{extension}", image_index);
        let epub_path = format!("image/{output_name}");
        let is_cover = is_auto_cover(cover)
            && cover_asset.is_none()
            && image_dimensions(&data, media_type)
                .is_some_and(|dimensions| dimensions.width > 64 && dimensions.height > 64);
        if is_cover {
            cover_asset = Some(epub_path.clone());
        }
        let data = process_image(&data, media_type, &config.ini, is_cover)?;
        assets.push(CollectedAsset {
            asset: EpubAsset::new(epub_path, media_type, data),
            references: vec![reference],
            source: source_path,
            resolved: output_name,
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
            let data = fs::read(&source)?;
            let extension = extension.to_ascii_lowercase();
            let media_type = media_type_for_extension(&extension).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unsupported cover image type: {extension}"),
                )
            })?;
            let data = process_image(&data, media_type, &config.ini, true)?;
            let output_name = format!("{:04}.{extension}", image_index + 1);
            let epub_path = format!("image/{output_name}");
            assets.push(CollectedAsset {
                asset: EpubAsset::new(epub_path.clone(), media_type, data),
                references: Vec::new(),
                source: source.to_string_lossy().replace('\\', "/"),
                resolved: output_name,
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
                let data = process_image(&fs::read(&source)?, media_type, &config.ini, true)?;
                let output_name = format!("{:04}.{extension}", image_index + 1);
                let epub_path = format!("image/{output_name}");
                assets.push(CollectedAsset {
                    asset: EpubAsset::new(epub_path.clone(), media_type, data),
                    references: Vec::new(),
                    source: normalized,
                    resolved: output_name,
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
    resolved_references: &[String],
) {
    for reference in references {
        if resolved_references.contains(reference) {
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
                if !tag.contains(&format!("src=\"{source}\"")) {
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
                    section.replace_range(line_start..remove_end, "");
                    cursor = line_start;
                    continue;
                }
                let replacement = tag_attribute(tag, "alt").unwrap_or_default().to_owned();
                let replacement_len = replacement.len();
                section.replace_range(start..end, &replacement);
                cursor = start + replacement_len;
            }
        }
    }
}

fn is_image_only_paragraph(line: &str, image_tag: &str) -> bool {
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

fn tag_attribute<'a>(tag: &'a str, attribute: &str) -> Option<&'a str> {
    let marker = format!("{attribute}=\"");
    let start = tag.find(&marker)? + marker.len();
    let end = tag[start..].find('"')?;
    Some(&tag[start..start + end])
}

/// Makes local EPUB fragment links self-contained by removing references to
/// missing external documents. Named anchors are emitted as XHTML `id`s by
/// the inline converter, so fragment links remain valid across sections.
fn is_external_reference(value: &str) -> bool {
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

fn is_auto_cover(value: Option<&str>) -> bool {
    value.is_some_and(|value| matches!(value.trim(), "0" | "先頭の挿絵" | "[先頭の挿絵]"))
}

fn is_same_name_cover(value: &str) -> bool {
    matches!(
        value.trim(),
        "1" | "入力ファイル名と同じ画像(png,jpg,webp)" | "[入力ファイル名と同じ画像(png,jpg,webp)]"
    )
}

fn is_no_cover(value: &str) -> bool {
    matches!(value.trim(), "" | "表紙無し" | "[表紙無し]")
}

fn sanitize_anchor_links(sections: &mut [String]) {
    let mut ids = std::collections::BTreeMap::new();
    for (section_index, section) in sections.iter().enumerate() {
        let mut cursor = 0;
        while let Some(offset) = section[cursor..].find(" id=\"") {
            let start = cursor + offset + 5;
            let Some(end) = section[start..].find('"') else {
                break;
            };
            ids.insert(section[start..start + end].to_owned(), section_index);
            cursor = start + end + 1;
        }
    }
    for (section_index, section) in sections.iter_mut().enumerate() {
        let mut cursor = 0;
        while let Some(offset) = section[cursor..].find("<a href=\"") {
            let start = cursor + offset;
            let href_start = start + "<a href=\"".len();
            let Some(href_end) = section[href_start..].find('"') else {
                break;
            };
            let href_end = href_start + href_end;
            let href = &section[href_start..href_end];
            let replacement = if let Some(fragment) = href.strip_prefix('#') {
                ids.get(fragment).map(|target_index| {
                    if *target_index == section_index {
                        format!("<a href=\"#{fragment}\">")
                    } else {
                        format!("<a href=\"{:04}.xhtml#{fragment}\">", target_index + 1)
                    }
                })
            } else if is_external_reference(href) {
                None
            } else {
                ids.get(href).map(|target_index| {
                    if *target_index == section_index {
                        format!("<a href=\"#{href}\">")
                    } else {
                        format!("<a href=\"{:04}.xhtml#{href}\">", target_index + 1)
                    }
                })
            };
            if let Some(replacement) = replacement {
                let tag_end = section[start..].find('>').map(|value| start + value + 1);
                if let Some(tag_end) = tag_end {
                    section.replace_range(start..tag_end, &replacement);
                    cursor = start + replacement.len();
                    continue;
                }
            } else {
                let tag_end = section[start..].find('>').map(|value| start + value + 1);
                if let Some(tag_end) = tag_end {
                    section.replace_range(start..tag_end, "<a>");
                    cursor = start + 3;
                    continue;
                }
            }
            cursor = href_end + 1;
        }
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ImagePageType {
    Inline,
    Page,
}

impl ImagePageType {
    fn is_page(self) -> bool {
        matches!(self, Self::Page)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ImagePageFit {
    None,
    Width,
    Height,
    HeightPercent(f32),
}

fn image_setting_f32(config: &AozoraConfig, key: &str, default: f32) -> f32 {
    config
        .ini
        .get(key)
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(default)
}

fn image_setting_usize(config: &AozoraConfig, key: &str, default: usize) -> usize {
    config
        .ini
        .get(key)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

fn image_setting_bool(config: &AozoraConfig, key: &str, default: bool) -> bool {
    config.ini.get_bool(key).unwrap_or(default)
}

fn image_rotation(config: &AozoraConfig) -> Option<i32> {
    match config.ini.get("RotateImage").map(str::trim) {
        Some("1") => Some(90),
        Some("2") => Some(-90),
        _ => None,
    }
}

fn image_page_type(
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

    let single_page_width = image_setting_usize(config, "SinglePageWidth", 550) as u32;
    let single_page_size_width = image_setting_usize(config, "SinglePageSizeW", 400) as u32;
    let single_page_size_height = image_setting_usize(config, "SinglePageSizeH", 600) as u32;
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
fn rotated_dimensions(
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

fn image_page_fit(
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
    match image_setting_usize(config, "ImageSizeType", 2) {
        1 => ImagePageFit::None,
        3 if image_width / image_height > display_width / display_height => ImagePageFit::Width,
        2 if image_width / image_height > display_width / display_height => {
            ImagePageFit::HeightPercent(
                (image_height / image_width * display_width / display_height * 100.0)
                    .clamp(0.0, 100.0),
            )
        }
        _ => ImagePageFit::Height,
    }
}

fn image_float_type(dimensions: ImageDimensions, config: &AozoraConfig) -> Option<(usize, bool)> {
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

fn image_tag_in_line(line: &str) -> Option<&str> {
    let start = line.find("<img")?;
    let end = start + line[start..].find('>')? + 1;
    Some(&line[start..end])
}

fn image_asset_for_line<'a>(line: &str, assets: &'a [EpubAsset]) -> Option<(&'a EpubAsset, bool)> {
    let tag = image_tag_in_line(line)?;
    let source = tag_attribute(tag, "src")?;
    let source = source.strip_prefix("../")?;
    let asset = assets.iter().find(|asset| asset.path == source)?;
    let has_caption = line.contains("キャプション") || line.contains("caption");
    Some((asset, has_caption))
}

fn is_standalone_image_line(line: &str) -> bool {
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
    inner.starts_with("<img") && inner.ends_with("/>")
}

fn should_split_image_page(
    dimensions: ImageDimensions,
    config: &AozoraConfig,
    has_caption: bool,
) -> bool {
    image_page_type(dimensions, config, has_caption, 0).is_page()
        && !image_setting_bool(config, "ImageFloatPage", false)
}

fn split_image_page_sections(
    section: &str,
    assets: &[EpubAsset],
    config: &AozoraConfig,
) -> Vec<String> {
    let mut output = Vec::new();
    let mut current = Vec::new();
    for line in section.split_inclusive('\n') {
        if is_standalone_image_line(line)
            && let Some((asset, has_caption)) = image_asset_for_line(line, assets)
            && should_split_image_page(
                image_dimensions(&asset.data, &asset.media_type).unwrap_or(ImageDimensions {
                    width: 0,
                    height: 0,
                }),
                config,
                has_caption,
            )
        {
            let prefix: String = current.concat();
            if !prefix.trim().is_empty() {
                output.push(prefix);
            }
            output.push(line.to_owned());
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

fn standalone_image_page_type(
    section: &str,
    assets: &[EpubAsset],
    config: &AozoraConfig,
) -> Option<ImagePageType> {
    if section.contains("aozora-page-") {
        return None;
    }
    if !is_standalone_image_line(section) {
        return None;
    }
    let (asset, has_caption) = image_asset_for_line(section, assets)?;
    let dimensions = image_dimensions(&asset.data, &asset.media_type)?;
    let page_type = image_page_type(dimensions, config, has_caption, 0);
    Some(
        if page_type.is_page() && image_setting_bool(config, "ImageFloatPage", false) {
            ImagePageType::Inline
        } else {
            page_type
        },
    )
}

fn reflow_image_sections(sections: &mut Vec<String>, assets: &[EpubAsset], config: &AozoraConfig) {
    let split = sections
        .drain(..)
        .flat_map(|section| split_image_page_sections(&section, assets, config))
        .filter(|section| !section.trim().is_empty())
        .collect::<Vec<_>>();
    let mut output: Vec<String> = Vec::with_capacity(split.len());
    let mut pending = String::new();
    for section in split {
        if standalone_image_page_type(&section, assets, config) == Some(ImagePageType::Inline) {
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
            }
            output.push(section);
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
}

fn image_wrapper_range(original: &str, image_start: usize) -> Option<(usize, usize)> {
    let start = original[..image_start].rfind("<span")?;
    let end = start + original[start..].find('>')? + 1;
    original[end..image_start]
        .trim()
        .is_empty()
        .then_some((start, end))
}

fn escape_image_alt(value: &str) -> String {
    let decoded = value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&times;", "×");
    escape_html(&decoded).replace('×', "&times;")
}

fn render_image_tag(
    source: &str,
    alt: &str,
    class_name: Option<&str>,
    style: Option<&str>,
    dimensions: Option<ImageDimensions>,
) -> String {
    let class = class_name
        .map(|value| format!(" class=\"{value}\""))
        .unwrap_or_default();
    let dimensions = dimensions
        .map(|value| format!(" width=\"{}\" height=\"{}\"", value.width, value.height))
        .unwrap_or_default();
    let style = style
        .filter(|value| !value.is_empty())
        .map(|value| format!(" style=\"{value}\""))
        .unwrap_or_default();
    format!(
        "<img{class}{dimensions}{style} src=\"{}\" alt=\"{alt}\"/>",
        escape_html(source)
    )
}

fn decorate_image_tags(sections: &mut [String], assets: &[EpubAsset], config: &AozoraConfig) {
    let display_width = image_setting_f32(config, "DispW", 600.0);
    let display_height = image_setting_f32(config, "DispH", 800.0);
    let rotation = image_rotation(config);

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
            let Some(asset) = source
                .strip_prefix("../")
                .and_then(|path| assets.iter().find(|asset| asset.path == path))
            else {
                cursor = end;
                continue;
            };
            let Some(dimensions) = image_dimensions(&asset.data, &asset.media_type) else {
                cursor = end;
                continue;
            };
            let line_start = original[..start].rfind('\n').map_or(0, |index| index + 1);
            let line_end = original[end..]
                .find('\n')
                .map_or(original.len(), |index| end + index);
            let line = &original[line_start..line_end];
            let has_caption = line.contains("キャプション") || line.contains("caption");
            let page_type = image_page_type(dimensions, config, has_caption, 0);
            let ratio = if page_type.is_page() {
                0.0
            } else {
                image_width_ratio(dimensions, config, has_caption)
            };
            let alt = escape_image_alt(tag_attribute(tag, "alt").unwrap_or_default().trim());
            let wrapper =
                image_wrapper_range(&original, start).filter(|(wrapper_start, wrapper_end)| {
                    &original[*wrapper_start..*wrapper_end] == "<span>"
                });

            if tag_attribute(tag, "class")
                .is_some_and(|class| class.split_whitespace().any(|name| name == "gaiji"))
            {
                let class_name = match image_orientation(dimensions, config) {
                    1 => "gaiji-wide",
                    2 => "gaiji-line",
                    _ => "gaiji",
                };
                replacements.push((
                    start,
                    end,
                    render_image_tag(source, &alt, Some(class_name), None, None),
                ));
                cursor = end;
                continue;
            }

            let page_fit = image_page_fit(dimensions, config, has_caption, page_type);
            let float_type = image_float_type(dimensions, config);
            let page_float =
                page_type.is_page() && image_setting_bool(config, "ImageFloatPage", false);
            let block_float =
                !page_type.is_page() && image_setting_bool(config, "ImageFloatBlock", false);
            let (wrapper_replacement, image_class, image_style, image_dimensions) = if page_float {
                let style = match page_fit {
                    ImagePageFit::Width => Some("width:100%;".to_owned()),
                    ImagePageFit::Height => Some("height:100%;".to_owned()),
                    ImagePageFit::HeightPercent(value) => {
                        Some(format!("height:{value:.1}%; min-height:{value:.1}%;"))
                    }
                    ImagePageFit::None => None,
                };
                (
                    Some("<span class=\"img fpage\">".to_owned()),
                    None,
                    style,
                    None,
                )
            } else if page_type.is_page() {
                let style = match page_fit {
                    ImagePageFit::HeightPercent(value) => {
                        Some(format!("height:{value:.1}%; min-height:{value:.1}%;"))
                    }
                    ImagePageFit::Width => Some("width:100%;".to_owned()),
                    ImagePageFit::Height | ImagePageFit::None => None,
                };
                let dimensions = rotation
                    .filter(|_| should_rotate(dimensions, display_width, display_height))
                    .map(|_| dimensions);
                (Some("<span>".to_owned()), Some("fit"), style, dimensions)
            } else if let Some((float_type, _)) = float_type {
                let class = if float_type == 1 { "ft" } else { "fb" };
                if ratio > 0.0 {
                    (
                        Some(format!(
                            "<span class=\"img {class}\" style=\"width:{ratio}%\">"
                        )),
                        None,
                        Some("width:100%".to_owned()),
                        None,
                    )
                } else {
                    let class = if float_type == 1 {
                        "float-start m-end-1em"
                    } else {
                        "float-end m-start-1em"
                    };
                    (
                        Some(format!("<span class=\"{class}\">")),
                        Some("fit"),
                        None,
                        None,
                    )
                }
            } else if block_float {
                if ratio > 0.0 {
                    (
                        Some(format!(
                            "<span class=\"img fblk\" style=\"width:{ratio}%\">"
                        )),
                        None,
                        Some("width:100%".to_owned()),
                        None,
                    )
                } else {
                    (
                        Some("<span class=\"img fblk\">".to_owned()),
                        Some("fit"),
                        None,
                        None,
                    )
                }
            } else if ratio > 0.0 {
                (
                    Some(format!("<span class=\"img\" style=\"width:{ratio}%\">")),
                    None,
                    Some("width:100%".to_owned()),
                    None,
                )
            } else {
                (None, Some("fit"), None, None)
            };
            let angle = rotation
                .filter(|_| page_type.is_page())
                .filter(|_| should_rotate(dimensions, display_width, display_height));
            let image_style = if let Some(angle) = angle {
                Some(format!(
                    "{}transform: rotate({angle}deg); transform-origin: center;",
                    image_style.as_deref().unwrap_or_default()
                ))
            } else {
                image_style
            };
            if let Some((span_start, span_end)) = wrapper
                && let Some(wrapper_replacement) = wrapper_replacement
            {
                replacements.push((span_start, span_end, wrapper_replacement));
            }
            replacements.push((
                start,
                end,
                render_image_tag(
                    source,
                    &alt,
                    image_class,
                    image_style.as_deref(),
                    image_dimensions,
                ),
            ));
            cursor = end;
        }

        replacements.sort_unstable_by_key(|replacement| std::cmp::Reverse(replacement.0));
        for (start, end, replacement) in replacements {
            section.replace_range(start..end, &replacement);
        }
    }
}

fn image_orientation(dimensions: ImageDimensions, config: &AozoraConfig) -> i32 {
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

fn image_width_ratio(dimensions: ImageDimensions, config: &AozoraConfig, has_caption: bool) -> f32 {
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
    let mut width_ratio = dimensions.width as f32 / display_width * scale * 100.0;
    let height_ratio = dimensions.height as f32 / display_height * scale * 100.0;
    if has_caption && height_ratio >= 90.0 {
        width_ratio *= 100.0 / height_ratio * 0.9;
    } else if height_ratio >= 100.0 {
        width_ratio *= 100.0 / height_ratio;
    }
    width_ratio.min(100.0)
}

fn should_rotate(dimensions: ImageDimensions, display_width: f32, display_height: f32) -> bool {
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

/// Resolves a filesystem image by its path relative to the input text,
/// accepting a supported extension variant in the same directory.
fn resolve_image_source(base: &Path, image_path: &str) -> io::Result<(PathBuf, String)> {
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
     \x20 --language <lang>        EPUB language (default ja)\n\
     \x20 --creator <name>       override creator\n\
     \x20 --config-dir <dir>     configuration directory (repeatable)\n\
     \x20 --preset <file>        preset ini file\n\
     \x20 --vertical             vertical writing"
}

#[cfg(test)]
mod tests {
    use super::{
        AozoraConfig, CliOptions, EpubAsset, ImageDimensions, ImagePageType, TitleType,
        decorate_image_tags, image_dimensions, image_page_type, java_name_uuid, output_path,
        parse_args, reflow_image_sections, remove_metadata_lines, remove_missing_image_sources,
        sanitize_anchor_links, should_rotate,
    };
    use aozora_epub3_lite::{IniSettings, decode_text, detect_meta};
    use std::path::Path;

    fn parse(args: &[&str]) -> Result<CliOptions, String> {
        parse_args(args.iter().map(|value| value.to_string()))
    }

    #[test]
    fn generates_java_compatible_name_uuid() {
        assert_eq!(
            java_name_uuid("横書き横組み", "テスト"),
            "27128c1c-ed73-341e-ae9d-9d052775453a"
        );
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
    fn drops_separator_blank_before_hidden_comment_block() {
        let input =
            "表題\n著者名\n\n-------------------------------------------------------\n注記\n本文";
        let metadata = detect_meta(input, TitleType::TitleAuthor, false);
        assert_eq!(
            remove_metadata_lines(input, &metadata),
            "-------------------------------------------------------\n注記\n本文"
        );
    }
    #[test]
    fn removes_separator_blank_from_gaiji_title_fixture() {
        let input = "｜ルビ※［＃米印］《るび》※［＃米印］※［＃始め二重山括弧］※［＃終わり二重山括弧］\n\
                     テスト《てすと》\n\
                     \n\
                     -------------------------------------------------------\n\
                     注記";
        let config = AozoraConfig::default();
        let metadata = aozora_epub3_lite::detect_meta_with_gaiji(
            input,
            TitleType::TitleAuthor,
            false,
            &config.gaiji,
        );
        assert_eq!(
            remove_metadata_lines(input, &metadata),
            "-------------------------------------------------------\n注記"
        );
    }
    #[test]
    fn removes_separator_blank_from_real_ruby_fixture() {
        let bytes = std::fs::read("sample/AozoraEpub3/test_data/test_ruby.txt").unwrap();
        let text = decode_text(&bytes, None).unwrap();
        let config = AozoraConfig::default();
        let metadata = aozora_epub3_lite::detect_meta_with_gaiji(
            &text,
            TitleType::TitleAuthor,
            false,
            &config.gaiji,
        );
        let body = remove_metadata_lines(&text, &metadata);
        assert!(!body.starts_with('\n'), "{body:?}");
        let sections =
            aozora_epub3_lite::aozora_text_to_xhtml_sections_with_config(&body, &config).unwrap();
        assert!(
            !sections[0].starts_with("    <p><br/></p>"),
            "{:?}",
            sections[0]
        );
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

    #[test]
    fn decorates_inline_images_with_java_width_ratio() {
        let mut png = vec![0; 24];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[16..20].copy_from_slice(&459u32.to_be_bytes());
        png[20..24].copy_from_slice(&350u32.to_be_bytes());
        let asset = EpubAsset::new("image/fig.png", "image/png", png);
        let config = AozoraConfig::from_ini(
            IniSettings::parse("DispW=600\nDispH=800\nSinglePageWidth=1000\nImageScale=1\n")
                .unwrap(),
        );
        let mut sections = vec![
            "<p><span><img class=\"fit\" src=\"../image/fig.png\" alt=\"図\"/></span></p>"
                .to_owned(),
        ];
        decorate_image_tags(&mut sections, &[asset], &config);
        assert!(sections[0].contains("<span class=\"img\" style=\"width:76.5%\">"));
        assert!(sections[0].contains("<img style=\"width:100%\""));
        assert!(!sections[0].contains("width=\"459\""));
    }

    #[test]
    fn applies_java_float_image_classes() {
        let mut png = vec![0; 24];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[16..20].copy_from_slice(&500u32.to_be_bytes());
        png[20..24].copy_from_slice(&300u32.to_be_bytes());
        let asset = EpubAsset::new("image/float.png", "image/png", png);
        let config = AozoraConfig::from_ini(
            IniSettings::parse(
                "DispW=600\nDispH=800\nImageFloatType=1\nImageFloatW=600\nImageFloatH=400\n",
            )
            .unwrap(),
        );
        let mut sections = vec![
            "<p><span><img class=\"fit\" src=\"../image/float.png\" alt=\"\"/></span></p>"
                .to_owned(),
        ];
        decorate_image_tags(&mut sections, &[asset], &config);
        assert!(sections[0].contains("<span class=\"img ft\""));
        assert!(sections[0].contains("style=\"width:83.33333%\""));
    }

    #[test]
    fn emits_height_fit_for_landscape_image_pages() {
        let mut png = vec![0; 24];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[16..20].copy_from_slice(&1200u32.to_be_bytes());
        png[20..24].copy_from_slice(&600u32.to_be_bytes());
        let asset = EpubAsset::new("image/page.png", "image/png", png);
        let config = AozoraConfig::from_ini(
            IniSettings::parse(
                "DispW=600\nDispH=800\nFitImage=1\nImageSizeType=2\nSinglePageWidth=550\n",
            )
            .unwrap(),
        );
        let mut sections = vec![
            "<p><span><img class=\"fit\" src=\"../image/page.png\" alt=\"\"/></span></p>"
                .to_owned(),
        ];
        decorate_image_tags(&mut sections, &[asset], &config);
        assert!(sections[0].contains("height:37.5%"));
        assert!(sections[0].contains("<img class=\"fit\""));
    }
    #[test]
    fn removes_missing_image_only_paragraphs() {
        let mut sections = vec![
            "<p><span><img class=\"fit\" src=\"../image/missing.png\" alt=\"未解決\"/></span></p>\n\
             <p>本文</p>"
                .to_owned(),
        ];
        remove_missing_image_sources(&mut sections, &["missing.png".to_owned()], &[]);
        assert_eq!(sections[0], "<p>本文</p>");
    }

    #[test]
    fn removes_unresolved_local_anchor_targets() {
        let mut sections = vec![
            r##"<p><a href="#missing">参照</a></p>"##.to_owned(),
            r##"<p><a id="present">本文</a><a href="#present">参照</a></p>"##.to_owned(),
        ];
        sanitize_anchor_links(&mut sections);
        assert_eq!(sections[0], "<p><a>参照</a></p>");
        assert!(sections[1].contains(r##"<a href="#present">参照</a>"##));
    }

    #[test]
    fn classifies_large_standalone_images_as_pages() {
        let config = AozoraConfig::from_ini(
            IniSettings::parse(
                "DispW=584\nDispH=754\nSinglePageWidth=550\nSinglePageSizeW=400\nSinglePageSizeH=600\n",
            )
            .unwrap(),
        );
        assert_eq!(
            image_page_type(
                ImageDimensions {
                    width: 1836,
                    height: 1400,
                },
                &config,
                false,
                0,
            ),
            ImagePageType::Page
        );
        assert_eq!(
            image_page_type(
                ImageDimensions {
                    width: 459,
                    height: 350,
                },
                &config,
                false,
                0,
            ),
            ImagePageType::Inline
        );
        assert_eq!(
            image_page_type(
                ImageDimensions {
                    width: 1836,
                    height: 1400,
                },
                &config,
                true,
                0,
            ),
            ImagePageType::Inline
        );
    }

    #[test]
    fn reflows_large_image_lines_into_page_sections() {
        let mut png = vec![0; 24];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[16..20].copy_from_slice(&1836u32.to_be_bytes());
        png[20..24].copy_from_slice(&1400u32.to_be_bytes());
        let asset = EpubAsset::new("image/large.png", "image/png", png);
        let config = AozoraConfig::from_ini(
            IniSettings::parse("DispW=584\nDispH=754\nSinglePageWidth=550\n").unwrap(),
        );
        let mut sections = vec![
            "<p>前</p>\n<p><span><img class=\"fit\" src=\"../image/large.png\" alt=\"\"/></span></p>\n<p>後</p>\n"
                .to_owned(),
        ];
        reflow_image_sections(&mut sections, &[asset], &config);
        assert_eq!(sections.len(), 3);
        assert!(sections[1].contains("<p><span><img"));
        assert!(sections[0].contains("<p>前</p>"));
        assert!(sections[2].contains("<p>後</p>"));
    }

    #[test]
    fn removes_external_anchor_targets() {
        let mut sections = vec![
            r##"<p><a href="https://example.com">外部</a></p>"##.to_owned(),
            r##"<p><a href="//cdn.example.com/book">CDN</a></p>"##.to_owned(),
        ];
        sanitize_anchor_links(&mut sections);
        assert_eq!(sections[0], "<p><a>外部</a></p>");
        assert_eq!(sections[1], "<p><a>CDN</a></p>");
    }
}
