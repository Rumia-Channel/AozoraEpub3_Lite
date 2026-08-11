use std::io::{Cursor, Read};

use aozora_epub3_lite::{EpubAsset, EpubBook, EpubMetadata};
use zip::{CompressionMethod, ZipArchive};

#[test]
fn writes_epub3_layout_with_uncompressed_mimetype_first() {
    let metadata = EpubMetadata::new("試験 <作品>", "urn:test:epub").with_creator("著者 & 共著");
    let bytes = EpubBook::new(metadata, "    <p>本文</p>\n")
        .write_to(Cursor::new(Vec::new()))
        .unwrap()
        .into_inner();

    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    assert_eq!(archive.by_index(0).unwrap().name(), "mimetype");
    assert_eq!(
        archive.by_index(0).unwrap().compression(),
        CompressionMethod::Stored
    );

    for path in [
        "META-INF/container.xml",
        "item/standard.opf",
        "item/nav.xhtml",
        "item/toc.ncx",
        "item/style/book-style.css",
        "item/xhtml/0001.xhtml",
    ] {
        assert!(archive.by_name(path).is_ok(), "missing EPUB entry: {path}");
    }

    let mut package = String::new();
    archive
        .by_name("item/standard.opf")
        .unwrap()
        .read_to_string(&mut package)
        .unwrap();
    assert!(package.contains("<dc:title>試験 &lt;作品&gt;</dc:title>"));
    assert!(package.contains("<dc:creator id=\"creator\">著者 &amp; 共著</dc:creator>"));
}

#[test]
fn writes_all_sections_to_manifest_spine_and_navigation() {
    let book = EpubBook::from_sections(
        EpubMetadata::new("分割", "urn:test:sections"),
        ["    <p>一</p>\n", "    <p>二</p>\n"],
    );
    let bytes = book.write_to(Cursor::new(Vec::new())).unwrap().into_inner();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();

    assert!(archive.by_name("item/xhtml/0001.xhtml").is_ok());
    assert!(archive.by_name("item/xhtml/0002.xhtml").is_ok());

    let mut package = String::new();
    archive
        .by_name("item/standard.opf")
        .unwrap()
        .read_to_string(&mut package)
        .unwrap();
    assert!(package.contains("id=\"section-0001\""));
    assert!(package.contains("id=\"section-0002\""));
    assert!(package.contains("idref=\"section-0001\""));
    assert!(package.contains("idref=\"section-0002\""));

    let mut nav = String::new();
    archive
        .by_name("item/nav.xhtml")
        .unwrap()
        .read_to_string(&mut nav)
        .unwrap();
    assert!(nav.contains("xhtml/0001.xhtml"));
    assert!(nav.contains("xhtml/0002.xhtml"));
}

#[test]
fn writes_assets_and_manifest_entries() {
    let book = EpubBook::new(
        EpubMetadata::new("画像", "urn:test:image"),
        "    <p><img src=\"../image/sample.png\" alt=\"\"/></p>\n",
    )
    .with_assets([EpubAsset::new(
        "image/sample.png",
        "image/png",
        vec![0x89, b'P', b'N', b'G'],
    )]);
    let bytes = book.write_to(Cursor::new(Vec::new())).unwrap().into_inner();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();

    assert_eq!(
        archive
            .by_name("item/image/sample.png")
            .unwrap()
            .bytes()
            .count(),
        4
    );
    let mut package = String::new();
    archive
        .by_name("item/standard.opf")
        .unwrap()
        .read_to_string(&mut package)
        .unwrap();
    assert!(package.contains("href=\"image/sample.png\" media-type=\"image/png\""));
}
