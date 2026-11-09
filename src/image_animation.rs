use super::{AnimationTransform, ImageError, ImageOptions};
use image::codecs::{gif::{GifDecoder, GifEncoder, Repeat}, png::PngDecoder, webp::{WebPDecoder, WebPEncoder}};
use image::{AnimationDecoder, DynamicImage, ExtendedColorType, Frame, Frames, ImageEncoder, ImageFormat};
use image::metadata::LoopCount;
use std::io::Cursor;

fn decode(error: impl std::fmt::Display) -> ImageError {
    ImageError::Decode(error.to_string())
}
fn encode(error: impl std::fmt::Display) -> ImageError {
    ImageError::Encode(error.to_string())
}

fn frames(data: &[u8], format: ImageFormat) -> Result<Option<(Frames<'_>, LoopCount)>, ImageError> {
    let input = Cursor::new(data);
    match format {
        ImageFormat::Png => {
            let decoder = PngDecoder::new(input).map_err(decode)?;
            if !decoder.is_apng().map_err(decode)? {
                return Ok(None);
            }
            let decoder = decoder.apng().map_err(decode)?;
            let loops = decoder.loop_count();
            Ok(Some((decoder.into_frames(), loops)))
        }
        ImageFormat::Gif => {
            let decoder = GifDecoder::new(input).map_err(decode)?;
            let loops = decoder.loop_count();
            Ok(Some((decoder.into_frames(), loops)))
        }
        ImageFormat::WebP => {
            let decoder = WebPDecoder::new(input).map_err(decode)?;
            if !decoder.has_animation() {
                return Ok(None);
            }
            let loops = decoder.loop_count();
            Ok(Some((decoder.into_frames(), loops)))
        }
        _ => Ok(None),
    }
}

