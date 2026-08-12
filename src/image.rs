use crate::config::IniSettings;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::{
    DynamicImage, ExtendedColorType, GenericImageView, ImageEncoder, ImageFormat, ImageReader,
};
use std::fmt;
use std::io::Cursor;

#[derive(Debug)]
pub enum ImageError {
    UnsupportedFormat(String),
    Decode(String),
    Encode(String),
    InvalidDimensions,
}

impl fmt::Display for ImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormat(format) => {
                write!(formatter, "unsupported image format: {format}")
            }
            Self::Decode(message) => write!(formatter, "image decode failed: {message}"),
            Self::Encode(message) => write!(formatter, "image encode failed: {message}"),
            Self::InvalidDimensions => formatter.write_str("image has invalid dimensions"),
        }
    }
}

impl std::error::Error for ImageError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Margins {
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
}

#[derive(Clone, Copy, Debug)]
struct ImageOptions {
    display_width: u32,
    display_height: u32,
    max_width: u32,
    max_height: u32,
    rotate: i32,
    jpeg_quality: u8,
    gamma: Option<f32>,
    auto_margin_limit_h: u32,
    auto_margin_limit_v: u32,
    auto_margin_white_level: u32,
    auto_margin_padding: f32,
    auto_margin_nombre: u32,
    auto_margin_nombre_size: f32,
}

impl ImageOptions {
    fn from_ini(ini: &IniSettings, cover: bool) -> Self {
        let display_width = get_u32(ini, "DispW", 600);
        let display_height = get_u32(ini, "DispH", 800);
        let max_width = if cover {
            get_u32(ini, "CoverW", 600)
        } else if ini.get_bool("ResizeW").unwrap_or(false) {
            get_u32(ini, "ResizeNumW", 0)
        } else {
            0
        };
        let max_height = if cover {
            get_u32(ini, "CoverH", 800)
        } else if ini.get_bool("ResizeH").unwrap_or(false) {
            get_u32(ini, "ResizeNumH", 0)
        } else {
            0
        };
        let rotate = if cover {
            0
        } else {
            match ini.get("RotateImage").map(str::trim) {
                Some("1") => 90,
                Some("2") => -90,
                _ => 0,
            }
        };
        let jpeg_quality = get_u32(ini, "JpegQuality", 80).clamp(1, 100) as u8;
        let gamma = if ini.get_bool("Gamma").unwrap_or(false) {
            ini.get("GammaValue")
                .and_then(|value| value.trim().parse::<f32>().ok())
                .filter(|value| value.is_finite() && *value > 0.0 && *value != 1.0)
        } else {
            None
        };
        let auto_margin_enabled = !cover && ini.get_bool("AutoMargin").unwrap_or(false);
        Self {
            display_width,
            display_height,
            max_width,
            max_height,
            rotate,
            jpeg_quality,
            gamma,
            auto_margin_limit_h: if auto_margin_enabled {
                get_u32(ini, "AutoMarginLimitH", 0)
            } else {
                0
            },
            auto_margin_limit_v: if auto_margin_enabled {
                get_u32(ini, "AutoMarginLimitV", 0)
            } else {
                0
            },
            auto_margin_white_level: if auto_margin_enabled {
                get_u32(ini, "AutoMarginWhiteLevel", 100)
            } else {
                100
            },
            auto_margin_padding: if auto_margin_enabled {
                get_f32(ini, "AutoMarginPadding", 0.0) / 100.0
            } else {
                0.0
            },
            auto_margin_nombre: if auto_margin_enabled {
                get_u32(ini, "AutoMarginNombre", 0)
            } else {
                0
            },
            auto_margin_nombre_size: if auto_margin_enabled {
                get_f32(ini, "AutoMarginNombreSize", 3.0) / 100.0
            } else {
                0.03
            },
        }
    }

    fn changes_pixels(self) -> bool {
        self.max_width > 0
            || self.max_height > 0
            || self.rotate != 0
            || self.gamma.is_some()
            || self.auto_margin_limit_h > 0
            || self.auto_margin_limit_v > 0
    }
}

fn get_u32(ini: &IniSettings, key: &str, default: u32) -> u32 {
    ini.get(key)
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(default)
}

