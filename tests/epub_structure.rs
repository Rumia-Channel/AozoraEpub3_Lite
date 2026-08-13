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
    assert!(package.contains("<dc:title id=\"title\">試験 &lt;作品&gt;</dc:title>"));
    assert!(package.contains("<dc:creator id=\"creator01\">著者 &amp; 共著</dc:creator>"));
    let mut section = String::new();
    archive
        .by_name("item/xhtml/0001.xhtml")
        .unwrap()
        .read_to_string(&mut section)
        .unwrap();
    assert!(section.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n"));
    assert!(section.contains("<html\n xmlns=\"http://www.w3.org/1999/xhtml\""));
    assert!(section.contains("xmlns:epub=\"http://www.idpf.org/2007/ops\""));
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
    assert!(package.contains("id=\"sec0001\""));
    assert!(package.contains("id=\"sec0002\""));
    assert!(package.contains("idref=\"sec0001\""));
    assert!(package.contains("idref=\"sec0002\""));

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
fn title_page_navigation_includes_unheaded_body_entries() {
    let book = EpubBook::new(
        EpubMetadata::new("題名", "urn:test:title-navigation"),
        "<p>本文だけ</p>\n",
    )
    .with_metadata_markup("題名", None)
    .with_title_page();
    let bytes = book.write_to(Cursor::new(Vec::new())).unwrap().into_inner();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut nav = String::new();
    archive
        .by_name("item/nav.xhtml")
        .unwrap()
        .read_to_string(&mut nav)
        .unwrap();
    let toc = nav.split("<nav epub:type=\"toc\"").nth(1).unwrap();
    assert!(toc.contains("xhtml/title.xhtml"));
    assert!(toc.contains("xhtml/0001.xhtml"));
}

#[test]
fn title_page_uses_java_xhtml_head_and_spacing() {
    let book = EpubBook::new(
        EpubMetadata::new("題名", "urn:test:title-template").with_creator("著者"),
        "<p>本文</p>\n",
    )
    .with_metadata_markup("題名", Some("著者".to_owned()))
    .with_title_page();
    let bytes = book.write_to(Cursor::new(Vec::new())).unwrap().into_inner();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut title = String::new();
    archive
        .by_name("item/xhtml/title.xhtml")
        .unwrap()
        .read_to_string(&mut title)
        .unwrap();
    assert!(!title.contains("<meta charset=\"UTF-8\"/>"));
    assert!(title.contains(
        "<link rel=\"stylesheet\" type=\"text/css\" href=\"../style/book-style.css\"/>\n\n<title>"
    ));
    assert!(title.contains(
        "<div class=\"main vrtl block-align-center\">\n\n\t<br/>\n\n<div class=\"book-title start-2em\">"
    ));
    assert!(title.contains("</div>\n<div class=\"author\"><p>著者</p></div>\n\n</div>"));
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
fn writes_gaiji_font_assets_and_dynamic_font_css() {
    let book = EpubBook::new(
        EpubMetadata::new("外字", "urn:test:gaiji"),
        "<p><span class=\"glyph u3048-u3099\">え</span></p>",
    )
    .with_assets([EpubAsset::new(
        "gaiji/u3048-u3099.ttf",
        "application/font-sfnt",
        vec![0, 1, 2, 3],
    )]);
    let bytes = book.write_to(Cursor::new(Vec::new())).unwrap().into_inner();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();

    let mut font = Vec::new();
    archive
        .by_name("item/gaiji/u3048-u3099.ttf")
        .unwrap()
        .read_to_end(&mut font)
        .unwrap();
    assert_eq!(font, vec![0, 1, 2, 3]);

    let mut package = String::new();
    archive
        .by_name("item/standard.opf")
        .unwrap()
        .read_to_string(&mut package)
        .unwrap();
    assert!(
        package.contains("href=\"gaiji/u3048-u3099.ttf\" media-type=\"application/font-sfnt\"")
    );

    let mut css = String::new();
    archive
        .by_name("item/style/text.css")
        .unwrap()
        .read_to_string(&mut css)
        .unwrap();
    assert!(
        css.contains(
            "@font-face {font-family:\"u3048-u3099\"; src:url(../gaiji/u3048-u3099.ttf);}"
        )
    );
    assert!(css.contains(".u3048-u3099 {font-family:\"u3048-u3099\";}"));
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
    assert!(package.contains("id=\"cover-page\" href=\"xhtml/cover.xhtml\""));
    assert!(package.contains("<itemref linear=\"yes\" idref=\"cover-page\""));

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
    assert!(package.contains("<dc:publisher id=\"publisher\">出版社</dc:publisher>"));

    let mut title = String::new();
    archive
        .by_name("item/xhtml/title.xhtml")
        .unwrap()
        .read_to_string(&mut title)
        .unwrap();
    assert!(title.contains("<body class=\"p-titlepage kindle\">"));
    assert!(title.contains("<div class=\"publisher\"><p>出版社</p></div>"));
}

#[test]
fn writes_heading_levels_as_nested_navigation() {
    let book = EpubBook::from_sections(
        EpubMetadata::new("階層", "urn:test:hierarchy"),
        [
            "<h1 class=\"font-1em50\">第一章</h1>\n",
            "<h2 class=\"font-1em30\">第一節</h2>\n",
            "<h1 class=\"font-1em50\">第二章</h1>\n",
        ],
    );
    let bytes = book.write_to(Cursor::new(Vec::new())).unwrap().into_inner();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut nav = String::new();
    archive
        .by_name("item/nav.xhtml")
        .unwrap()
        .read_to_string(&mut nav)
        .unwrap();
    assert!(nav.contains("第一章"));
    assert!(nav.contains("第一節"));
    assert!(nav.contains("第二章"));
}

#[test]
fn navigation_labels_omit_ruby_readings() {
    let book = EpubBook::from_sections(
        EpubMetadata::new("ルビ", "urn:test:ruby-navigation"),
        ["<h1><ruby>漢字<rt>かんじ</rt></ruby></h1>\n"],
    );
    let bytes = book.write_to(Cursor::new(Vec::new())).unwrap().into_inner();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut nav = String::new();
    archive
        .by_name("item/nav.xhtml")
        .unwrap()
        .read_to_string(&mut nav)
        .unwrap();
    assert!(nav.contains(">漢字</a>"));
    assert!(!nav.contains("かんじ"));
}

#[test]
fn renders_middle_and_bottom_pages_with_horizontal_document_class() {
    for marker in ["<!-- aozora-page-middle -->", "<!-- aozora-page-bottom -->"] {
        let book = EpubBook::new(
            EpubMetadata::new("ページ", format!("urn:test:{marker}")),
            format!("{marker}\n<p>本文</p>\n"),
        );
        let bytes = book.write_to(Cursor::new(Vec::new())).unwrap().into_inner();
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut section = String::new();
        archive
            .by_name("item/xhtml/0001.xhtml")
            .unwrap()
            .read_to_string(&mut section)
            .unwrap();
        assert!(section.contains("xml:lang=\"ja\"\n class=\"hltr\""));
        assert!(!section.contains("xml:lang=\"ja\"\n class=\"vrtl\""));
    }
}
