//! `pipeline` モジュールのテスト (`src/text_tests.rs` と同じ配置規則)。

use super::*;
use crate::{EpubAsset, FileSource, IniSettings, Input, InputError};
use std::io::{Cursor, Read};

/// 1参照1assetの CollectedAsset を構築（src は `../image/{参照名}` の形式）。
fn collected(asset: EpubAsset) -> CollectedAsset {
    let reference = asset
        .path
        .strip_prefix("image/")
        .unwrap_or(&asset.path)
        .to_owned();
    let dimensions = asset
        .data
        .as_deref()
        .and_then(|data| image_dimensions(data, &asset.media_type));
    CollectedAsset {
        asset,
        references: vec![reference.clone()],
        available: vec![true],
        source: reference.clone(),
        resolved: reference,
        dimensions,
        is_cover: false,
        rotate: 0,
    }
}

#[test]
fn sorts_image_names_like_java() {
    let mut names = vec!["2.png", "10.png", "_cover.png", "第3話.png", "第1話.png"];
    names.sort_by(|left, right| compare_image_names(left, right));
    assert_eq!(
        names,
        vec!["_cover.png", "10.png", "2.png", "第1話.png", "第3話.png"]
    );
}

#[test]
fn generates_java_compatible_name_uuid() {
    assert_eq!(
        java_name_uuid("横書き横組み", "テスト"),
        "27128c1c-ed73-341e-ae9d-9d052775453a"
    );
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
    let mut assets = vec![collected(asset)];
    decorate_image_tags(&mut sections, &mut assets, &config, false);
    // Java の画像タグは width/height 属性も CSS 回転も持たない
    assert!(!sections[0].contains("width=\"1600\""));
    assert!(!sections[0].contains("transform:"));
    // 回転は画素に対して行われる (Java imageInfo.rotateAngle)
    assert_eq!(assets[0].rotate, 90);
}

#[test]
fn decorates_inline_images_with_java_width_ratio() {
    let mut png = vec![0; 24];
    png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    png[16..20].copy_from_slice(&459u32.to_be_bytes());
    png[20..24].copy_from_slice(&350u32.to_be_bytes());
    let asset = EpubAsset::new("image/fig.png", "image/png", png);
    let config = AozoraConfig::from_ini(
        IniSettings::parse("DispW=600\nDispH=800\nSinglePageWidth=1000\nImageScale=1\n").unwrap(),
    );
    let mut sections = vec![
        "<p><span><img class=\"fit\" src=\"../image/fig.png\" alt=\"図\"/></span></p>".to_owned(),
    ];
    decorate_image_tags(&mut sections, &mut [collected(asset)], &config, false);
    assert!(sections[0].contains("<span class=\"img\" style=\"width:76.5%\">"));
    assert!(sections[0].contains("<img style=\"width:100%\""));
    assert!(!sections[0].contains("width=\"459\""));
}

#[test]
fn uses_fit_template_for_extension_fallback_references() {
    // Java: getImageWidthRatio(srcFilePath) は元の参照名（img/fig.jpg）で
    // 画像を引けなければ ratio=0 となり fit テンプレートになる（拡張子
    // フォールバックで解決された参照では幅%を付けない）
    let mut png = vec![0; 24];
    png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    png[16..20].copy_from_slice(&459u32.to_be_bytes());
    png[20..24].copy_from_slice(&350u32.to_be_bytes());
    let asset = EpubAsset::new("image/fig.png", "image/png", png);
    let collected = CollectedAsset {
        dimensions: asset
            .data
            .as_deref()
            .and_then(|data| image_dimensions(data, &asset.media_type)),
        is_cover: false,
        asset,
        references: vec!["img/fig.jpg".to_owned()],
        available: vec![false],
        source: "img/fig.jpg".to_owned(),
        resolved: "fig.png".to_owned(),
        rotate: 0,
    };
    let config = AozoraConfig::from_ini(
        IniSettings::parse("DispW=600\nDispH=800\nSinglePageWidth=1000\nImageScale=1\n").unwrap(),
    );
    let mut sections = vec![
        "<p><span><img class=\"fit\" src=\"../image/img/fig.jpg\" alt=\"図\"/></span></p>"
            .to_owned(),
    ];
    decorate_image_tags(&mut sections, &mut [collected], &config, false);
    assert!(sections[0].contains("class=\"fit\""));
    assert!(!sections[0].contains("width:"));
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
        "<p><span><img class=\"fit\" src=\"../image/float.png\" alt=\"\"/></span></p>".to_owned(),
    ];
    decorate_image_tags(&mut sections, &mut [collected(asset)], &config, false);
    assert!(sections[0].contains("<span class=\"img ft\""));
    assert!(sections[0].contains("style=\"width:83.33333333333334%\""));
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
        "<p><span><img class=\"fit\" src=\"../image/page.png\" alt=\"\"/></span></p>".to_owned(),
    ];
    decorate_image_tags(&mut sections, &mut [collected(asset)], &config, false);
    assert!(sections[0].contains("<img class=\"fit\""));
    assert!(!sections[0].contains("height:"));
}

