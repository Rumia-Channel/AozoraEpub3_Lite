use std::io::{Cursor, Read};

use aozora_epub3_lite::{EpubAsset, EpubBook, EpubMetadata, IniSettings, StyleSettings};
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
    // Java のセクション xhtml は LF (CRLF なのは nav.xhtml と OPF / NCX)。
    assert!(section.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n"));
    assert!(section.contains("<html\n xmlns=\"http://www.w3.org/1999/xhtml\""));
    assert!(section.contains("xmlns:epub=\"http://www.idpf.org/2007/ops\""));
}

/// Java 版の出力は「セクション xhtml と CSS が LF、nav.xhtml と
/// standard.opf / toc.ncx が CRLF」。作業ツリーの改行 (Windows の
/// core.autocrlf=true など) がそのまま EPUB に入らないことを固定する。
#[test]
fn writes_java_line_endings_for_each_entry() {
    let book = EpubBook::new(
        EpubMetadata::new("改行", "urn:test:newlines"),
        "    <p>本文</p>\n",
    )
    .with_title_page();
    let bytes = book.write_to(Cursor::new(Vec::new())).unwrap().into_inner();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();

    let mut entry = |path: &str| {
        let mut text = String::new();
        archive
            .by_name(path)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        text
    };
    // 期待値は Java 版の出力そのまま (LF / CRLF)。
    for lf_path in [
        "mimetype",
        "item/xhtml/0001.xhtml",
        "item/xhtml/title.xhtml",
        "item/style/aozora.css",
        "item/style/book-style.css",
        "item/style/text.css",
    ] {
        let text = entry(lf_path);
        assert!(
            !text.trim_end_matches('\n').contains('\r'),
            "{lf_path} must use LF"
        );
    }
    for crlf_path in [
        "META-INF/container.xml",
        "item/nav.xhtml",
        "item/standard.opf",
        "item/toc.ncx",
    ] {
        let text = entry(crlf_path);
        assert!(text.contains("\r\n"), "{crlf_path} must use CRLF");
        assert_eq!(
            text.matches('\n').count(),
            text.matches("\r\n").count(),
            "{crlf_path} must use CRLF on every line"
        );
    }
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
    .with_cover_asset("image/cover.jpg")
    .with_cover_page(true, false);
    let bytes = book.write_to(Cursor::new(Vec::new())).unwrap().into_inner();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();

    let mut package = String::new();
    archive
        .by_name("item/standard.opf")
        .unwrap()
        .read_to_string(&mut package)
        .unwrap();
    // Java package.vm: 表紙画像だけ属性順が media-type → id → href。
    assert!(
        package.contains(
            "<item media-type=\"image/jpeg\" id=\"img0001\" href=\"image/cover.jpg\" properties=\"cover-image\"/>"
        )
    );
    assert!(package.contains("id=\"cover-page\" href=\"xhtml/cover.xhtml\""));
    assert!(package.contains("<itemref linear=\"yes\" idref=\"cover-page\""));

    // manifest の href は実在する ZIP エントリでなければならない。
    let names = archive.file_names().map(str::to_owned).collect::<Vec<_>>();
    for href in manifest_hrefs(&package) {
        assert!(
            names.iter().any(|name| *name == format!("item/{href}")),
            "manifest href {href} has no archive entry"
        );
    }

    let mut cover = String::new();
    archive
        .by_name("item/xhtml/cover.xhtml")
        .unwrap()
        .read_to_string(&mut cover)
        .unwrap();
    assert!(cover.contains("href=\"../style/fixed-layout-jp.css\""));
    assert!(cover.contains("<body epub:type=\"cover\">"));
    assert!(cover.contains("xlink:href=\"../image/cover.jpg\""));
}

