use aozora_epub3_lite::{AozoraConfig, EpubAsset, EpubBook, EpubMetadata, EpubSection, IniSettings, MetadataScanner, SectionStream, TitleType};
use image::{AnimationDecoder, DynamicImage, ImageEncoder};
use std::cell::RefCell;
use std::io::{self, Cursor, Read, Write};
use std::rc::Rc;

#[derive(Clone, Default)]
struct DrainSink(Rc<RefCell<Vec<u8>>>);

impl Write for DrainSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut pending = self.0.borrow_mut();
        if pending.len() + bytes.len() > 128 * 1024 {
            return Err(io::Error::other("producer did not honor backpressure"));
        }
        pending.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl DrainSink {
    fn drain(&self, output: &mut Vec<u8>) {
        output.extend(std::mem::take(&mut *self.0.borrow_mut()));
    }
}

#[test]
fn zip_entry_is_written_in_chunks_and_deferred_sections_are_replayed() {
    let mut book = EpubBook::new(EpubMetadata::new("Streaming", "urn:test:streaming"), "");
    book.sections = vec![EpubSection::deferred(false), EpubSection::deferred(false)];
    book.assets.push(EpubAsset::lazy("image/0001.png", "image/png"));
    let sink = DrainSink::default();
    let mut writer = book.stream_writer(sink.clone()).unwrap();
    let mut archive = Vec::new(); // Explicit test consumer boundary.
    let payload: Vec<u8> = (0..32768u32).map(|value| (value.wrapping_mul(997) >> 3) as u8).collect();
    while let Some(entry) = writer.next_entry() {
        let rendered = if entry.asset_path.is_some() { None } else {
            let body = entry.section_index.map(|index| if index == 0 { "<p>First</p>" } else { "<p>Second</p>" });
            Some(writer.render_current(body).unwrap())
        };
        writer.begin_current().unwrap();
        sink.drain(&mut archive);
        if entry.asset_path.is_some() {
            for _ in 0..96 {
                writer.write_chunk(&payload).unwrap();
                sink.drain(&mut archive);
            }
        } else {
            for chunk in rendered.as_deref().unwrap().chunks(32768) {
                writer.write_chunk(chunk).unwrap();
                sink.drain(&mut archive);
            }
        }
        writer.end_current().unwrap();
        sink.drain(&mut archive);
    }
    writer.finish().unwrap();
    sink.drain(&mut archive);
    let mut archive = zip::ZipArchive::new(Cursor::new(archive)).unwrap();
    for (name, expected) in [("item/xhtml/0001.xhtml", "First"), ("item/xhtml/0002.xhtml", "Second")] {
        let mut text = String::new();
        archive.by_name(name).unwrap().read_to_string(&mut text).unwrap();
        assert!(text.contains(expected));
    }
    let mut image = archive.by_name("item/image/0001.png").unwrap();
    let mut chunk = vec![0; payload.len()];
    for _ in 0..96 {
        image.read_exact(&mut chunk).unwrap();
        assert_eq!(chunk, payload);
    }
    assert_eq!(image.read(&mut chunk).unwrap(), 0);
}

#[test]
fn incomplete_and_failed_zip_writes_cannot_finish_successfully() {
    let book = || EpubBook::new(EpubMetadata::new("Errors", "urn:test:errors"), "<p>body</p>");
    assert!(book().stream_writer(Vec::new()).unwrap().finish().is_err());
    let mut writer = book().stream_writer(Vec::new()).unwrap();
    assert!(writer.write_chunk(b"before begin").is_err());
    writer.begin_current().unwrap();
    assert!(writer.begin_current().is_err());
    writer.write_chunk(b"application/epub+zip").unwrap();
    assert!(writer.finish().is_err());

    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> { Err(io::Error::other("disconnected")) }
        fn flush(&mut self) -> io::Result<()> { Err(io::Error::other("disconnected")) }
    }
    let mut writer = book().stream_writer(Broken).unwrap();
    assert!(writer.begin_current().is_err());
    assert!(writer.finish().is_err());
}