#[test]
fn removes_missing_image_only_paragraphs() {
    let mut sections = vec![
        "<p><span><img class=\"fit\" src=\"../image/missing.png\" alt=\"未解決\"/></span></p>"
            .to_owned(),
    ];
    remove_missing_image_sources(&mut sections, &["missing.png".to_owned()], &[]);
    assert_eq!(sections[0], "");
}

#[test]
fn preserves_missing_image_placeholder_inside_content() {
    let mut sections = vec![
        "<p>前</p>\n\
             <p><span><img class=\"fit\" src=\"../image/missing.png\" alt=\"未解決\"/></span></p>\n\
             <p>後</p>"
            .to_owned(),
    ];
    remove_missing_image_sources(&mut sections, &["missing.png".to_owned()], &[]);
    assert_eq!(sections[0], "<p>前</p>\n<p><br/></p>\n<p>後</p>");
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
    let asset = collected(EpubAsset::new("image/large.png", "image/png", png));
    let config = AozoraConfig::from_ini(
        IniSettings::parse("DispW=584\nDispH=754\nSinglePageWidth=550\n").unwrap(),
    );
    let mut sections = vec![
            "<p>前</p>\n<p><span><img class=\"fit\" src=\"../image/large.png\" alt=\"\"/></span></p>\n<p>後</p>\n"
                .to_owned(),
        ];
    reflow_image_sections(&mut sections, &mut [], &[asset], &config);
    assert_eq!(sections.len(), 3);
    assert!(sections[1].contains("<span><img"));
    assert!(sections[1].contains("src=\"../image/large.png\""));
    assert!(!sections[1].contains("<p>"));
    assert!(sections[0].contains("<p>前</p>"));
    assert!(sections[2].contains("<p>後</p>"));
}

#[test]
fn keeps_large_images_inside_block_containers_inline() {
    let mut png = vec![0; 24];
    png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    png[16..20].copy_from_slice(&1836u32.to_be_bytes());
    png[20..24].copy_from_slice(&1400u32.to_be_bytes());
    let asset = collected(EpubAsset::new("image/large.png", "image/png", png));
    let config = AozoraConfig::from_ini(
        IniSettings::parse("DispW=584\nDispH=754\nSinglePageWidth=550\n").unwrap(),
    );
    let mut sections = vec![
        "<div class=\"mt5\">\n\
             <p><span><img class=\"fit\" src=\"../image/large.png\" alt=\"\"/></span></p>\n\
             <p>後</p>\n\
             </div>\n"
            .to_owned(),
    ];
    reflow_image_sections(&mut sections, &mut [], &[asset], &config);
    assert_eq!(sections.len(), 1);
    assert!(sections[0].contains("<div class=\"mt5\">"));
    assert!(sections[0].contains("<img class=\"fit\""));
    assert!(sections[0].contains("<p>後</p>"));
}

#[test]
fn keeps_class_carrying_empty_spans_when_images_are_missing() {
    // 画像取得失敗で <img> が消えた後、画像ラッパー（class なし）の空 span だけが
    // 除去される。二分アキなどクラス付きの空 span は残す。
    let mut sections = vec![format!(
        "<p><span class=\"half_em_space\"></span>{text}<span><img class=\"fit\" src=\"../image/missing.png\" alt=\"\"/></span></p>",
        text = "\u{300c}\u{305d}\u{3063}\u{3061}\u{ff1f}\u{300d}",
    )];
    remove_missing_image_sources(&mut sections, &["missing.png".to_owned()], &[]);
    assert_eq!(
        sections[0],
        "<p><span class=\"half_em_space\"></span>\u{300c}\u{305d}\u{3063}\u{3061}\u{ff1f}\u{300d}</p>"
    );
}

