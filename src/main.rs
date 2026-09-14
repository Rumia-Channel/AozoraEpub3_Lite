use aozora_epub3_lite::pipeline::{
    CollectedAsset, ImagePageType, append_gaiji_assets, build_metadata, build_title_page_markup,
    collect_assets, compare_image_names, decorate_image_tags, image_dimensions, is_auto_cover,
    media_type_for_extension, reflow_image_sections, remove_image_sources,
    remove_missing_image_sources, rewrite_image_source, rotate_for_image, svg_image_fragment,
};
use aozora_epub3_lite::{
    AozoraConfig, EpubAsset, EpubBook, Input, NavChapter, StyleSettings, TextEntry, TitleType,
    aozora_text_to_xhtml_sections_with_chapters, collect_image_alts, decode_text,
    detect_meta_with_gaiji, escape_html, file_title_creator, image::process as process_image,
    image_references, inline_to_xhtml, remove_metadata_lines, tcy_label,
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
    let mut options = match parse_args(env::args().skip(1)) {
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
    let preset = external_settings_path(&options)?;
    let uses_builtin_config = options.config_dirs.is_empty();
    let config_dirs = if uses_builtin_config {
        vec![default_config_dir()]
    } else {
        options.config_dirs.iter().map(PathBuf::from).collect()
    };
    let config_dir_refs = config_dirs.iter().map(PathBuf::as_path).collect::<Vec<_>>();
    let mut config = AozoraConfig::load_from_dirs(&config_dir_refs, preset)?;
    if preset.is_none() {
        // Java CLI parity: without -i/--preset the reference CLI runs with an
        // empty profile, so these flags stay off. An explicit INI always wins —
        // `--config-dir` only selects where the note assets live and must not
        // change conversion flags. replace.txt is likewise inert in the
        // reference distribution (it ships as replace_sample.txt), so the
        // bundled fallback directory no longer needs special-casing.
        config.auto_yoko = false;
        config.dakuten_type = 0;
        config.print_ivs_bmp = false;
        config.print_ivs_ssp = false;
        config.title_toc = false;
    }
    apply_ini_defaults(&mut options, &config);
    let vertical = options
        .horizontal
        .unwrap_or_else(|| config.ini.get_bool("Vertical").unwrap_or(true));
    // Java AozoraEpub3 は -hor を converter.vertical / bookInfo.vertical の両方に
    // 反映する。Lite の config.vertical は本文変換側の converter.vertical 相当。
    config.vertical = vertical;
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
            &mut config,
            title_type,
            publisher_first,
            vertical,
        )?;
    }
    Ok(())
}

/// 既定の設定ディレクトリ。実行ファイルの隣（本家 AozoraEpub3 と同じ配置:
/// `chuki_*.txt` や `gaiji/` が実行ファイルと同じディレクトリにある）を
/// 優先し、無ければ開発用の `assets/aozora` にフォールバックする。
/// 配布バイナリは実行ファイル隣の資産で動く。
fn default_config_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(directory) = exe.parent()
    {
        let has_aozora_assets =
            directory.join("chuki_tag.txt").is_file() || directory.join("gaiji").is_dir();
        if has_aozora_assets {
            return directory.to_path_buf();
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/aozora")
}

fn apply_ini_defaults(options: &mut CliOptions, config: &AozoraConfig) {
    if options.out_ext.is_empty()
        && let Some(extension) = config.ini.get("Ext")
    {
        options.out_ext = extension.to_owned();
    }
    if options.cover.is_none() {
        options.cover = config.ini.get("Cover").map(str::to_owned);
    }
}

fn external_settings_path(options: &CliOptions) -> Result<Option<&Path>, io::Error> {
    match (options.ini.as_deref(), options.preset.as_deref()) {
        (Some(_), Some(_)) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "-i/--ini and --preset cannot be used together",
        )),
        (Some(path), None) | (None, Some(path)) => Ok(Some(Path::new(path))),
        (None, None) => Ok(None),
    }
}

