//! Input layer for AozoraEpub3-compatible conversion.
//!
//! Supports plain filesystem TXT files and ZIP/TXTZ/CBZ archives without
//! extracting anything to a user-visible directory:
//!
//! * text entries are enumerated in archive order and read on demand,
//! * each text entry keeps its parent directory so image references in the
//!   text can be resolved against the archive,
//! * archive image entries are enumerated lazily and read one image at a
//!   time by normalized entry path, so images are never all resident in
//!   memory at once,
//! * CBZ / image-only archives are representable through [`Input::image_only`]
//!   even when no text entry exists.
//!
//! Charset auto-detection follows the Java reference: a UTF-8 BOM or valid
//! UTF-8 decodes as UTF-8, anything else as Shift_JIS/MS932. An explicit
//! encoding label always wins over auto-detection.

use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use encoding_rs::{Encoding, SHIFT_JIS, UTF_8};
use zip::ZipArchive;

/// Streaming file source for conversion: the full file list is known up
/// front (via [`FileSource::list`]), and each file's raw data is opened as a
/// stream on demand (via [`FileSource::open`]) so no archive is ever fully
/// resident in memory.
///
/// This is the building block for object-storage inputs such as Cloudflare
/// R2: list the object keys once, then stream each object (text or image)
/// when the converter needs it. Archive inputs (ZIP/TXTZ/CBZ) still use
/// [`Input::open`], which needs seeking.
pub trait FileSource: fmt::Debug + Send + Sync {
    /// Known file names (normalized paths, e.g. `"挿絵/i1186242.png"`).
    fn list(&self) -> &[String];
    /// Opens the raw data stream for one file. `Ok(None)` when the file is
    /// not present.
    fn open(&self, name: &str) -> Result<Option<Box<dyn Read + Send>>, InputError>;
}

/// Errors raised while opening or reading inputs.
#[derive(Debug)]
pub enum InputError {
    Io(io::Error),
    Zip(zip::result::ZipError),
    UnsupportedEncoding(String),
    Decode(String),
    UnsupportedInput(String),
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "input I/O error: {error}"),
            Self::Zip(error) => write!(f, "ZIP error: {error}"),
            Self::UnsupportedEncoding(encoding) => {
                write!(f, "unsupported input encoding: {encoding}")
            }
            Self::Decode(encoding) => write!(f, "input cannot be decoded as {encoding}"),
            Self::UnsupportedInput(path) => {
                write!(
                    f,
                    "unsupported input file (txt, zip, txtz, cbz only): {path}"
                )
            }
        }
    }
}

impl std::error::Error for InputError {}

impl From<io::Error> for InputError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<zip::result::ZipError> for InputError {
    fn from(error: zip::result::ZipError) -> Self {
        Self::Zip(error)
    }
}

/// One text file inside an opened input.
///
/// For a plain TXT input this is a single entry whose `name` is the file
/// name and whose `parent` is empty. For an archive it is one `.txt` entry;
/// `parent` is the entry's directory inside the archive ("" at the root).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextEntry {
    /// Normalized entry path inside the archive, e.g. `"novel/01.txt"`.
    pub name: String,
    /// Parent directory of the entry inside the archive, `""` at the root.
    pub parent: String,
    /// Archive entry index used to re-read the bytes; private to the module.
    entry_index: usize,
}

impl TextEntry {
    /// File name of the entry without any parent directory.
    pub fn file_name(&self) -> &str {
        self.name.rsplit('/').next().unwrap_or(&self.name)
    }
}

/// An opened input: a plain text file or an archive (zip/txtz/cbz).
///
/// Text entry bytes are read on demand via [`Input::read_text`]. Image
/// entries are enumerated lazily (names only) and their bytes are read one
/// image at a time via [`Input::read_image`], so a large archive never has
/// all images resident in memory at once.
#[derive(Clone, Debug)]
pub struct Input {
    path: PathBuf,
    archive: bool,
    entries: Vec<TextEntry>,
    image_paths: Vec<String>,
    image_indices: Vec<usize>,
    source: Option<Arc<dyn FileSource>>,
}

