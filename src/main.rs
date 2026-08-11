use aozora_epub3_lite::{
    AozoraConfig, EpubAsset, EpubBook, EpubMetadata, aozora_text_to_xhtml_sections_with_config,
    decode_input, image_references,
};
use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io;
use std::path::Path;
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

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let input_path = match args.next().as_deref() {
        None | Some("-h") | Some("--help") => {
            print_usage();
            return Ok(());
        }
        Some(path) => path.to_owned(),
    };
    let output_path = args.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "an output EPUB path is required",
        )
    })?;
    let mut title = None;
    let mut creator = None;
    let mut language = None;
    let mut encoding = None;
    let mut cover = None;
    let mut config_dir = None;
    let mut preset = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--creator" => {
                creator = Some(args.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--creator requires a value")
                })?);
            }
            "--language" => {
                language = Some(args.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--language requires a value")
                })?);
            }
            "--encoding" => {
                encoding = Some(args.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--encoding requires a label")
                })?);
            }
            "--cover" => {
                cover = Some(args.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--cover requires a path")
                })?);
            }
            "--config-dir" => {
                config_dir = Some(args.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--config-dir requires a path")
                })?);
            }
            "--preset" => {
                preset = Some(args.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--preset requires a path")
                })?);
            }
            argument if argument.starts_with('-') => {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, usage()).into());
            }
            argument if title.is_none() => title = Some(argument.to_owned()),
            _ => return Err(io::Error::new(io::ErrorKind::InvalidInput, usage()).into()),
        }
    }
    let title = title.unwrap_or_else(|| title_from_path(Path::new(&input_path)));
    let config = AozoraConfig::load(
        config_dir.as_deref().map(Path::new),
        preset.as_deref().map(Path::new),
    )?;

    let input = fs::read(&input_path)?;
    let input = decode_input(&input, encoding.as_deref())?;
    let sections = aozora_text_to_xhtml_sections_with_config(&input, &config)?;
    let cover = cover.as_deref().map(normalize_relative_path).transpose()?;
    let assets = collect_assets(Path::new(&input_path), &input, cover.as_deref())?;
    let identifier = format!("urn:aozoraepub3-lite:{}", percent_encode(&title));
    let mut metadata = EpubMetadata::new(title, identifier);
    if let Some(creator) = creator {
        metadata = metadata.with_creator(creator);
    }
    if let Some(language) = language {
        metadata = metadata.with_language(language);
    }

    let output = File::create(output_path)?;
    let mut book = EpubBook::from_sections(metadata, sections).with_assets(assets);
    if let Some(cover) = cover {
        book = book.with_cover_asset(format!("image/{cover}"));
    }
    book.write_to(output)?;
    Ok(())
}
fn collect_assets(
    input_path: &Path,
    input: &str,
    cover: Option<&str>,
) -> Result<Vec<EpubAsset>, Box<dyn Error>> {
    let base = input_path.parent().unwrap_or_else(|| Path::new("."));
    let mut image_paths = image_references(input);
    if let Some(cover) = cover
        && !image_paths.iter().any(|path| path == cover)
    {
        image_paths.push(cover.to_owned());
    }
    image_paths
        .into_iter()
        .map(|image_path| {
            let source = base.join(&image_path);
            let data = fs::read(&source)?;
            let media_type = media_type_for_path(&image_path).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unsupported image type: {image_path}"),
                )
            })?;
            Ok(EpubAsset::new(
                format!("image/{image_path}"),
                media_type,
                data,
            ))
        })
        .collect()
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

fn media_type_for_path(path: &str) -> Option<&'static str> {
    match Path::new(path)
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        Some("svg") => Some("image/svg+xml"),
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

fn print_usage() {
    println!("{}", usage());
}

fn usage() -> &'static str {
    "Usage: AozoraEpub3_Lite <input.txt> <output.epub> [title] [--creator name] [--language lang] [--encoding utf-8|shift_jis] [--cover image] [--config-dir dir] [--preset file]"
}