fn get_f32(ini: &IniSettings, key: &str, default: f32) -> f32 {
    ini.get(key)
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .unwrap_or(default)
}

fn image_format(media_type: &str) -> Option<ImageFormat> {
    match media_type {
        "image/png" => Some(ImageFormat::Png),
        "image/jpeg" => Some(ImageFormat::Jpeg),
        "image/gif" => Some(ImageFormat::Gif),
        "image/bmp" => Some(ImageFormat::Bmp),
        "image/webp" => Some(ImageFormat::WebP),
        _ => None,
    }
}

pub fn dimensions(data: &[u8], media_type: &str) -> Option<(u32, u32)> {
    let format = image_format(media_type)?;
    let reader = ImageReader::with_format(Cursor::new(data), format);
    reader.into_dimensions().ok()
}

pub fn process(
    data: &[u8],
    media_type: &str,
    ini: &IniSettings,
    cover: bool,
) -> Result<Vec<u8>, ImageError> {
    let format = image_format(media_type)
        .ok_or_else(|| ImageError::UnsupportedFormat(media_type.to_owned()))?;
    let options = ImageOptions::from_ini(ini, cover);
    if !options.changes_pixels() {
        return Ok(data.to_vec());
    }
    let source = image::load_from_memory_with_format(data, format)
        .map_err(|error| ImageError::Decode(error.to_string()))?;
    let (source_width, source_height) = source.dimensions();
    if source_width == 0 || source_height == 0 {
        return Err(ImageError::InvalidDimensions);
    }

    let margins = if options.auto_margin_limit_h > 0 || options.auto_margin_limit_v > 0 {
        plain_margin(&source, options)
    } else {
        None
    };
    let margins = margins.map(|mut margins| {
        adjust_margin_for_display(
            &mut margins,
            source_width,
            source_height,
            options.display_width,
            options.display_height,
        );
        margins
    });
    let has_margins = margins.is_some();
    let (mut image, width, height) = if let Some(margins) = margins {
        let width = source_width.saturating_sub(margins.left.saturating_add(margins.right));
        let height = source_height.saturating_sub(margins.top.saturating_add(margins.bottom));
        if width == 0 || height == 0 {
            (source, source_width, source_height)
        } else {
            (
                source.crop_imm(margins.left, margins.top, width, height),
                width,
                height,
            )
        }
    } else {
        (source, source_width, source_height)
    };

    let mut scale = 1.0_f64;
    if options.max_width > 0 {
        scale = scale.min(options.max_width as f64 / width as f64);
    }
    if options.max_height > 0 {
        scale = scale.min(options.max_height as f64 / height as f64);
    }
    if !scale.is_finite() || scale <= 0.0 {
        return Err(ImageError::InvalidDimensions);
    }
    if scale >= 1.0 && !has_margins && options.rotate == 0 && options.gamma.is_none() {
        return Ok(data.to_vec());
    }
    if options.rotate == 90 {
        image = image::DynamicImage::ImageRgba8(imageops::rotate90(&image));
    } else if options.rotate == -90 {
        image = image::DynamicImage::ImageRgba8(imageops::rotate270(&image));
    }
    if scale < 1.0 {
        let scaled_width = ((width as f64 * scale) + 0.5).floor().max(1.0) as u32;
        let scaled_height = ((height as f64 * scale) + 0.5).floor().max(1.0) as u32;
        let (scaled_width, scaled_height) = if options.rotate != 0 {
            (scaled_height, scaled_width)
        } else {
            (scaled_width, scaled_height)
        };
        image = image.resize_exact(scaled_width, scaled_height, FilterType::CatmullRom);
    }
    if let Some(gamma) = options.gamma {
        apply_gamma(&mut image, gamma);
    }
    encode(&image, format, options.jpeg_quality)
}

fn encode(
    image: &DynamicImage,
    format: ImageFormat,
    jpeg_quality: u8,
) -> Result<Vec<u8>, ImageError> {
    let mut output = Cursor::new(Vec::new());
    if format == ImageFormat::Jpeg {
        let rgb = image.to_rgb8();
        JpegEncoder::new_with_quality(&mut output, jpeg_quality)
            .write_image(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                ExtendedColorType::Rgb8,
            )
            .map_err(|error| ImageError::Encode(error.to_string()))?;
    } else {
        image
            .write_to(&mut output, format)
            .map_err(|error| ImageError::Encode(error.to_string()))?;
    }
    Ok(output.into_inner())
}