impl Input {
    /// Opens a TXT file or a ZIP/TXTZ/CBZ archive.
    ///
    /// The input kind is chosen from the file extension, falling back to
    /// sniffing the ZIP magic bytes for unknown extensions.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, InputError> {
        let path = path.as_ref();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match extension.as_str() {
            "txt" => Self::open_text(path),
            "zip" | "txtz" | "cbz" => Self::open_archive(path),
            _ => {
                let mut file = File::open(path)?;
                let mut magic = [0u8; 4];
                if file.read_exact(&mut magic).is_ok() && magic == *b"PK\x03\x04" {
                    Self::open_archive(path)
                } else {
                    Err(InputError::UnsupportedInput(path.display().to_string()))
                }
            }
        }
    }

    /// Builds an input from a streaming [`FileSource`] (e.g. object storage):
    /// text files are enumerated from [`FileSource::list`] and their bytes are
    /// streamed on demand. The returned input has an empty [`Input::path`];
    /// derive titles from the text itself or pass them explicitly.
    pub fn from_source(source: Arc<dyn FileSource>) -> Result<Self, InputError> {
        let mut entries = Vec::new();
        let mut image_paths = Vec::new();
        for (index, name) in source.list().iter().enumerate() {
            let Some(normalized) = normalize_entry_path(name) else {
                continue;
            };
            if normalized.to_ascii_lowercase().ends_with(".txt") {
                let parent = normalized
                    .rsplit_once('/')
                    .map(|(parent, _)| parent.to_owned())
                    .unwrap_or_default();
                entries.push(TextEntry {
                    name: normalized,
                    parent,
                    entry_index: index,
                });
            } else if image_media_type(&normalized).is_some() {
                image_paths.push(normalized);
            }
        }
        Ok(Self {
            path: PathBuf::new(),
            archive: false,
            entries,
            image_paths,
            image_indices: Vec::new(),
            source: Some(source),
        })
    }

    /// The input file path as given.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The input file name (used for filename-derived titles).
    pub fn file_name(&self) -> Option<&str> {
        self.path.file_name().and_then(|name| name.to_str())
    }

    /// True for archives without any text entry (image-only / CBZ).
    pub fn is_image_only(&self) -> bool {
        self.archive && self.entries.is_empty()
    }

    /// True when the input is a ZIP/TXTZ/CBZ archive rather than a plain
    /// text file.
    pub fn is_archive(&self) -> bool {
        self.archive
    }

    /// Text entries in archive order (a single entry for plain TXT files).
    pub fn text_entries(&self) -> &[TextEntry] {
        &self.entries
    }

    /// Normalized image entry paths in archive order (empty for plain TXT
    /// inputs).
    pub fn image_paths(&self) -> &[String] {
        &self.image_paths
    }

    /// Reads the raw bytes of a text entry.
    pub fn read_text(&self, entry: &TextEntry) -> Result<Vec<u8>, InputError> {
        if let Some(source) = &self.source {
            let mut reader = source
                .open(&entry.name)?
                .ok_or_else(|| InputError::UnsupportedInput(entry.name.clone()))?;
            let mut data = Vec::new();
            reader.read_to_end(&mut data)?;
            return Ok(data);
        }
        if !self.archive {
            return Ok(std::fs::read(&self.path)?);
        }
        let file = File::open(&self.path)?;
        let mut archive = ZipArchive::new(file)?;
        let mut zip_entry = archive.by_index(entry.entry_index)?;
        let mut data = Vec::with_capacity(zip_entry.size() as usize);
        zip_entry.read_to_end(&mut data)?;
        Ok(data)
    }

    /// Reads the bytes of one image by normalized entry path. The archive is
    /// reopened per call, so images are never all resident in memory at once.
    /// Returns `Ok(None)` for plain TXT inputs and unknown paths.
    pub fn read_image(&self, path: &str) -> Result<Option<Vec<u8>>, InputError> {
        if let Some(source) = &self.source {
            let Some(normalized) = normalize_entry_path(path) else {
                return Ok(None);
            };
            let Some(path_index) = self.find_image_path_index(&normalized) else {
                return Ok(None);
            };
            let mut reader = match source.open(&self.image_paths[path_index])? {
                Some(reader) => reader,
                None => return Ok(None),
            };
            let mut data = Vec::new();
            reader.read_to_end(&mut data)?;
            return Ok(Some(data));
        }
        if !self.archive {
            return Ok(None);
        }
        let Some(normalized) = normalize_entry_path(path) else {
            return Ok(None);
        };
        let Some(path_index) = self.find_image_path_index(&normalized) else {
            return Ok(None);
        };
        let entry_index = self.image_indices[path_index];
        let file = File::open(&self.path)?;
        let mut archive = ZipArchive::new(file)?;
        let mut zip_entry = archive.by_index(entry_index)?;
        let mut data = Vec::with_capacity(zip_entry.size() as usize);
        zip_entry.read_to_end(&mut data)?;
        Ok(Some(data))
    }

    /// Resolves an image reference to a normalized entry path without reading
    /// any bytes, mirroring `ImageInfoReader` behavior: the exact path first
    /// (as written in the text), then the path joined to the text entry's
    /// parent directory, each with extension correction
    /// (.png/.jpg/.jpeg/.gif/.webp, case-insensitive). Returns the matched
    /// entry path.
    pub fn resolve_image_path(&self, entry: &TextEntry, image_path: &str) -> Option<String> {
        let normalized = normalize_entry_path(image_path)?;
        if let Some(index) = self.find_image_path_index(&normalized) {
            return Some(self.image_paths[index].clone());
        }
        if !entry.parent.is_empty() {
            let joined = format!("{}/{}", entry.parent, normalized);
            if let Some(index) = self.find_image_path_index(&joined) {
                return Some(self.image_paths[index].clone());
            }
        }
        None
    }

    /// Resolves an image reference from a text entry and reads its bytes
    /// (see [`Input::resolve_image_path`] for the resolution rules).
    pub fn resolve_image(
        &self,
        entry: &TextEntry,
        image_path: &str,
    ) -> Result<Option<(String, Vec<u8>)>, InputError> {
        let Some(path) = self.resolve_image_path(entry, image_path) else {
            return Ok(None);
        };
        let Some(data) = self.read_image(&path)? else {
            return Ok(None);
        };
        Ok(Some((path, data)))
    }

    fn open_text(path: &Path) -> Result<Self, InputError> {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("book.txt")
            .to_owned();
        let entries = vec![TextEntry {
            name,
            parent: String::new(),
            entry_index: 0,
        }];
        Ok(Self {
            path: path.to_owned(),
            archive: false,
            entries,
            image_paths: Vec::new(),
            image_indices: Vec::new(),
            source: None,
        })
    }

    fn open_archive(path: &Path) -> Result<Self, InputError> {
        let file = File::open(path)?;
        let mut archive = ZipArchive::new(file)?;
        let mut entries = Vec::new();
        let mut image_paths = Vec::new();
        let mut image_indices = Vec::new();
        for index in 0..archive.len() {
            let zip_entry = archive.by_index(index)?;
            let Some(name) = decode_entry_name(zip_entry.name_raw()) else {
                continue;
            };
            let Some(normalized) = normalize_entry_path(&name) else {
                continue;
            };
            if normalized.to_ascii_lowercase().ends_with(".txt") {
                let parent = normalized
                    .rsplit_once('/')
                    .map(|(parent, _)| parent.to_owned())
                    .unwrap_or_default();
                entries.push(TextEntry {
                    name: normalized,
                    parent,
                    entry_index: index,
                });
            } else if image_media_type(&normalized).is_some() {
                image_paths.push(normalized);
                image_indices.push(index);
            }
        }
        Ok(Self {
            path: path.to_owned(),
            archive: true,
            entries,
            image_paths,
            image_indices,
            source: None,
        })
    }

    /// Locates an image entry by normalized path, with same-stem extension
    /// correction (.png/.jpg/.jpeg/.gif/.webp, case-insensitive). Returns the
    /// position inside [`Input::image_paths`].
    fn find_image_path_index(&self, base: &str) -> Option<usize> {
        if let Some(index) = self.image_paths.iter().position(|path| path == base) {
            return Some(index);
        }
        let (stem, _) = base.rsplit_once('.').unwrap_or((base, ""));
        for extension in ["png", "jpg", "jpeg", "gif", "webp"] {
            let candidate = format!("{stem}.{extension}");
            if let Some(index) = self
                .image_paths
                .iter()
                .position(|path| path.eq_ignore_ascii_case(&candidate))
            {
                return Some(index);
            }
        }
        None
    }
}

