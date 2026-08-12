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

    let mut image = Vec::new();
    archive
        .by_name("item/image/sample.png")
        .unwrap()
        .read_to_end(&mut image)
        .unwrap();
    assert_eq!(image.len(), 4);
    let mut package = String::new();
    archive
        .by_name("item/standard.opf")
        .unwrap()
        .read_to_string(&mut package)
        .unwrap();
    assert!(package.contains("href=\"image/sample.png\" media-type=\"image/png\""));
}

#[test]
fn writes_cover_document_and_cover_manifest_property() {
    let book = EpubBook::new(
        EpubMetadata::new("表紙", "urn:test:cover"),
        "    <p>本文</p>\n",
    )
    .with_assets([EpubAsset::new(
        "image/cover.jpg",
        "image/jpeg",
        vec![0xff, 0xd8, 0xff],
    )])
    .with_cover_asset("image/cover.jpg");
    let bytes = book.write_to(Cursor::new(Vec::new())).unwrap().into_inner();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();

    let mut package = String::new();
    archive
        .by_name("item/standard.opf")
        .unwrap()
        .read_to_string(&mut package)
        .unwrap();
    assert!(
        package.contains(
            "href=\"image/cover.jpg\" media-type=\"image/jpeg\" properties=\"cover-image\""
        )
    );
    assert!(package.contains("id=\"cover\" href=\"cover.xhtml\""));
    assert!(package.contains("<itemref idref=\"cover\"/>"));

    let mut cover = String::new();
    archive
        .by_name("item/cover.xhtml")
        .unwrap()
        .read_to_string(&mut cover)
        .unwrap();
    assert!(cover.contains("<img src=\"image/cover.jpg\""));
}

#[test]
fn writes_publisher_metadata_and_kindle_body_class() {
    let book = EpubBook::new(
        EpubMetadata::new("題名", "urn:test:kindle")
            .with_creator("著者")
            .with_publisher("出版社"),
        "<p>本文</p>",
    )
    .with_title_page()
    .with_kindle(true);
    let bytes = book.write_to(Cursor::new(Vec::new())).unwrap().into_inner();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();

    let mut package = String::new();
    archive
        .by_name("item/standard.opf")
        .unwrap()
        .read_to_string(&mut package)
        .unwrap();
    assert!(package.contains("<dc:publisher>出版社</dc:publisher>"));

    let mut title = String::new();
    archive
        .by_name("item/xhtml/title.xhtml")
        .unwrap()
        .read_to_string(&mut title)
        .unwrap();
    assert!(title.contains("<body class=\"p-titlepage kindle\">"));
    assert!(title.contains("<div class=\"publisher\"><p>出版社</p></div>"));
}