fn apply_gamma(image: &mut DynamicImage, gamma: f32) {
    let exponent = 1.0 / gamma;
    let mut rgba = image.to_rgba8();
    for pixel in rgba.pixels_mut() {
        for channel in &mut pixel.0[..3] {
            let value = f32::from(*channel) / 255.0;
            *channel = (255.0 * value.powf(exponent)).round().clamp(0.0, 255.0) as u8;
        }
    }
    *image = DynamicImage::ImageRgba8(rgba);
}

fn plain_margin(image: &DynamicImage, options: ImageOptions) -> Option<Margins> {
    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return None;
    }
    let start_pixel = (width as f32 * 0.01) as u32;
    let ignore_edge = (width as f32 * 0.03) as u32;
    let dust_size = (width as f32 * 0.01) as u32;
    let white_level = (256.0 * (options.auto_margin_white_level as f32 / 100.0)) as u32;
    let padding_h = (width as f32 * options.auto_margin_padding).max(1.0) as u32;
    let padding_v = (height as f32 * options.auto_margin_padding).max(1.0) as u32;
    let limit_h = (width as f32 * options.auto_margin_limit_h as f32 / 100.0) as u32;
    let limit_v = (height as f32 * options.auto_margin_limit_v as f32 / 100.0) as u32 / 2;
    let mut limit_top = limit_v;
    let mut limit_bottom = limit_v;
    if matches!(options.auto_margin_nombre, 1 | 3) {
        limit_top = limit_top.saturating_add((height as f32 * 0.05) as u32);
    }
    if matches!(options.auto_margin_nombre, 2 | 3) {
        limit_bottom = limit_bottom.saturating_add((height as f32 * 0.05) as u32);
    }
    let mut margin = Margins {
        left: 0,
        top: scan_top(
            image,
            start_pixel,
            white_level,
            ignore_edge,
            dust_size,
            limit_top,
        ),
        right: 0,
        bottom: scan_bottom(
            image,
            start_pixel,
            white_level,
            ignore_edge,
            dust_size,
            limit_bottom,
        ),
    };

    let has_nombre_top = if matches!(options.auto_margin_nombre, 1 | 3) {
        remove_nombre_top(
            image,
            &mut margin,
            white_level,
            ignore_edge,
            limit_top,
            options.auto_margin_nombre_size,
        )
    } else {
        false
    };
    let has_nombre_bottom = if matches!(options.auto_margin_nombre, 2 | 3) {
        remove_nombre_bottom(
            image,
            &mut margin,
            white_level,
            ignore_edge,
            limit_bottom,
            options.auto_margin_nombre_size,
        )
    } else {
        false
    };
    let ignore_top = ignore_edge.max(margin.top);
    let ignore_bottom = ignore_edge.max(margin.bottom);
    margin.left = scan_left(
        image,
        start_pixel,
        white_level,
        ignore_top,
        ignore_bottom,
        dust_size,
        limit_h,
    );
    margin.right = scan_right(
        image,
        start_pixel,
        white_level,
        ignore_top,
        ignore_bottom,
        dust_size,
        limit_h,
    );
    if margin.left.saturating_add(margin.right) > limit_h {
        let rate = limit_h as f64 / margin.left.saturating_add(margin.right) as f64;
        margin.left = (margin.left as f64 * rate) as u32;
        margin.right = (margin.right as f64 * rate) as u32;
    }
    if !has_nombre_top {
        margin.top = margin.top.min(limit_v);
    }
    if !has_nombre_bottom {
        margin.bottom = margin.bottom.min(limit_v);
    }
    margin.left = margin.left.saturating_sub(padding_h);
    margin.top = margin.top.saturating_sub(padding_v);
    margin.right = margin.right.saturating_sub(padding_h);
    margin.bottom = margin.bottom.saturating_sub(padding_v);
    (margin
        != Margins {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        })
    .then_some(margin)
}
fn adjust_margin_for_display(
    margin: &mut Margins,
    width: u32,
    height: u32,
    display_width: u32,
    display_height: u32,
) {
    if display_width == 0 || display_height == 0 {
        return;
    }
    let cropped_width = width.saturating_sub(margin.left.saturating_add(margin.right));
    let cropped_height = height.saturating_sub(margin.top.saturating_add(margin.bottom));
    if cropped_width == 0 || cropped_height == 0 {
        return;
    }
    let display_ratio = display_width as f64 / display_height as f64;
    let cropped_ratio = cropped_width as f64 / cropped_height as f64;
    if width as f64 / (height as f64) < display_ratio {
        if cropped_ratio > display_ratio && cropped_width > display_width {
            let target_height = (cropped_width as f64 / display_ratio) as u32;
            let adjusted_bottom = height as i64 - margin.top as i64 - target_height as i64;
            if adjusted_bottom < 0 {
                margin.bottom = 0;
                margin.top = height.saturating_sub(target_height);
            } else {
                margin.bottom = adjusted_bottom as u32;
            }
        }
    } else if cropped_ratio < display_ratio && cropped_height > display_height {
        let target_width = (cropped_height as f64 * display_ratio) as u32;
        let total = margin.left.saturating_add(margin.right);
        if total > 0 {
            margin.left = ((width.saturating_sub(target_width)) as f64 * margin.left as f64
                / total as f64) as u32;
            margin.right = ((width.saturating_sub(target_width)) as f64 * margin.right as f64
                / total as f64) as u32;
        }
    }
}