/// Detects the character encoding of text bytes the way the Java reference
/// does: UTF-8 BOM or valid UTF-8 is `"UTF-8"`, everything else is
/// `"SHIFT_JIS"` (MS932).
pub fn detect_encoding(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return "UTF-8";
    }
    let (_, _, had_errors) = UTF_8.decode(bytes);
    if had_errors { "SHIFT_JIS" } else { "UTF-8" }
}

/// Decodes text bytes, honoring an explicit encoding label when given and
/// falling back to auto-detection otherwise. A UTF-8 BOM is stripped.
///
/// `label` accepts `AUTO` (treated as auto-detection), `MS932`/`cp932`/
/// `Shift_JIS`/`sjis`/`windows-31j` and their usual aliases, `UTF-8`/`utf8`,
/// or any other label understood by `encoding_rs`.
pub fn decode_text(bytes: &[u8], label: Option<&str>) -> Result<String, InputError> {
    let encoding = match label {
        None | Some("AUTO") | Some("auto") => {
            if detect_encoding(bytes) == "UTF-8" {
                UTF_8
            } else {
                SHIFT_JIS
            }
        }
        Some(label) => encoding_for_label(label)
            .ok_or_else(|| InputError::UnsupportedEncoding(label.to_owned()))?,
    };
    let (decoded, _, had_errors) = encoding.decode(bytes);
    if had_errors {
        return Err(InputError::Decode(encoding.name().to_owned()));
    }
    Ok(decoded
        .strip_prefix('\u{feff}')
        .unwrap_or(decoded.as_ref())
        .to_owned())
}