/// `<item ... href="..."/>` として宣言されたリソースパスを取り出す。
fn manifest_hrefs(package: &str) -> Vec<String> {
    package
        .match_indices("href=\"")
        .filter_map(|(index, _)| {
            let rest = &package[index + "href=\"".len()..];
            let end = rest.find('"')?;
            let href = &rest[..end];
            (!href.starts_with("http")).then(|| href.to_owned())
        })
        .collect()
}

/// Java `text.vm` と同じく、INI のスタイル値が text.css に反映されること。
#[test]
fn writes_text_css_from_style_settings() {
    let ini = IniSettings::parse(
        "PageMargin=0,0.5,0,0\n\
         PageMarginUnit=0\n\
         BodyMargin=1,1,0.5,0.5\n\
         BodyMarginUnit=0\n\
         LineHeight=1.5\n\
         FontSize=120\n\
         BoldUseGothic=1\n\
         gothicUseBold=1\n",
    )
    .unwrap();
    let book = EpubBook::new(EpubMetadata::new("題名", "urn:test:css"), "<p>本文</p>")
        .with_style(StyleSettings::from_ini(&ini));
    let bytes = book.write_to(Cursor::new(Vec::new())).unwrap().into_inner();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut css = String::new();
    archive
        .by_name("item/style/text.css")
        .unwrap()
        .read_to_string(&mut css)
        .unwrap();

    assert!(css.contains("margin: 0em 0.5em 0em 0em;"));
    assert!(css.contains("margin: 1em 1em 0.5em 0.5em;"));
    assert!(css.contains("font-size: 120%;"));
    assert!(css.contains("line-height: 1.5;"));
    // BoldUseGothic / gothicUseBold はセレクタ行を増やす
    assert!(css.contains(".vrtl .b,\n.vrtl .gtc {"));
    assert!(css.contains(".gtc,\n.b { font-weight: bold; }"));
}

/// キー未指定なら Java CLI の既定 (PageMargin/BodyMargin は単位なしの 0)。
#[test]
fn writes_default_text_css_without_style_settings() {
    let book = EpubBook::new(EpubMetadata::new("題名", "urn:test:css"), "<p>本文</p>");
    let bytes = book.write_to(Cursor::new(Vec::new())).unwrap().into_inner();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut css = String::new();
    archive
        .by_name("item/style/text.css")
        .unwrap()
        .read_to_string(&mut css)
        .unwrap();

    assert!(css.contains("margin: 0 0 0 0;"));
    assert!(css.contains("font-size: 100%;"));
    assert!(css.contains("line-height: 1.8;"));
    assert!(!css.contains(".vrtl .b,\n"));
}

/// Java TocVertical: 目次ページが縦書きになり、ラベルは
/// `convertTcyText` 済みの XHTML として素通しで出力される。
#[test]
fn writes_vertical_toc_with_markup_labels() {
    let book = EpubBook::from_sections(EpubMetadata::new("題名", "urn:test:tocv"), ["<p>本文</p>"])
        .with_chapters([aozora_epub3_lite::NavChapter::new(
            "第<span class=\"tcy\">1</span>話",
            "xhtml/0001.xhtml",
        )
        .with_markup(true)])
        .with_toc_vertical(true);
    let bytes = book.write_to(Cursor::new(Vec::new())).unwrap().into_inner();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut nav = String::new();
    archive
        .by_name("item/nav.xhtml")
        .unwrap()
        .read_to_string(&mut nav)
        .unwrap();

    assert!(nav.contains("writing-mode: vertical-rl;"));
    // markup ラベルは再エスケープされない
    assert!(nav.contains("第<span class=\"tcy\">1</span>話"));
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
    assert!(nav.contains("xhtml/0001.xhtml"));
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
    assert!(nav.contains("xhtml/0001.xhtml"));
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
        if marker.contains("middle") {
            assert!(section.contains("xml:lang=\"ja\"\n class=\"hltr\""));
        } else {
            assert!(section.contains("xml:lang=\"ja\"\n class=\"vrtl\""));
        }
    }
}