#[test]
fn incremental_conversion_emits_without_retaining_previous_sections() {
    let config = AozoraConfig::default();
    let mut stream = SectionStream::new(&config, true);
    let mut emitted = 0usize;
    for index in 0..2000 {
        let heading = format!("章{index}");
        stream.push_line(&heading, &mut |_| panic!("unexpected page boundary")).unwrap();
        stream.push_line("｜本文《ほんぶん》", &mut |_| panic!("unexpected page boundary")).unwrap();
        stream.push_line("［＃改ページ］", &mut |fragment| {
            assert!(fragment.contains("<ruby>"));
            // Default vertical layout wraps short numbers in TCY spans. Check
            // all rendered characters and their order, not a source substring
            // that legitimately crosses inline XHTML tags.
            let mut in_tag = false;
            let visible = fragment.chars().filter(|character| match character {
                '<' => { in_tag = true; false }
                '>' => { in_tag = false; false }
                _ => !in_tag,
            }).collect::<String>();
            let lines = visible.lines().map(str::trim)
                .filter(|line| !line.is_empty()).collect::<Vec<_>>();
            assert_eq!(lines, [heading.as_str(), "本文ほんぶん"]);
            emitted += 1;
        }).unwrap();
        assert_eq!(emitted, index + 1);
    }
    let chapters = stream.finish(&mut |_| panic!("extra empty section")).unwrap();
    assert!(chapters.len() <= 2000);
    assert!(stream.push_line("after EOF", &mut |_| {}).is_err());
    assert!(stream.finish(&mut |_| {}).is_err());
}

#[test]
fn comment_heading_and_empty_line_state_survive_input_calls() {
    let mut config = AozoraConfig::default();
    config.comment_print = false;
    config.remove_empty_line = 1;
    config.max_empty_line = 2;
    let input = "前半\n--------------------------------------------------\n隠す\n--------------------------------------------------\n\n\n後半\n［＃改ページ］\n末尾";
    let mut stream = SectionStream::new(&config, true);
    let mut sections = Vec::new();
    for line in input.lines() {
        stream.push_line(line, &mut |section| sections.push(section)).unwrap();
    }
    stream.finish(&mut |section| sections.push(section)).unwrap();
    assert_eq!(sections.len(), 2);
    assert!(sections[0].contains("前半"));
    assert!(sections[0].contains("後半"));
    assert!(!sections[0].contains("隠す"));
    assert!(sections[1].contains("末尾"));
}

#[test]
fn metadata_scanner_preserves_offsets_gaiji_and_all_title_layouts() {
    let input = "\n題名※［＃丸印］\n\n著者\n--------------------------------------------------\n本文\n";
    let mut gaiji = std::collections::BTreeMap::new();
    gaiji.insert("丸印".to_owned(), "○".to_owned());
    for title_type in TitleType::ALL {
        let expected = aozora_epub3_lite::detect_meta_with_gaiji(input, title_type, false, &gaiji);
        let mut scanner = MetadataScanner::new(title_type, false);
        for line in input.lines() { scanner.push_line(line, &gaiji); }
        assert!(scanner.is_complete());
        for _ in 0..10000 { scanner.push_line("本文は保持しない", &gaiji); }
        assert_eq!(scanner.finish(), expected);
    }
}

fn animated_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 3, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_animated(3, 4).unwrap();
        let mut writer = encoder.write_header().unwrap();
        for frame in 0..3u8 {
            writer.set_frame_delay(u16::from(frame) + 1, 20).unwrap();
            writer.set_blend_op(png::BlendOp::Source).unwrap();
            let image = image::RgbaImage::from_fn(3, 2, |x, y| image::Rgba([frame * 70 + x as u8, y as u8 * 90, 80, if x == 0 { 0 } else { 128 }]));
            writer.write_image_data(image.as_raw()).unwrap();
        }
        writer.finish().unwrap();
    }
    bytes
}

