//! ストリーミング書き出し（write_to_stream）の実動作検証用サンプル。
use std::fs::File;

use aozora_epub3_lite::{EpubBook, EpubMetadata};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let book = EpubBook::new(
        EpubMetadata::new("stream検証", "urn:uuid:stream-check"),
        "<p>ストリーミング書き出しの検証</p>",
    )
    .with_vertical(true);
    let file = File::create("target/stream-check.epub")?;
    book.write_to_stream(file)?;
    println!("generated: target/stream-check.epub");
    Ok(())
}