/// PNG のヘッダのみ (寸法判定用)。ピクセルは読まれない。
fn png_header(width: u32, height: u32) -> Vec<u8> {
    let mut png = vec![0; 24];
    png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    png[16..20].copy_from_slice(&width.to_be_bytes());
    png[20..24].copy_from_slice(&height.to_be_bytes());
    png
}

/// ストリーミング FileSource のテスト用実装。
#[derive(Debug)]
struct MemorySource {
    names: Vec<String>,
    files: Vec<(String, Vec<u8>)>,
}

impl MemorySource {
    fn new(files: &[(&str, Vec<u8>)]) -> Self {
        Self {
            names: files.iter().map(|(name, _)| (*name).to_owned()).collect(),
            files: files
                .iter()
                .map(|(name, data)| ((*name).to_owned(), data.clone()))
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
            .iter()
            .find(|(path, _)| path == name)
            .map(|(_, data)| Box::new(Cursor::new(data.clone())) as Box<dyn Read + Send>))
    }
}

/// FileSource 入力 (narou.rs の Worker 経路) でも挿絵を Input 経由で解決し、
/// CLI と同じ順 (収集 → 装飾 → 参照名の書き換え → 表紙画像の除外 → 再構成) で
/// 単ページ画像化・連番・表紙判定まで通ることを確認する。
#[test]
fn collects_and_reflows_images_from_a_file_source() {
    let text = "表題\n著者\n\n［＃区切り線］\n［＃改ページ］\n\
        ［＃挿絵（cover.png）入る］\n［＃改ページ］\n\
        ［＃挿絵（fig.png）入る］\n［＃改ページ］\n本文\n";
    let source = MemorySource::new(&[
        ("dir/book.txt", text.as_bytes().to_vec()),
        ("dir/cover.png", png_header(700, 900)),
        ("dir/fig.png", png_header(700, 900)),
    ]);
    let input = Input::from_source(std::sync::Arc::new(source)).expect("source input");
    assert!(!input.is_archive(), "FileSource 入力はアーカイブではない");
    assert!(input.has_source());
    let entry = input.text_entries()[0].clone();
    let config = AozoraConfig::default();
    let (mut sections, mut chapters) =
        crate::aozora_text_to_xhtml_sections_with_chapters(text, &config, true).unwrap();

    // -c 0 (先頭の挿絵を表紙) 相当
    let (mut assets, cover) = collect_assets(&input, &entry, text, Some("0"), None).unwrap();
    assert_eq!(assets.len(), 2, "両方の挿絵が解決される");
    assert_eq!(assets[0].asset.path, "image/0001.png");
    assert_eq!(assets[0].source, "dir/cover.png", "実ファイル名は source");
    assert_eq!(assets[0].references, vec!["cover.png".to_owned()]);
    assert_eq!(assets[0].resolved, "0001.png");
    assert_eq!(
        assets[0]
            .dimensions
            .map(|value| (value.width, value.height)),
        Some((700, 900))
    );
    assert!(assets[0].is_cover);
    assert_eq!(cover.as_deref(), Some("image/0001.png"));

    decorate_image_tags(&mut sections, &mut assets, &config, true);
    for collected in &assets {
        for reference in &collected.references {
            if collected.resolved != *reference {
                rewrite_image_source(&mut sections, reference, &collected.resolved);
            }
        }
    }
    remove_image_sources(&mut sections, &[cover.clone().unwrap()]);
    reflow_image_sections(&mut sections, &mut chapters, &assets, &config);

    let body = sections.join("");
    assert!(
        !body.contains("0001.png"),
        "表紙画像は本文から消える: {body}"
    );
    assert!(
        body.contains("../image/0002.png"),
        "挿絵は連番で残る: {body}"
    );
    assert!(
        body.contains("<span><img class=\"fit\""),
        "大きい挿絵は単ページ画像になる: {body}"
    );
}