fn encoding_for_label(label: &str) -> Option<&'static Encoding> {
    let normalized = label
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|character| !matches!(character, '-' | '_' | ' '))
        .collect::<String>();
    match normalized.as_str() {
        "utf8" => Some(UTF_8),
        "shiftjis" | "sjis" | "ms932" | "cp932" | "windows31j" | "mskanji" | "csshiftjis" => {
            Some(SHIFT_JIS)
        }
        _ => Encoding::for_label(label.trim().as_bytes()),
    }
}

/// Normalizes an archive entry path: forward slashes, no empty or `.`
/// segments, no `..` traversal, no leading slash. Returns `None` for
/// invalid paths.
pub fn normalize_entry_path(name: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return None,
            _ => parts.push(part),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

/// Decodes a raw archive entry name: UTF-8 first, then Shift_JIS/MS932
/// (matching the Java reference, which reads ZIP entry names as MS932).
fn decode_entry_name(raw: &[u8]) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    if let Ok(name) = std::str::from_utf8(raw) {
        return Some(name.to_owned());
    }
    let (decoded, _, _) = SHIFT_JIS.decode(raw);
    Some(decoded.into_owned())
}

/// Media type for image entries found in archives (the same set the Java
/// reference loads: png/jpg/jpeg/gif/webp).
fn image_media_type(path: &str) -> Option<&'static str> {
    let extension = path
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .unwrap_or_default();
    match extension.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use encoding_rs::SHIFT_JIS;
    use zip::write::{SimpleFileOptions, ZipWriter};

    use super::{
        FileSource, Input, InputError, decode_text, detect_encoding, normalize_entry_path,
    };

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct TempFile {
        path: std::path::PathBuf,
    }

    impl TempFile {
        fn new(name: &str, bytes: &[u8]) -> Self {
            let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "aozora_input_test_{}_{}_{name}",
                std::process::id(),
                counter
            ));
            std::fs::write(&path, bytes).unwrap();
            Self { path }
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in entries {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn open_zip(entries: &[(&str, &[u8])]) -> (TempFile, Input) {
        let bytes = make_zip(entries);
        let file = TempFile::new("book.zip", &bytes);
        let input = Input::open(&file.path).unwrap();
        (file, input)
    }

    #[test]
    fn enumerates_text_entries_in_archive_order_with_parents() {
        let (file, input) = open_zip(&[
            ("novel/01.txt", "一".as_bytes()),
            ("img/fig.png", b"\x89PNG\r\n\x1a\n"),
            ("novel/02.txt", "二".as_bytes()),
            ("cover.jpg", b"\xff\xd8"),
        ]);
        assert!(!input.is_image_only());
        let entries = input.text_entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "novel/01.txt");
        assert_eq!(entries[0].parent, "novel");
        assert_eq!(entries[1].name, "novel/02.txt");
        assert_eq!(entries[1].parent, "novel");

        assert_eq!(input.read_text(&entries[0]).unwrap(), "一".as_bytes());
        assert_eq!(input.read_text(&entries[1]).unwrap(), "二".as_bytes());
        assert!(input.image_paths().iter().any(|path| path == "img/fig.png"));
        assert!(input.image_paths().iter().any(|path| path == "cover.jpg"));
        assert_eq!(input.image_paths().len(), 2);
        drop(file);
    }

    #[test]
    fn reads_image_bytes_by_normalized_path_lazily() {
        let (file, input) = open_zip(&[
            ("img/a.png", b"png-a"),
            ("sub/img\\b.jpg", b"jpg-b"),
            ("img/c.webp", b"webp-c"),
        ]);
        assert!(input.is_image_only());
        assert_eq!(
            input.read_image("img/a.png").unwrap().as_deref(),
            Some(&b"png-a"[..])
        );
        // backslashes are normalized to forward slashes
        assert_eq!(
            input.read_image("sub/img/b.jpg").unwrap().as_deref(),
            Some(&b"jpg-b"[..])
        );
        assert_eq!(
            input.read_image("img/c.webp").unwrap().as_deref(),
            Some(&b"webp-c"[..])
        );
        // unknown and plain-TXT lookups yield None
        assert!(input.read_image("img/missing.png").unwrap().is_none());
        drop(file);
    }

    #[test]
    fn skips_directory_entries_and_traversal_names() {
        let (file, input) = open_zip(&[
            ("dir/", b""),
            ("novel/01.txt", b"x"),
            ("../evil.txt", b"evil"),
            ("img/fig.png", b"png"),
        ]);
        assert_eq!(input.text_entries().len(), 1);
        assert_eq!(input.text_entries()[0].name, "novel/01.txt");
        // "../evil.txt" is dropped and "dir/" never becomes an image
        assert_eq!(input.image_paths().len(), 1);
        assert!(input.image_paths().iter().any(|path| path == "img/fig.png"));
        drop(file);
    }

    #[test]
    fn resolves_images_from_text_entry_parent_with_extension_fallback() {
        let (file, input) = open_zip(&[
            ("novel/01.txt", b"x"),
            ("novel/fig.png", b"png"),
            ("img/other.JPG", b"jpg"),
        ]);
        let entry = &input.text_entries()[0];
        // exact root-relative path wins
        let (path, data) = input
            .resolve_image(entry, "novel/fig.png")
            .unwrap()
            .unwrap();
        assert_eq!(path, "novel/fig.png");
        assert_eq!(data, b"png");
        // parent-joined resolution for a bare reference
        let (path, data) = input.resolve_image(entry, "fig.png").unwrap().unwrap();
        assert_eq!(path, "novel/fig.png");
        assert_eq!(data, b"png");
        // extension fallback matches case-insensitively
        let (path, data) = input
            .resolve_image(entry, "img/other.png")
            .unwrap()
            .unwrap();
        assert_eq!(path, "img/other.JPG");
        assert_eq!(data, b"jpg");
        // missing image yields None
        assert!(input.resolve_image(entry, "missing.png").unwrap().is_none());
        drop(file);
    }

    #[test]
    fn opens_plain_text_files_with_a_single_entry() {
        let file = TempFile::new("book.txt", "こんにちは".as_bytes());
        let input = Input::open(&file.path).unwrap();
        assert!(!input.is_image_only());
        assert!(input.image_paths().is_empty());
        assert_eq!(input.text_entries().len(), 1);
        assert_eq!(input.text_entries()[0].parent, "");
        assert_eq!(
            input.read_text(&input.text_entries()[0]).unwrap(),
            "こんにちは".as_bytes()
        );
        drop(file);
    }

    #[test]
    fn sniffs_zip_magic_for_unknown_extensions() {
        let bytes = make_zip(&[("a.txt", b"x")]);
        let file = TempFile::new("book.unknown", &bytes);
        let input = Input::open(&file.path).unwrap();
        assert_eq!(input.text_entries().len(), 1);
        drop(file);
    }

    #[test]
    fn detects_utf8_bom_valid_utf8_and_shift_jis() {
        assert_eq!(detect_encoding(b"\xef\xbb\xbfabc"), "UTF-8");
        assert_eq!(detect_encoding("日本語".as_bytes()), "UTF-8");
        let (sjis, _, _) = SHIFT_JIS.encode("日本語");
        assert_eq!(detect_encoding(&sjis), "SHIFT_JIS");
    }

    #[test]
    fn decodes_with_auto_detection_and_explicit_overrides() {
        let (utf8, _, _) = SHIFT_JIS.encode("こんにちは");
        assert_eq!(decode_text(&utf8, None).unwrap(), "こんにちは");
        assert_eq!(decode_text(&utf8, Some("MS932")).unwrap(), "こんにちは");
        assert_eq!(decode_text(&utf8, Some("shift_jis")).unwrap(), "こんにちは");
        assert_eq!(
            decode_text(&utf8, Some("windows-31j")).unwrap(),
            "こんにちは"
        );

        let bom = b"\xef\xbb\xbf\xe3\x81\x82".to_vec();
        assert_eq!(decode_text(&bom, None).unwrap(), "あ");
        assert_eq!(decode_text(&bom, Some("UTF-8")).unwrap(), "あ");

        // explicit UTF-8 label on Shift_JIS bytes is an error, not a fallback
        assert!(decode_text(&utf8, Some("UTF-8")).is_err());
        assert!(decode_text(b"x", Some("no-such-encoding")).is_err());
        // AUTO behaves like auto-detection
        assert_eq!(decode_text(&utf8, Some("AUTO")).unwrap(), "こんにちは");
    }

    #[test]
    fn normalizes_entry_paths() {
        assert_eq!(normalize_entry_path("a/b.txt"), Some("a/b.txt".to_owned()));
        assert_eq!(normalize_entry_path("a\\b.txt"), Some("a/b.txt".to_owned()));
        assert_eq!(
            normalize_entry_path("./a/./b.txt"),
            Some("a/b.txt".to_owned())
        );
        assert_eq!(normalize_entry_path("/a/b.txt"), Some("a/b.txt".to_owned()));
        assert_eq!(normalize_entry_path("../a.txt"), None);
        assert_eq!(normalize_entry_path("a/../b.txt"), None);
        assert_eq!(normalize_entry_path(""), None);
        assert_eq!(normalize_entry_path("."), None);
    }

    /// In-memory [`FileSource`] standing in for object storage (R2 etc.).
    #[derive(Debug)]
    struct MemorySource {
        names: Vec<String>,
        files: std::collections::BTreeMap<String, Vec<u8>>,
    }

    impl MemorySource {
        fn new(files: &[(&str, &[u8])]) -> Self {
            Self {
                names: files.iter().map(|(name, _)| (*name).to_owned()).collect(),
                files: files
                    .iter()
                    .map(|(name, data)| ((*name).to_owned(), data.to_vec()))
                    .collect(),
            }
        }
    }

    impl FileSource for MemorySource {
        fn list(&self) -> &[String] {
            &self.names
        }
        fn open(&self, name: &str) -> Result<Option<Box<dyn Read + Send>>, InputError> {
            Ok(self
                .files
                .get(name)
                .map(|data| Box::new(Cursor::new(data.clone())) as Box<dyn Read + Send>))
        }
    }

    #[test]
    fn streams_text_and_images_from_a_file_source() {
        let input = Input::from_source(std::sync::Arc::new(MemorySource::new(&[
            ("novel/01.txt", "一".as_bytes()),
            ("img/fig.png", b"png"),
            ("img/other.JPG", b"jpg"),
        ])))
        .unwrap();
        assert!(!input.is_archive());
        assert_eq!(input.text_entries().len(), 1);
        assert_eq!(input.text_entries()[0].name, "novel/01.txt");
        assert_eq!(input.text_entries()[0].parent, "novel");
        assert_eq!(
            input.read_text(&input.text_entries()[0]).unwrap(),
            "一".as_bytes()
        );
        assert_eq!(
            input.image_paths(),
            &["img/fig.png".to_owned(), "img/other.JPG".to_owned()]
        );
        assert_eq!(
            input.read_image("img/fig.png").unwrap().as_deref(),
            Some(&b"png"[..])
        );
        // extension fallback and parent joining follow the archive rules
        let entry = &input.text_entries()[0];
        let (path, data) = input.resolve_image(entry, "img/fig.jpg").unwrap().unwrap();
        assert_eq!(path, "img/fig.png");
        assert_eq!(data, b"png");
        assert!(input.resolve_image(entry, "missing.png").unwrap().is_none());
        // a missing stream yields Ok(None), not an error
        assert!(input.read_image("img/absent.png").unwrap().is_none());
    }
}