fn scan_top(image: &DynamicImage, start: u32, limit: u32, ignore: u32, dust: u32, max: u32) -> u32 {
    let start = start.min(image.height().saturating_sub(1));
    if colored_h(image, start, limit, ignore, ignore, dust) > 0 {
        let mut margin = 0;
        for index in (0..start).rev() {
            margin = index;
            if colored_h(image, index, limit, ignore, ignore, 0) == 0 {
                break;
            }
        }
        margin
    } else {
        let margin = start;
        for index in (start + 1)..=max.min(image.height().saturating_sub(1)) {
            if colored_h(image, index, limit, ignore, ignore, dust) == 0 {
            } else {
                break;
            }
        }
        margin
    }
}
fn scan_bottom(
    image: &DynamicImage,
    start: u32,
    limit: u32,
    ignore: u32,
    dust: u32,
    max: u32,
) -> u32 {
    let start = start.min(image.height().saturating_sub(1));
    let row = image.height() - 1 - start;
    if colored_h(image, row, limit, ignore, ignore, dust) > 0 {
        let mut margin = 0;
        for index in (0..start).rev() {
            margin = index;
            if colored_h(image, image.height() - 1 - index, limit, ignore, ignore, 0) == 0 {
                break;
            }
        }
        margin
    } else {
        let mut margin = start;
        for index in (start + 1)..=max.min(image.height().saturating_sub(1)) {
            if colored_h(
                image,
                image.height() - 1 - index,
                limit,
                ignore,
                ignore,
                dust,
            ) == 0
            {
                margin = index;
            } else {
                break;
            }
        }
        margin
    }
}

fn scan_left(
    image: &DynamicImage,
    start: u32,
    limit: u32,
    ignore_top: u32,
    ignore_bottom: u32,
    dust: u32,
    max: u32,
) -> u32 {
    let start = start.min(image.width().saturating_sub(1));
    if colored_v(image, start, limit, ignore_top, ignore_bottom, dust) > 0 {
        let mut margin = 0;
        for index in (0..start).rev() {
            margin = index;
            if colored_v(image, index, limit, ignore_top, ignore_bottom, 0) == 0 {
                break;
            }
        }
        margin
    } else {
        let mut margin = start;
        for index in (start + 1)..=max.min(image.width().saturating_sub(1)) {
            if colored_v(image, index, limit, ignore_top, ignore_bottom, dust) == 0 {
                margin = index;
            } else {
                break;
            }
        }
        margin
    }
}

