use aozora_epub3_lite::{EpubBook, EpubMetadata, aozora_text_to_xhtml_sections, decode_input};
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
    let mut encoding = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--encoding" => {
                encoding = Some(args.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--encoding requires a label")
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

    let input = fs::read(&input_path)?;
    let input = decode_input(&input, encoding.as_deref())?;
    let sections = aozora_text_to_xhtml_sections(&input)?;
    let identifier = format!("urn:aozoraepub3-lite:{}", percent_encode(&title));
    let metadata = EpubMetadata::new(title, identifier);

    let output = File::create(output_path)?;
    EpubBook::from_sections(metadata, sections).write_to(output)?;
    Ok(())
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
    "Usage: AozoraEpub3_Lite <input.txt> <output.epub> [title] [--encoding utf-8|shift_jis]"
}
