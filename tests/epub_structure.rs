use std::io::{Cursor, Read};

use aozora_epub3_lite::{EpubBook, EpubMetadata};
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