fn scan_right(
    image: &DynamicImage,
    start: u32,
    limit: u32,
    ignore_top: u32,
    ignore_bottom: u32,
    dust: u32,
    max: u32,
) -> u32 {
    let start = start.min(image.width().saturating_sub(1));
    let column = image.width() - 1 - start;
    if colored_v(image, column, limit, ignore_top, ignore_bottom, dust) > 0 {
        let mut margin = 0;
        for index in (0..start).rev() {
            margin = index;
            if colored_v(
                image,
                image.width() - 1 - index,
                limit,
                ignore_top,
                ignore_bottom,
                0,
            ) == 0
            {
                break;
            }
        }
        margin
    } else {
        let mut margin = start;
        for index in (start + 1)..=max.min(image.width().saturating_sub(1)) {
            // Java passes the octal literal 05 here; preserve that behavior.
            if colored_v(
                image,
                image.width() - 1 - index,
                limit,
                ignore_top,
                ignore_bottom,
                dust,
            )
            .min(5)
                == 0
            {
                margin = index;
            } else {
                break;
            }
        }
        margin
    }
}

fn colored_h(
    image: &DynamicImage,
    row: u32,
    limit: u32,
    ignore_left: u32,
    ignore_right: u32,
    dust: u32,
) -> u32 {
    let left = ignore_left.min(image.width());
    let right = image
        .width()
        .saturating_sub(ignore_right.min(image.width()));
    let mut colored = 0;
    for x in (left..right).rev() {
        if is_colored(image, x, row, limit) && (dust < 4 || !is_dust(image, x, row, dust, limit)) {
            colored += 1;
        }
    }
    colored
}

fn colored_v(
    image: &DynamicImage,
    column: u32,
    limit: u32,
    ignore_top: u32,
    ignore_bottom: u32,
    dust: u32,
) -> u32 {
    let top = ignore_top.min(image.height());
    let bottom = image
        .height()
        .saturating_sub(ignore_bottom.min(image.height()));
    let mut colored = 0;
    for y in (top..bottom).rev() {
        if is_colored(image, column, y, limit)
            && (dust < 4 || !is_dust(image, column, y, dust, limit))
        {
            colored += 1;
        }
    }
    colored
}

fn is_colored(image: &DynamicImage, x: u32, y: u32, limit: u32) -> bool {
    let pixel = image.get_pixel(x.min(image.width() - 1), y.min(image.height() - 1));
    limit > u32::from(pixel[0]) || limit > u32::from(pixel[1]) || limit > u32::from(pixel[2])
}

fn is_dust(image: &DynamicImage, x: u32, y: u32, dust_size: u32, limit: u32) -> bool {
    if dust_size == 0 {
        return false;
    }
    let min_x = x.saturating_sub(dust_size + 1);
    let max_x = (x + dust_size + 1).min(image.width());
    let min_y = y.saturating_sub(dust_size + 1);
    let max_y = (y + dust_size + 1).min(image.height());
    let mut h = 1;
    for yy in (min_y..y).rev() {
        if is_colored(image, x, yy, limit) {
            h += 1;
        } else {
            break;
        }
    }
    for yy in (y + 1)..max_y {
        if is_colored(image, x, yy, limit) {
            h += 1;
        } else {
            break;
        }
    }
    if h > dust_size {
        return false;
    }
    let mut w = 1;
    for xx in (min_x..x).rev() {
        if is_colored(image, xx, y, limit) {
            w += 1;
        } else {
            break;
        }
    }
    for xx in (x + 1)..max_x {
        if is_colored(image, xx, y, limit) {
            w += 1;
        } else {
            break;
        }
    }
    if w > dust_size {
        return false;
    }
    for xx in (min_x..x).rev() {
        let count = (min_y..max_y)
            .filter(|yy| is_colored(image, xx, *yy, limit))
            .count() as u32;
        if count > dust_size {
            return false;
        }
        if count == 0 {
            break;
        }
    }
    for xx in (x + 1)..max_x {
        let count = (min_y..max_y)
            .filter(|yy| is_colored(image, xx, *yy, limit))
            .count() as u32;
        if count > dust_size {
            return false;
        }
        if count == 0 {
            break;
        }
    }
    for yy in (min_y..y).rev() {
        let count = (min_x..max_x)
            .filter(|xx| is_colored(image, *xx, yy, limit))
            .count() as u32;
        if count > dust_size {
            return false;
        }
        if count == 0 {
            break;
        }
    }
    for yy in (y + 1)..max_y {
        let count = (min_x..max_x)
            .filter(|xx| is_colored(image, *xx, yy, limit))
            .count() as u32;
        if count > dust_size {
            return false;
        }
        if count == 0 {
            break;
        }
    }
    true
}