// A native whole-value boundary: the result is deliberately a complete encoded
// image. Only auto-margin needs a pixel analysis pass; ordinary transforms
// decode each composited frame once.
pub(super) fn process(data: &[u8], format: ImageFormat, options: ImageOptions) -> Result<Option<Vec<u8>>, ImageError> {
    let Some((mut input, loops)) = frames(data, format)? else {
        return Ok(None);
    };
    let mut plan = AnimationTransform::with_options(options);
    let (count, separate_default) = if format == ImageFormat::Png {
        png_animation_info(data)?
    } else {
        (0, false)
    };
    let analyze_margins = options.auto_margin_limit_h > 0 || options.auto_margin_limit_v > 0;
    if analyze_margins {
        if separate_default {
            let fallback = image::load_from_memory_with_format(data, ImageFormat::Png).map_err(decode)?;
            plan.observe(&fallback)?;
        }
        let mut observed = 0u32;
        for frame in input {
            let frame = frame.map_err(decode)?;
            plan.observe(&DynamicImage::ImageRgba8(frame.into_buffer()))?;
            observed = observed.checked_add(1).ok_or(ImageError::InvalidDimensions)?;
        }
        if observed == 0 || (format == ImageFormat::Png && observed != count) {
            return Err(decode("animation frame count does not match its header"));
        }
        input = frames(data, format)?.ok_or_else(|| decode("animation disappeared"))?.0;
    }
    let first = input.next().ok_or_else(|| decode("animation contains no frames"))?.map_err(decode)?;
    let delay = first.delay();
    let source = DynamicImage::ImageRgba8(first.into_buffer());
    if !analyze_margins {
        plan.observe(&source)?;
    }
    let first = Frame::from_parts(plan.apply(source)?.into_rgba8(), 0, 0, delay);
    let width = first.buffer().width();
    let height = first.buffer().height();
    let mut output = Vec::new();
    match format {
        ImageFormat::Png => {
            let mut encoder = png::Encoder::new(&mut output, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_animated(count, loop_number(loops)).map_err(encode)?;
            encoder.set_sep_def_img(separate_default).map_err(encode)?;
            let mut writer = encoder.write_header().map_err(encode)?;
            if separate_default {
                let fallback = image::load_from_memory_with_format(data, ImageFormat::Png).map_err(decode)?;
                let fallback = plan.apply(fallback)?.into_rgba8();
                writer.write_image_data(fallback.as_raw()).map_err(encode)?;
            }
            for frame in std::iter::once(Ok(first)).chain(input.map(|frame| transform(frame.map_err(decode)?, &plan))) {
                let frame = frame?;
                let (numerator, denominator) = png_delay(frame.delay())?;
                writer.set_frame_delay(numerator, denominator).map_err(encode)?;
                // Decoders return composited canvases. Source+None preserves the
                // result of every original blend/disposal operation exactly.
                writer.set_blend_op(png::BlendOp::Source).map_err(encode)?;
                writer.set_dispose_op(png::DisposeOp::None).map_err(encode)?;
                writer.write_image_data(frame.buffer().as_raw()).map_err(encode)?;
            }
            writer.finish().map_err(encode)?;
        }
        ImageFormat::Gif => {
            let mut writer = GifEncoder::new(&mut output);
            let repeat = match loops {
                LoopCount::Infinite => Repeat::Infinite,
                LoopCount::Finite(value) => Repeat::Finite(u16::try_from(value.get()).map_err(encode)?),
            };
            writer.set_repeat(repeat).map_err(encode)?;
            writer.encode_frame(first).map_err(encode)?;
            for frame in input {
                writer.encode_frame(transform(frame.map_err(decode)?, &plan)?).map_err(encode)?;
            }
        }
        ImageFormat::WebP => {
            output.extend_from_slice(b"RIFF\0\0\0\0WEBP");
            let mut extended = [0u8; 10];
            extended[0] = 0x12; // animation and alpha
            put_u24(&mut extended[4..7], width.checked_sub(1).ok_or(ImageError::InvalidDimensions)?)?;
            put_u24(&mut extended[7..10], height.checked_sub(1).ok_or(ImageError::InvalidDimensions)?)?;
            riff_chunk(&mut output, b"VP8X", &extended)?;
            let mut animation = [0u8; 6];
            let loops = u16::try_from(loop_number(loops)).map_err(encode)?;
            animation[4..6].copy_from_slice(&loops.to_le_bytes());
            riff_chunk(&mut output, b"ANIM", &animation)?;
            webp_frame(&mut output, first)?;
            for frame in input {
                webp_frame(&mut output, transform(frame.map_err(decode)?, &plan)?)?;
            }
            let length = u32::try_from(output.len().checked_sub(8).ok_or(ImageError::InvalidDimensions)?).map_err(encode)?;
            output[4..8].copy_from_slice(&length.to_le_bytes());
        }
        _ => unreachable!("frames only exposes animated formats"),
    }
    Ok(Some(output))
}

fn transform(frame: Frame, plan: &AnimationTransform) -> Result<Frame, ImageError> {
    let delay = frame.delay();
    let image = plan.apply(DynamicImage::ImageRgba8(frame.into_buffer()))?.into_rgba8();
    Ok(Frame::from_parts(image, 0, 0, delay))
}

fn loop_number(loops: LoopCount) -> u32 {
    match loops {
        LoopCount::Infinite => 0,
        LoopCount::Finite(value) => value.get(),
    }
}

fn png_delay(delay: image::Delay) -> Result<(u16, u16), ImageError> {
    let (numerator, denominator) = delay.numer_denom_ms();
    let numerator = u64::from(numerator);
    let denominator = u64::from(denominator) * 1000;
    let divisor = gcd(numerator, denominator);
    Ok((u16::try_from(numerator / divisor).map_err(encode)?, u16::try_from(denominator / divisor).map_err(encode)?))
}

fn gcd(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left.max(1)
}

fn png_animation_info(data: &[u8]) -> Result<(u32, bool), ImageError> {
    let reader = png::Decoder::new(Cursor::new(data)).read_info().map_err(decode)?;
    let animation = reader.info().animation_control().ok_or_else(|| decode("PNG animation control missing"))?;
    if animation.num_frames == 0 {
        return Err(decode("PNG animation contains no frames"));
    }
    Ok((animation.num_frames, reader.info().frame_control().is_none()))
}

fn put_u24(output: &mut [u8], value: u32) -> Result<(), ImageError> {
    if value > 0x00ff_ffff {
        return Err(ImageError::InvalidDimensions);
    }
    output.copy_from_slice(&value.to_le_bytes()[..3]);
    Ok(())
}

fn riff_chunk(output: &mut Vec<u8>, kind: &[u8; 4], bytes: &[u8]) -> Result<(), ImageError> {
    let length = u32::try_from(bytes.len()).map_err(encode)?;
    let final_length = output.len().checked_add(8).and_then(|length| length.checked_add(bytes.len()))
        .and_then(|length| length.checked_add(bytes.len() % 2)).ok_or(ImageError::InvalidDimensions)?;
    u32::try_from(final_length).map_err(encode)?;
    output.extend_from_slice(kind);
    output.extend_from_slice(&length.to_le_bytes());
    output.extend_from_slice(bytes);
    if bytes.len() % 2 != 0 {
        output.push(0);
    }
    Ok(())
}

fn webp_frame(output: &mut Vec<u8>, frame: Frame) -> Result<(), ImageError> {
    let mut still = Vec::new();
    let image = frame.buffer();
    WebPEncoder::new_lossless(&mut still).write_image(image.as_raw(), image.width(), image.height(), ExtendedColorType::Rgba8).map_err(encode)?;
    if still.get(..4) != Some(b"RIFF") || still.get(8..12) != Some(b"WEBP") {
        return Err(encode("invalid encoded WebP frame"));
    }
    let (numerator, denominator) = frame.delay().numer_denom_ms();
    if denominator == 0 || numerator % denominator != 0 {
        return Err(encode("WebP frame delay is not an integer millisecond"));
    }
    let mut header = [0u8; 16];
    put_u24(&mut header[6..9], image.width().checked_sub(1).ok_or(ImageError::InvalidDimensions)?)?;
    put_u24(&mut header[9..12], image.height().checked_sub(1).ok_or(ImageError::InvalidDimensions)?)?;
    put_u24(&mut header[12..15], numerator / denominator)?;
    header[15] = 0x02; // replace the complete composited canvas, retain for next frame
    let payload_length = 16usize.checked_add(still.len() - 12).ok_or(ImageError::InvalidDimensions)?;
    let payload_length = u32::try_from(payload_length).map_err(encode)?;
    let final_length = output.len().checked_add(8 + payload_length as usize)
        .and_then(|length| length.checked_add(payload_length as usize % 2)).ok_or(ImageError::InvalidDimensions)?;
    u32::try_from(final_length).map_err(encode)?;
    output.extend_from_slice(b"ANMF");
    output.extend_from_slice(&payload_length.to_le_bytes());
    output.extend_from_slice(&header);
    output.extend_from_slice(&still[12..]);
    if payload_length % 2 != 0 {
        output.push(0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::IniSettings;
    use image::{Delay, Rgba, RgbaImage};

    fn frame(color: [u8; 4], milliseconds: u32) -> Frame {
        Frame::from_parts(RgbaImage::from_pixel(4, 2, Rgba(color)), 0, 0,
            Delay::from_numer_denom_ms(milliseconds, 1))
    }

    #[test]
    fn gif_pixel_transforms_keep_every_frame_and_delay() {
        let mut source = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut source);
            encoder.set_repeat(Repeat::Finite(3)).unwrap();
            encoder.encode_frame(frame([255, 0, 0, 255], 70)).unwrap();
            encoder.encode_frame(frame([0, 0, 255, 255], 130)).unwrap();
        }
        let output = super::super::process(&source, "image/gif", &IniSettings::default(), false, 90).unwrap();
        let decoder = GifDecoder::new(Cursor::new(output)).unwrap();
        assert!(matches!(decoder.loop_count(), LoopCount::Finite(value) if value.get() == 3));
        let mut frames = decoder.into_frames();
        for (color, milliseconds) in [([255, 0, 0, 255], 70), ([0, 0, 255, 255], 130)] {
            let frame = frames.next().unwrap().unwrap();
            assert_eq!(frame.buffer().dimensions(), (2, 4));
            assert_eq!(frame.delay(), Delay::from_numer_denom_ms(milliseconds, 1));
            assert_eq!(frame.buffer().get_pixel(0, 0).0, color);
        }
        assert!(frames.next().is_none());
    }

    #[test]
    fn webp_pixel_transforms_keep_alpha_timing_and_loop_count() {
        let mut source = b"RIFF\0\0\0\0WEBP".to_vec();
        let mut extended = [0u8; 10];
        extended[0] = 0x12;
        put_u24(&mut extended[4..7], 3).unwrap();
        put_u24(&mut extended[7..10], 1).unwrap();
        riff_chunk(&mut source, b"VP8X", &extended).unwrap();
        riff_chunk(&mut source, b"ANIM", &[0, 0, 0, 0, 2, 0]).unwrap();
        webp_frame(&mut source, frame([120, 30, 80, 128], 75)).unwrap();
        webp_frame(&mut source, frame([70, 100, 20, 192], 125)).unwrap();
        let size = (source.len() - 8) as u32;
        source[4..8].copy_from_slice(&size.to_le_bytes());
        let output = super::super::process(&source, "image/webp", &IniSettings::default(), false, 90).unwrap();
        let decoder = WebPDecoder::new(Cursor::new(output)).unwrap();
        assert!(decoder.has_animation());
        assert!(matches!(decoder.loop_count(), LoopCount::Finite(value) if value.get() == 2));
        let mut frames = decoder.into_frames();
        for (alpha, milliseconds) in [(128, 75), (192, 125)] {
            let frame = frames.next().unwrap().unwrap();
            assert_eq!(frame.buffer().dimensions(), (2, 4));
            assert_eq!(frame.delay(), Delay::from_numer_denom_ms(milliseconds, 1));
            assert_eq!(frame.buffer().get_pixel(0, 0)[3], alpha);
        }
        assert!(frames.next().is_none());
    }
}