/// Converts one input path (TXT or ZIP/TXTZ/CBZ). Archives produce one EPUB
/// per text entry, in archive order; image-only archives produce one EPUB
/// from all archive images.
fn convert_input(
    input_arg: &str,
    options: &CliOptions,
    config: &mut AozoraConfig,
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

        // The reference pre-read consumes the first-chapter slot at the title
        // line; the body scan therefore starts without a pending chapter.
        let initial_add_section_chapter = detected.title_line.is_none();
        // Java は1入力1プロセスで imageAltMap を初期化するため、入力ごとに破棄する
        config.image_alt_map.clear();
        collect_image_alts(&body_text, config);
        let (mut sections, mut chapter_records) = aozora_text_to_xhtml_sections_with_chapters(
            &body_text,
            config,
            initial_add_section_chapter,
        )?;
        let title_page_selected =
            config.title_page_write && matches!(config.title_page_type, 1 | 2);
        let cover_setting = options.cover.as_deref();
        // NoIllust では本文から消えた挿絵を EPUB に格納しない
        let body_filter = config.no_illust.then(|| sections.concat());
        let (mut assets, cover) =
            collect_assets(&input, entry, &text, cover_setting, body_filter.as_deref())?;
        // 装飾を書き換え前に実行: 書き換え前の src（../image/{参照名}）から
        // 参照単位の available（拡張子違いの解決有無）を判定する。Java の
        // getImageWidthRatio(srcFilePath) は元の参照名で引けなければ ratio=0 → fit。
        decorate_image_tags(&mut sections, &mut assets, config, input.is_archive());
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
        // Java Epub3Writer.getImageFilePath: 表紙ページに移動した挿絵は
        // 本文から取り除く (先頭の挿絵を表紙に使う -c 0 のときのみ)。
        if config.cover_page
            && is_auto_cover(cover_setting)
            && let Some(cover) = cover.as_deref()
        {
            remove_image_sources(&mut sections, &[cover.to_owned()]);
        }
        reflow_image_sections(&mut sections, &mut chapter_records, &assets, config);

        let nav_chapters = chapter_records
            .into_iter()
            .map(|record| {
                // Java Epub3Writer: TocVertical のときだけ章名をエスケープ後に
                // convertTcyText へ通す。
                let (label, markup) = if config.toc_vertical {
                    (tcy_label(&escape_html(&record.label), config), true)
                } else {
                    (record.label, false)
                };
                let mut chapter = NavChapter::new(
                    label,
                    format!("xhtml/{:04}.xhtml", record.section_index + 1),
                )
                .with_level(record.level)
                .with_markup(markup);
                if let Some(anchor) = record.anchor {
                    chapter = chapter.with_anchor(anchor);
                }
                chapter
            })
            .collect::<Vec<_>>();
        let title_markup_input = if options.use_file_name {
            title.as_str()
        } else {
            detected_title_source.as_deref().unwrap_or(title.as_str())
        };
        // Java Epub3Writer: TITLE_HORIZONTAL の表題ページは converter.vertical=false で
        // 変換する（縦中横・正立タグが付かない）。TITLE_MIDDLE は本文と同じ縦書き設定。
        let title_line_config = if config.title_page_type == 2 {
            let mut horizontal = config.clone();
            horizontal.vertical = false;
            std::borrow::Cow::Owned(horizontal)
        } else {
            std::borrow::Cow::Borrowed(config)
        };
        let title_markup = inline_to_xhtml(title_markup_input, &title_line_config);
        let creator_markup = creator.map(|value| {
            let source = if options.use_file_name || options.creator.is_some() {
                value
            } else {
                detected_creator_source.as_deref().unwrap_or(value)
            };
            inline_to_xhtml(source, &title_line_config)
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
        let mut gaiji_assets = Vec::new();
        append_gaiji_assets(
            &mut gaiji_assets,
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
            .with_title_page_if(title_page_selected)
            .with_title_page_type(config.title_page_type)
            .with_vertical(vertical)
            .with_kindle(is_kindle(options))
            .with_toc_page(config.toc_page)
            .with_toc_vertical(config.toc_vertical)
            .with_cover_page(config.cover_page, config.cover_page_toc)
            .with_style(StyleSettings::from_ini(&config.ini))
            .with_toc_nest(config.nav_nest, config.ncx_nest)
            .with_title_toc(config.title_toc)
            .with_assets(
                assets
                    .iter()
                    .map(|collected| collected.asset.clone())
                    .chain(gaiji_assets),
            )
            .with_chapters(nav_chapters)
            .with_metadata_markup(title_markup, creator_markup);
        if let Some(title_page_markup) = title_page_markup {
            book = book.with_title_page_markup(title_page_markup);
        }
        if let Some(cover) = cover {
            // Java cover.vm の viewport / viewBox は表紙画像の元寸法を使う。
            let cover_dimensions = assets
                .iter()
                .find(|collected| collected.asset.path == cover)
                .and_then(|collected| collected.dimensions)
                .map(|dimensions| (dimensions.width, dimensions.height));
            book = book
                .with_cover_asset(cover)
                .with_cover_dimensions(cover_dimensions);
        }
        let file = File::create(&output)?;
        // 画像は書き出し時に1枚ずつ「読む → 処理 → 書く」して、全画像を
        // メモリに載せない（Java の Epub3ImageWriter と同様）。
        let base = input
            .path()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let provider = |epub_path: &str| -> Option<Vec<u8>> {
            let collected = assets.iter().find(|asset| asset.asset.path == epub_path)?;
            let data = if input.is_archive() {
                input.read_image(&collected.source).ok().flatten()?
            } else {
                fs::read(base.join(collected.source.replace('\\', "/"))).ok()?
            };
            process_image(
                &data,
                &collected.asset.media_type,
                &config.ini,
                collected.is_cover,
                collected.rotate,
            )
            .ok()
        };
        book.write_to_with(file, provider)?;
    }
    Ok(())
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

fn convert_image_only(
    input: &Input,
    options: &CliOptions,
    config: &AozoraConfig,
    vertical: bool,
) -> Result<(), Box<dyn Error>> {
    let input_path = input.path();
    let mut sections = Vec::new();
    let mut assets = Vec::new();
    // Java AozoraEpub3: imageOnly のときだけ FileNameComparator で並べ替える
    let mut image_paths = input.image_paths().to_vec();
    image_paths.sort_by(|left, right| compare_image_names(left, right));
    for (index, path) in image_paths.iter().enumerate() {
        let data = input.read_image(path)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("image entry not found: {path}"),
            )
        })?;
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
        // Java: 出力拡張子は常に .jpg (.jpeg の jpeg を jpg に置換)
        let output_name = format!("{:04}.{}", index + 1, extension.replace("jpeg", "jpg"));
        // Java Epub3ImageWriter: 画像のみの EPUB は全ページが単ページ扱いなので
        // 単ページ画像の条件で回転を決める。
        let source_dimensions = image_dimensions(&data, media_type);
        let rotate = source_dimensions
            .map(|dimensions| rotate_for_image(config, dimensions, ImagePageType::Page, true))
            .unwrap_or(0);
        let processed_data = process_image(&data, media_type, &config.ini, index == 0, rotate)?;
        let dimensions = image_dimensions(&processed_data, media_type);
        let fragment = dimensions
            .map(|dimensions| svg_image_fragment(&output_name, dimensions))
            .unwrap_or_else(|| {
                format!(
                    "<p><img class=\"fit\" src=\"../image/{}\" alt=\"\"/></p>",
                    escape_html(&output_name)
                )
            });
        sections.push(fragment);
        assets.push(CollectedAsset {
            asset: EpubAsset::new(format!("image/{output_name}"), media_type, processed_data),
            references: vec![output_name.clone()],
            available: vec![true],
            source: output_name.clone(),
            resolved: output_name,
            dimensions,
            is_cover: index == 0,
            rotate,
        });
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
    decorate_image_tags(&mut sections, &mut assets, config, false);
    let output = output_path(
        input_path,
        options.dst.as_deref().map(Path::new),
        Some(&title),
        options.creator.as_deref(),
        !options.out_from_input_name,
        &options.out_ext,
        None,
    );
    // Java: 画像ページは sectionIndex==1 または 5 の倍数で章（画像番号）を追加
    let chapters = sections
        .iter()
        .enumerate()
        .filter(|(index, _)| *index == 0 || (*index + 1) % 5 == 0)
        .map(|(index, _)| {
            NavChapter::new(
                (index + 1).to_string(),
                format!("xhtml/{:04}.xhtml", index + 1),
            )
        })
        .collect::<Vec<_>>();
    let book = EpubBook::from_sections(metadata, sections)
        .with_vertical(vertical)
        .with_kindle(is_kindle(options))
        .with_chapters(chapters)
        .with_assets(
            assets
                .into_iter()
                .map(|item| item.asset)
                .collect::<Vec<_>>(),
        );
    let file = File::create(&output)?;
    book.write_to(file)?;
    Ok(())
}

fn is_kindle(options: &CliOptions) -> bool {
    options
        .device
        .as_deref()
        .is_some_and(|device| device.eq_ignore_ascii_case("kindle"))
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
     \x20 -i, --ini <file>       load external INI settings\n\
     \x20 -t <index>             title type: 0:title->author (default) 1:author->title\n\
     \x20                         2:title->author(subtitle first) 3:title only\n\
     \x20                         4:title+author only 5:none\n\
     \x20 -tf                    use the input file name for title/creator\n\
     \x20 -c, --cover <value>    cover: 0:first illustration 1:same-name image <file name>\n\
     \x20                         uses INI Cover when omitted\n\
     \x20 -ext, --ext <extension> output extension (INI Ext or .epub by default)\n\
     \x20 -of, --of               use the input file name for output\n\
     \x20 -d, --dst <dir>         output directory\n\
     \x20 -enc, --encoding <name> input encoding: AUTO (default), MS932, UTF-8\n\
     \x20 -hor, --horizontal      horizontal writing (default vertical)\n\
     \x20 --vertical             vertical writing\n\
     \x20 -device, --device <kindle> enable Kindle-specific output handling\n\
     \x20 --language <lang>      EPUB language (default ja)\n\
     \x20 --creator <name>       override creator\n\
     \x20 --config-dir <dir>     configuration directory (repeatable)\n\
     \x20 --preset <file>        load external preset INI settings"
}

#[cfg(test)]
mod tests {
    use super::{
        AozoraConfig, CliOptions, TitleType, apply_ini_defaults, external_settings_path,
        output_path, parse_args, remove_metadata_lines, usage,
    };

    use aozora_epub3_lite::{IniSettings, decode_text, detect_meta};
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
    fn parses_long_aliases_for_all_java_cli_file_options() {
        let options = parse(&[
            "--ext=.kepub.epub",
            "--of",
            "--horizontal",
            "--device=kindle",
            "--encoding=MS932",
            "book.txt",
        ])
        .unwrap();
        assert_eq!(options.out_ext, ".kepub.epub");
        assert!(options.out_from_input_name);
        assert_eq!(options.horizontal, Some(false));
        assert_eq!(options.device.as_deref(), Some("kindle"));
        assert_eq!(options.encoding.as_deref(), Some("MS932"));
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
    fn rejects_ambiguous_ini_and_preset_selection() {
        let options = parse(&["-i", "base.ini", "--preset", "profile.ini", "book.txt"]).unwrap();
        let error = external_settings_path(&options).unwrap_err();
        assert_eq!(
            error.to_string(),
            "-i/--ini and --preset cannot be used together"
        );
    }

    #[test]
    fn applies_ini_defaults_without_overriding_cli_values() {
        let config =
            AozoraConfig::from_ini(IniSettings::parse("Ext=.kepub.epub\nCover=1\n").unwrap());
        let mut defaults = parse(&["book.txt"]).unwrap();
        apply_ini_defaults(&mut defaults, &config);
        assert_eq!(defaults.out_ext, ".kepub.epub");
        assert_eq!(defaults.cover.as_deref(), Some("1"));

        let mut explicit = parse(&["-ext", ".epub", "-c", "0", "book.txt"]).unwrap();
        apply_ini_defaults(&mut explicit, &config);
        assert_eq!(explicit.out_ext, ".epub");
        assert_eq!(explicit.cover.as_deref(), Some("0"));
    }

    #[test]
    fn documents_every_cli_option_in_help() {
        let help = usage();
        for option in [
            "-i, --ini",
            "-t <index>",
            "-tf",
            "-c, --cover",
            "-ext, --ext",
            "-of, --of",
            "-d, --dst",
            "-enc, --encoding",
            "-hor, --horizontal",
            "--vertical",
            "-device, --device <kindle>",
            "--language",
            "--creator",
            "--config-dir",
            "--preset",
        ] {
            assert!(help.contains(option), "missing help option: {option}");
        }
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
    fn keeps_separator_blank_before_hidden_comment_block() {
        // Java はタイトル行後の空行を `<p><br/></p>` として本文に出力する
        let input =
            "表題\n著者名\n\n-------------------------------------------------------\n注記\n本文";
        let metadata = detect_meta(input, TitleType::TitleAuthor, false);
        assert_eq!(
            remove_metadata_lines(input, &metadata),
            "\n-------------------------------------------------------\n注記\n本文"
        );
    }
    #[test]
    fn keeps_separator_blank_from_gaiji_title_fixture() {
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
            "\n-------------------------------------------------------\n注記"
        );
    }
    #[test]
    fn keeps_separator_blank_from_real_ruby_fixture() {
        // 直上の `keeps_separator_blank_from_gaiji_title_fixture` は同じ先頭行を
        // インライン文字列で持ち、メタデータ除去の完全一致を見る。こちらは
        // 同梱した Java 版 test_data/test_ruby.txt を丸ごと通す経路
        // (Shift_JIS の自動判別 → 全文のメタデータ検出 → 本文) を守るため、
        // 重複ではなく別の被覆。同梱物は tests/fixtures/ を参照。
        let bytes = std::fs::read("tests/fixtures/test_ruby.txt").unwrap();
        let text = decode_text(&bytes, None).unwrap();
        let config = AozoraConfig::default();
        let metadata = aozora_epub3_lite::detect_meta_with_gaiji(
            &text,
            TitleType::TitleAuthor,
            false,
            &config.gaiji,
        );
        // Java はタイトル行後の空行を本文に残し `<p><br/></p>` として出力する
        let body = remove_metadata_lines(&text, &metadata);
        assert!(body.starts_with('\n'), "{body:?}");
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
}