#[test]
fn native_apng_transform_preserves_all_frames_alpha_delays_and_loops() {
    let source = animated_png();
    let ini = IniSettings::parse("Gamma=1\nGammaValue=1.5\n").unwrap();
    let output = aozora_epub3_lite::image::process(&source, "image/png", &ini, false, 90).unwrap();
    let decoder = image::codecs::png::PngDecoder::new(Cursor::new(&output)).unwrap().apng().unwrap();
    assert!(matches!(decoder.loop_count(), image::metadata::LoopCount::Finite(count) if count.get() == 4));
    let source_frames = image::codecs::png::PngDecoder::new(Cursor::new(&source)).unwrap().apng().unwrap().into_frames();
    let mut frames = decoder.into_frames();
    let mut count = 0;
    for source_frame in source_frames {
        let source_frame = source_frame.unwrap();
        let frame = frames.next().unwrap().unwrap();
        assert_eq!(frame.delay(), source_frame.delay());
        let expected = aozora_epub3_lite::image::transform_frame(DynamicImage::ImageRgba8(source_frame.into_buffer()), &ini, false, 90).unwrap().into_rgba8();
        assert_eq!(frame.buffer(), &expected);
        assert_eq!(frame.buffer().dimensions(), (2, 3));
        count += 1;
    }
    assert_eq!(count, 3);
    assert!(frames.next().is_none());
}

#[test]
fn resize_that_already_fits_keeps_animation_bytes_unchanged() {
    let source = animated_png();
    let ini = IniSettings::parse("ResizeW=1\nResizeNumW=100\n").unwrap();
    assert!(!aozora_epub3_lite::image::needs_pixel_transform(&ini, false, 0, (3, 2)));
    assert_eq!(aozora_epub3_lite::image::process(&source, "image/png", &ini, false, 0).unwrap(), source);
}

#[test]
fn frame_transform_applies_real_resize_and_preserves_alpha() {
    let mut encoded = Vec::new();
    let pixels = image::RgbaImage::from_pixel(8, 4, image::Rgba([120, 60, 30, 128]));
    image::codecs::png::PngEncoder::new(&mut encoded).write_image(pixels.as_raw(), 8, 4, image::ExtendedColorType::Rgba8).unwrap();
    let ini = IniSettings::parse("ResizeW=1\nResizeNumW=4\nGamma=1\nGammaValue=2\n").unwrap();
    let output = aozora_epub3_lite::image::process(&encoded, "image/png", &ini, false, 0).unwrap();
    let output = image::load_from_memory(&output).unwrap().into_rgba8();
    assert_eq!(output.dimensions(), (4, 2));
    assert_eq!(output.get_pixel(0, 0)[3], 128);
    assert_ne!(output.get_pixel(0, 0)[0], 120);
}

#[test]
fn rendering_embedded_assets_transfers_the_original_allocation() {
    let font = vec![7u8; 1024 * 1024];
    let allocation = font.as_ptr();
    let book = EpubBook::new(EpubMetadata::new("Font", "urn:test:font-ownership"), "<p>Body</p>")
        .with_assets([EpubAsset::new("gaiji/font.ttf", "font/ttf", font)]);
    let mut writer = book.stream_writer(io::sink()).unwrap();
    let mut transferred = false;
    while let Some(entry) = writer.next_entry() {
        let bytes = writer.render_current(None).unwrap();
        if entry.name == "item/gaiji/font.ttf" {
            assert_eq!(bytes.as_ptr(), allocation);
            transferred = true;
        }
        writer.begin_current().unwrap();
        for chunk in bytes.chunks(64 * 1024) {
            writer.write_chunk(chunk).unwrap();
        }
        writer.end_current().unwrap();
    }
    writer.finish().unwrap();
    assert!(transferred);
}

#[test]
fn owned_row_stream_retains_rgb_source_without_expanding_the_canvas() {
    let source = std::sync::Arc::new(DynamicImage::ImageRgb8(
        image::RgbImage::from_fn(8, 4, |x, y| image::Rgb([(x * 10) as u8, (y * 20) as u8, 35])),
    ));
    let plan = aozora_epub3_lite::image::AnimationTransform::from_dimensions(
        &IniSettings::default(), false, 90, (8, 4),
    ).unwrap();
    let mut rows = plan.rgba_rows_owned(source.clone()).unwrap();
    assert_eq!(std::sync::Arc::strong_count(&source), 2);
    assert_eq!(rows.dimensions(), (4, 8));
    assert_eq!(rows.next_row().unwrap(), &[0, 60, 35, 255, 0, 40, 35, 255, 0, 20, 35, 255, 0, 0, 35, 255]);
    assert!(source.as_rgb8().is_some());
    drop(rows);
    assert_eq!(std::sync::Arc::strong_count(&source), 1);
}
