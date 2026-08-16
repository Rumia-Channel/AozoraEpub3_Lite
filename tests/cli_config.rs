use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use zip::ZipArchive;

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("aozora-epub3-{label}-{stamp}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn cli_applies_external_ini_and_config_dir_to_generated_epub() {
    let root = TempDir::new("cli-config");
    let output_dir = root.path().join("out");
    let config_dir = root.path().join("config");
    fs::create_dir_all(&output_dir).unwrap();
    fs::create_dir_all(&config_dir).unwrap();

    let input = root.path().join("book.txt");
    let ini = root.path().join("profile.ini");
    fs::write(&input, "［＃試験注記］本文［＃試験注記終わり］\n").unwrap();
    fs::write(
        &ini,
        "Ext=.configured.epub\nCover=表紙無し\nVertical=0\nTitleType=5\n",
    )
    .unwrap();
    fs::write(
        config_dir.join("custom_chuki_tag.txt"),
        "試験注記\t<span class=\"custom-note\">\n\
         試験注記終わり\t</span>\n",
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_AozoraEpub3_Lite"))
        .args([
            "--ini",
            ini.to_str().unwrap(),
            "--config-dir",
            config_dir.to_str().unwrap(),
            "--of",
            "--dst",
            output_dir.to_str().unwrap(),
            input.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = output_dir.join("book.configured.epub");
    assert!(
        output.is_file(),
        "missing generated EPUB: {}",
        output.display()
    );

    let file = fs::File::open(output).unwrap();
    let mut archive = ZipArchive::new(file).unwrap();
    assert!(archive.by_name("item/image/0001.png").is_err());

    let mut section = String::new();
    archive
        .by_name("item/xhtml/0001.xhtml")
        .unwrap()
        .read_to_string(&mut section)
        .unwrap();
    assert!(section.contains("class=\"custom-note\">本文</span>"));
    assert!(section.contains("class=\"hltr\""));
}

#[test]
fn cli_overrides_preset_values_and_applies_metadata_options() {
    let root = TempDir::new("cli-preset-overrides");
    let output_dir = root.path().join("out");
    fs::create_dir_all(&output_dir).unwrap();

    let input = root.path().join("book.txt");
    let preset = root.path().join("preset.ini");
    fs::write(&input, "本文\n").unwrap();
    fs::write(&preset, "Ext=.preset.epub\nVertical=0\nCover=表紙無し\n").unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_AozoraEpub3_Lite"))
        .args([
            "--preset",
            preset.to_str().unwrap(),
            "--ext",
            ".override.epub",
            "--vertical",
            "--cover",
            "0",
            "--creator",
            "試験著者",
            "--language",
            "en",
            "--of",
            "--dst",
            output_dir.to_str().unwrap(),
            input.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = output_dir.join("book.override.epub");
    assert!(
        output.is_file(),
        "missing generated EPUB: {}",
        output.display()
    );

    let file = fs::File::open(output).unwrap();
    let mut archive = ZipArchive::new(file).unwrap();
    let mut section = String::new();
    archive
        .by_name("item/xhtml/0001.xhtml")
        .unwrap()
        .read_to_string(&mut section)
        .unwrap();
    assert!(section.contains("class=\"vrtl\""));

    let mut package = String::new();
    archive
        .by_name("item/standard.opf")
        .unwrap()
        .read_to_string(&mut package)
        .unwrap();
    assert!(package.contains("xml:lang=\"en\""));
    assert!(package.contains(">試験著者</dc:creator>"));
}

#[test]
fn cli_embeds_gaiji_font_loaded_from_config_dir() {
    let root = TempDir::new("cli-gaiji");
    let output_dir = root.path().join("out");
    let config_dir = root.path().join("config");
    let gaiji_dir = config_dir.join("gaiji");
    fs::create_dir_all(&output_dir).unwrap();
    fs::create_dir_all(&gaiji_dir).unwrap();

    let input = root.path().join("book.txt");
    fs::write(&input, "本文\n※［＃U+845B］\n").unwrap();
    fs::write(
        config_dir.join("chuki_utf.txt"),
        "U+845B\t\t葛\t※［＃U+845B］\n",
    )
    .unwrap();
    fs::write(gaiji_dir.join("u845b.ttf"), b"test font bytes").unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_AozoraEpub3_Lite"))
        .args([
            "-t",
            "5",
            "--config-dir",
            config_dir.to_str().unwrap(),
            "--cover",
            "表紙無し",
            "--of",
            "--dst",
            output_dir.to_str().unwrap(),
            input.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = output_dir.join("book.epub");
    let file = fs::File::open(output).unwrap();
    let mut archive = ZipArchive::new(file).unwrap();
    let mut section = String::new();
    archive
        .by_name("item/xhtml/0001.xhtml")
        .unwrap()
        .read_to_string(&mut section)
        .unwrap();
    assert!(
        section.contains("<span class=\"glyph u845b\">〓</span>"),
        "{section}"
    );
    assert_eq!(
        archive.by_name("item/gaiji/u845b.ttf").unwrap().size(),
        b"test font bytes".len() as u64
    );
}