fn remove_nombre_top(
    image: &DynamicImage,
    margin: &mut Margins,
    limit: u32,
    ignore: u32,
    max: u32,
    size: f32,
) -> bool {
    let nombre_limit = (image.height() as f32 * size) as u32 + margin.top;
    let nombre_dust = (image.height() as f32 * 0.005) as u32;
    let dust = (image.width() as f32 * 0.01) as u32;
    let mut nombre_end = 0;
    for index in margin.top.saturating_add(1)..=nombre_limit.min(image.height().saturating_sub(1)) {
        if colored_h(image, index, limit, ignore, ignore, 0) == 0 {
            nombre_end = index;
            if nombre_end - margin.top > nombre_dust {
                break;
            }
        }
    }
    if nombre_end > margin.top + nombre_dust && nombre_end <= nombre_limit {
        let mut white_end = nombre_end;
        for index in nombre_end.saturating_add(1)..=max.min(image.height().saturating_sub(1)) {
            if colored_h(image, index, limit, ignore, ignore, dust) == 0 {
                white_end = index;
            } else if index - nombre_end > nombre_dust {
                break;
            }
        }
        if white_end > nombre_end + (image.height() as f32 * 0.01) as u32 {
            margin.top = white_end;
            return true;
        }
    }
    false
}

fn remove_nombre_bottom(
    image: &DynamicImage,
    margin: &mut Margins,
    limit: u32,
    ignore: u32,
    max: u32,
    size: f32,
) -> bool {
    let nombre_limit = (image.height() as f32 * size) as u32 + margin.bottom;
    let nombre_dust = (image.height() as f32 * 0.005) as u32;
    let dust = (image.width() as f32 * 0.01) as u32;
    let mut nombre_end = 0;
    for index in
        margin.bottom.saturating_add(1)..=nombre_limit.min(image.height().saturating_sub(1))
    {
        if colored_h(image, image.height() - 1 - index, limit, ignore, ignore, 0) == 0 {
            nombre_end = index;
            if nombre_end - margin.bottom > nombre_dust {
                break;
            }
        }
    }
    if nombre_end > margin.bottom + nombre_dust && nombre_end <= nombre_limit {
        let mut white_end = nombre_end;
        for index in nombre_end.saturating_add(1)..=max.min(image.height().saturating_sub(1)) {
            if colored_h(
                image,
                image.height() - 1 - index,
                limit,
                ignore,
                ignore,
                dust,
            ) == 0
            {
                white_end = index;
            } else if index - nombre_end > nombre_dust {
                break;
            }
        }
        if white_end > nombre_end + (image.height() as f32 * 0.01) as u32 {
            margin.bottom = white_end;
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32, fill: [u8; 4]) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba(fill));
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    #[test]
    fn keeps_original_bytes_when_no_pixel_setting_changes() {
        let source = png(8, 6, [255, 255, 255, 255]);
        let ini =
            IniSettings::parse("ResizeW=\nResizeH=\nRotateImage=0\nGamma=\nAutoMargin=\n").unwrap();
        assert_eq!(process(&source, "image/png", &ini, false).unwrap(), source);
    }

    #[test]
    fn resizes_and_rotates_using_java_dimensions() {
        let source = png(8, 4, [0, 0, 0, 255]);
        let ini = IniSettings::parse("ResizeW=1\nResizeNumW=4\nRotateImage=1\n").unwrap();
        let output = process(&source, "image/png", &ini, false).unwrap();
        assert_eq!(dimensions(&output, "image/png"), Some((2, 4)));
    }

    #[test]
    fn applies_java_gamma_table() {
        let source = png(1, 1, [128, 64, 0, 255]);
        let ini = IniSettings::parse("Gamma=1\nGammaValue=2\n").unwrap();
        let output = process(&source, "image/png", &ini, false).unwrap();
        let decoded = image::load_from_memory(&output).unwrap().to_rgba8();
        assert_eq!(decoded.get_pixel(0, 0).0, [181, 128, 0, 255]);
    }
}
