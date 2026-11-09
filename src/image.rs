use crate::config::IniSettings;
use image::codecs::jpeg::JpegEncoder;
use image::Pixel;
use image::{
    DynamicImage, ExtendedColorType, GenericImageView, ImageEncoder, ImageFormat, ImageReader,
};
use std::fmt;
use std::io::Cursor;

#[path = "image_animation.rs"]
mod animation;
#[path = "image_pixels.rs"]
mod pixels;
pub use pixels::{RgbaRows, RowDepth, SourceRows};

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
    fn from_ini(ini: &IniSettings, cover: bool, rotate: i32) -> Self {
        let display_width = get_u32(ini, "DispW", 600);
        let display_height = get_u32(ini, "DispH", 800);
        let max_width = if cover {
            get_u32(ini, "CoverW", 0)
        } else if ini.get_bool("ResizeW").unwrap_or(false) {
            get_u32(ini, "ResizeNumW", 0)
        } else {
            0
        };
        let max_height = if cover {
            get_u32(ini, "CoverH", 0)
        } else if ini.get_bool("ResizeH").unwrap_or(false) {
            get_u32(ini, "ResizeNumH", 0)
        } else {
            0
        };
        // Java は表紙の rotateAngle を常に 0 にする。適用条件は呼び出し側
        // (main.rs) が Java と同じ判定で決める。
        let rotate = if cover { 0 } else { rotate };
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
            // Java: AutoMargin 無効時は使われない (既定も 80 で揃える)
            auto_margin_white_level: get_u32(ini, "AutoMarginWhiteLevel", 80),
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

/// `rotate` は Java の `imageInfo.rotateAngle` 相当。呼び出し側が
/// `RotateImage` と画像の向き・画面の向きから決める (Java も同じ条件でしか
/// 設定しない)。
pub fn process(
    data: &[u8],
    media_type: &str,
    ini: &IniSettings,
    cover: bool,
    rotate: i32,
) -> Result<Vec<u8>, ImageError> {
    let format = image_format(media_type)
        .ok_or_else(|| ImageError::UnsupportedFormat(media_type.to_owned()))?;
    let options = ImageOptions::from_ini(ini, cover, rotate);
    if !options.changes_pixels() {
        return Ok(data.to_vec());
    }
    if options.rotate == 0 && options.gamma.is_none()
        && options.auto_margin_limit_h == 0 && options.auto_margin_limit_v == 0
        && dimensions(data, media_type).is_some_and(|(width, height)| {
            (options.max_width == 0 || width <= options.max_width)
                && (options.max_height == 0 || height <= options.max_height)
        })
    {
        return Ok(data.to_vec());
    }
    if let Some(output) = animation::process(data, format, options)? {
        return Ok(output);
    }
    let source = image::load_from_memory_with_format(data, format)
        .map_err(|error| ImageError::Decode(error.to_string()))?;
    let (image, changed) = transform(source, options)?;
    if changed {
        encode(&image, format, options.jpeg_quality)
    } else {
        Ok(data.to_vec())
    }
}

/// Whether the configured operation needs pixels. A resize that already fits is
/// a byte-preserving transfer, including every APNG/GIF/WebP frame and ancillary chunk.
pub fn needs_pixel_transform(
    ini: &IniSettings,
    cover: bool,
    rotate: i32,
    dimensions: (u32, u32),
) -> bool {
    let options = ImageOptions::from_ini(ini, cover, rotate);
    options.rotate != 0 || options.gamma.is_some()
        || options.auto_margin_limit_h > 0 || options.auto_margin_limit_v > 0
        || (options.max_width > 0 && dimensions.0 > options.max_width)
        || (options.max_height > 0 && dimensions.1 > options.max_height)
}

/// Transforms one composited frame. Ownership is transferred to avoid a decoded
/// pixel copy; animation consumers retain timing/disposal metadata themselves.
pub fn transform_frame(
    source: DynamicImage,
    ini: &IniSettings,
    cover: bool,
    rotate: i32,
) -> Result<DynamicImage, ImageError> {
    transform(source, ImageOptions::from_ini(ini, cover, rotate)).map(|(image, _)| image)
}

/// Metadata-only geometry pass for an animation. Auto-margin cropping uses the
/// intersection of safe margins across frames, not independent moving canvases.
pub struct AnimationTransform {
    options: ImageOptions,
    dimensions: Option<(u32, u32)>,
    margins: Option<Margins>,
    frames: u32,
}

impl AnimationTransform {
    pub fn new(ini: &IniSettings, cover: bool, rotate: i32) -> Self {
        Self::with_options(ImageOptions::from_ini(ini, cover, rotate))
    }

    fn with_options(options: ImageOptions) -> Self {
        Self { options, dimensions: None, margins: None, frames: 0 }
    }

    /// Initializes a fixed-canvas transform without decoding or copying pixels.
    /// Auto-margin requires `observe` instead, because its geometry depends on content.
    pub fn from_dimensions(
        ini: &IniSettings, cover: bool, rotate: i32, dimensions: (u32, u32),
    ) -> Result<Self, ImageError> {
        let options = ImageOptions::from_ini(ini, cover, rotate);
        if dimensions.0 == 0 || dimensions.1 == 0 {
            return Err(ImageError::InvalidDimensions);
        }
        if options.auto_margin_limit_h > 0 || options.auto_margin_limit_v > 0 {
            return Err(ImageError::Decode("automatic margins require a frame geometry pass".to_owned()));
        }
        Ok(Self { options, dimensions: Some(dimensions), margins: None, frames: 0 })
    }

    pub fn observe(&mut self, source: &DynamicImage) -> Result<(), ImageError> {
        let dimensions = source.dimensions();
        if dimensions.0 == 0 || dimensions.1 == 0
            || self.dimensions.is_some_and(|expected| expected != dimensions) {
            return Err(ImageError::InvalidDimensions);
        }
        let margins = frame_margins(source, self.options);
        self.margins = if self.frames == 0 {
            margins
        } else {
            self.margins.zip(margins).map(|(left, right)| Margins {
                left: left.left.min(right.left),
                top: left.top.min(right.top),
                right: left.right.min(right.right),
                bottom: left.bottom.min(right.bottom),
            })
        };
        self.dimensions = Some(dimensions);
        self.frames = self.frames.checked_add(1).ok_or(ImageError::InvalidDimensions)?;
        Ok(())
    }

    pub fn frame_count(&self) -> u32 {
        self.frames
    }

    pub fn apply(&self, source: DynamicImage) -> Result<DynamicImage, ImageError> {
        if self.dimensions != Some(source.dimensions()) {
            return Err(ImageError::InvalidDimensions);
        }
        transform_with_margins(source, self.options, self.margins).map(|(image, _)| image)
    }

    /// Samples a retained compositing canvas without cloning it. Only the target
    /// image and row-sized Catmull-Rom scratch are allocated.
    pub fn apply_ref(&self, source: &DynamicImage) -> Result<DynamicImage, ImageError> {
        if self.dimensions != Some(source.dimensions()) {
            return Err(ImageError::InvalidDimensions);
        }
        let geometry = TransformGeometry::new(source.dimensions(), self.options, self.margins)?;
        let mut output = pixels::borrowed(source, geometry)?;
        if let Some(gamma) = self.options.gamma {
            apply_gamma(&mut output, gamma)?;
        }
        Ok(output)
    }

    /// Streams transformed RGBA8 rows while preserving the caller's compositing
    /// canvas. Pull and encode one row before requesting the next.
    pub fn rgba_rows<'a>(&self, source: &'a image::RgbaImage) -> Result<RgbaRows<'a>, ImageError> {
        if self.dimensions != Some(source.dimensions()) {
            return Err(ImageError::InvalidDimensions);
        }
        let geometry = TransformGeometry::new(source.dimensions(), self.options, self.margins)?;
        RgbaRows::new(source, geometry, self.options.gamma)
    }

    /// Owned-source variant for asynchronous state machines. RGB/gray sources
    /// are expanded one row at a time, never into another full RGBA canvas.
    pub fn rgba_rows_owned(
        &self, source: std::sync::Arc<DynamicImage>,
    ) -> Result<RgbaRows<'static>, ImageError> {
        if self.dimensions != Some(source.dimensions()) {
            return Err(ImageError::InvalidDimensions);
        }
        let geometry = TransformGeometry::new(source.dimensions(), self.options, self.margins)?;
        RgbaRows::new_owned(source, geometry, self.options.gamma)
    }

    /// Conservative requested Rust storage for a transformed-row iterator,
    /// excluding its source, codec buffers, and allocator/runtime overhead.
    /// This performs no allocation and may be used for memory admission.
    pub fn row_memory_bytes(&self) -> Result<u64, ImageError> {
        let dimensions = self.dimensions.ok_or(ImageError::InvalidDimensions)?;
        let geometry = TransformGeometry::new(dimensions, self.options, self.margins)?;
        pixels::row_memory_bytes(geometry)
    }

    /// Converts ascending RGBA source rows without allocating a source canvas.
    /// Crop/rotation require the replayable or retained-canvas APIs instead.
    pub fn source_rows(&self, depth: RowDepth) -> Result<SourceRows, ImageError> {
        let dimensions = self.dimensions.ok_or(ImageError::InvalidDimensions)?;
        let geometry = TransformGeometry::new(dimensions, self.options, self.margins)?;
        SourceRows::new(dimensions, geometry, depth, self.options.gamma)
    }

    /// Allocation-free bound for `source_rows`, excluding caller-owned input
    /// rows, codec buffers, and allocator/runtime overhead.
    pub fn source_row_memory_bytes(&self, depth: RowDepth) -> Result<u64, ImageError> {
        let dimensions = self.dimensions.ok_or(ImageError::InvalidDimensions)?;
        let geometry = TransformGeometry::new(dimensions, self.options, self.margins)?;
        pixels::source_row_memory_bytes(dimensions, geometry, depth)
    }
}

fn transform(source: DynamicImage, options: ImageOptions) -> Result<(DynamicImage, bool), ImageError> {
    let margins = frame_margins(&source, options);
    transform_with_margins(source, options, margins)
}

fn frame_margins(source: &DynamicImage, options: ImageOptions) -> Option<Margins> {
    if options.auto_margin_limit_h > 0 || options.auto_margin_limit_v > 0 {
        plain_margin(source, options)
    } else {
        None
    }
}

#[derive(Clone, Copy)]
struct TransformGeometry {
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    rotate: i32,
    output_width: u32,
    output_height: u32,
}

impl TransformGeometry {
    fn new(dimensions: (u32, u32), options: ImageOptions, margins: Option<Margins>) -> Result<Self, ImageError> {
        let (source_width, source_height) = dimensions;
        if source_width == 0 || source_height == 0 {
            return Err(ImageError::InvalidDimensions);
        }
        let mut geometry = Self {
            left: 0, top: 0, width: source_width, height: source_height,
            rotate: options.rotate, output_width: source_width, output_height: source_height,
        };
        if let Some(mut margins) = margins {
            adjust_margin_for_display(&mut margins, source_width, source_height,
                options.display_width, options.display_height);
            let width = source_width.saturating_sub(margins.left.saturating_add(margins.right));
            let height = source_height.saturating_sub(margins.top.saturating_add(margins.bottom));
            if width != 0 && height != 0 {
                geometry.left = margins.left;
                geometry.top = margins.top;
                geometry.width = width;
                geometry.height = height;
            }
        }
        let mut scale = 1.0_f64;
        if options.max_width > 0 {
            scale = scale.min(options.max_width as f64 / geometry.width as f64);
        }
        if options.max_height > 0 {
            scale = scale.min(options.max_height as f64 / geometry.height as f64);
        }
        if !scale.is_finite() || scale <= 0.0 {
            return Err(ImageError::InvalidDimensions);
        }
        geometry.output_width = ((geometry.width as f64 * scale) + 0.5).floor().max(1.0) as u32;
        geometry.output_height = ((geometry.height as f64 * scale) + 0.5).floor().max(1.0) as u32;
        if options.rotate == 90 || options.rotate == -90 {
            std::mem::swap(&mut geometry.output_width, &mut geometry.output_height);
        }
        Ok(geometry)
    }

    fn rotated_dimensions(self) -> (u32, u32) {
        if self.rotate == 90 || self.rotate == -90 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }

    fn source_coordinates(self, x: u32, y: u32) -> (u32, u32) {
        let (x, y) = match self.rotate {
            90 => (y, self.height - 1 - x),
            -90 => (self.width - 1 - y, x),
            _ => (x, y),
        };
        (x + self.left, y + self.top)
    }
}

fn transform_with_margins(
    source: DynamicImage,
    options: ImageOptions,
    margins: Option<Margins>,
) -> Result<(DynamicImage, bool), ImageError> {
    let dimensions = source.dimensions();
    let geometry = TransformGeometry::new(dimensions, options, margins)?;
    let changed = geometry.left != 0 || geometry.top != 0
        || (geometry.width, geometry.height) != dimensions || geometry.rotate != 0
        || (geometry.output_width, geometry.output_height) != dimensions || options.gamma.is_some();
    if !changed {
        return Ok((source, false));
    }
    let mut output = pixels::owned(source, geometry)?;
    if let Some(gamma) = options.gamma {
        apply_gamma(&mut output, gamma)?;
    }
    Ok((output, true))
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

fn apply_gamma(image: &mut DynamicImage, gamma: f32) -> Result<(), ImageError> {
    let exponent = 1.0 / gamma;
    let mut table = [0u8; 256];
    for (value, mapped) in table.iter_mut().enumerate() {
        *mapped = (255.0 * (value as f32 / 255.0).powf(exponent)).round().clamp(0.0, 255.0) as u8;
    }
    macro_rules! gamma8 {
        ($buffer:expr, $colors:expr) => {
            for pixel in $buffer.pixels_mut() {
                for channel in &mut pixel.channels_mut()[..$colors] {
                    *channel = table[usize::from(*channel)];
                }
            }
        };
    }
    macro_rules! gamma16 {
        ($buffer:expr, $colors:expr) => {
            for pixel in $buffer.pixels_mut() {
                for channel in &mut pixel.channels_mut()[..$colors] {
                    *channel = (65535.0 * (f32::from(*channel) / 65535.0).powf(exponent))
                        .round().clamp(0.0, 65535.0) as u16;
                }
            }
        };
    }
    macro_rules! gamma_float {
        ($buffer:expr) => {
            for pixel in $buffer.pixels_mut() {
                for channel in &mut pixel.channels_mut()[..3] {
                    *channel = (*channel).clamp(0.0, 1.0).powf(exponent);
                }
            }
        };
    }
    match image {
        DynamicImage::ImageLuma8(buffer) => gamma8!(buffer, 1),
        DynamicImage::ImageLumaA8(buffer) => gamma8!(buffer, 1),
        DynamicImage::ImageRgb8(buffer) => gamma8!(buffer, 3),
        DynamicImage::ImageRgba8(buffer) => gamma8!(buffer, 3),
        DynamicImage::ImageLuma16(buffer) => gamma16!(buffer, 1),
        DynamicImage::ImageLumaA16(buffer) => gamma16!(buffer, 1),
        DynamicImage::ImageRgb16(buffer) => gamma16!(buffer, 3),
        DynamicImage::ImageRgba16(buffer) => gamma16!(buffer, 3),
        DynamicImage::ImageRgb32F(buffer) => gamma_float!(buffer),
        DynamicImage::ImageRgba32F(buffer) => gamma_float!(buffer),
        other => return Err(ImageError::UnsupportedFormat(format!("pixel format {:?}", other.color()))),
    }
    Ok(())
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
        let mut margin = start;
        for index in (start + 1)..=max.min(image.height().saturating_sub(1)) {
            if colored_h(image, index, limit, ignore, ignore, dust) == 0 {
                margin = index;
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
        assert_eq!(
            process(&source, "image/png", &ini, false, 0).unwrap(),
            source
        );
    }

    #[test]
    fn resizes_and_rotates_using_java_dimensions() {
        let source = png(8, 4, [0, 0, 0, 255]);
        let ini = IniSettings::parse("ResizeW=1\nResizeNumW=4\nRotateImage=1\n").unwrap();
        // 回転は呼び出し側が Java と同じ条件で決める
        let output = process(&source, "image/png", &ini, false, 90).unwrap();
        assert_eq!(dimensions(&output, "image/png"), Some((2, 4)));
    }

    #[test]
    fn applies_java_gamma_table() {
        let source = png(1, 1, [128, 64, 0, 255]);
        let ini = IniSettings::parse("Gamma=1\nGammaValue=2\n").unwrap();
        let output = process(&source, "image/png", &ini, false, 0).unwrap();
        let decoded = image::load_from_memory(&output).unwrap().to_rgba8();
        assert_eq!(decoded.get_pixel(0, 0).0, [181, 128, 0, 255]);
    }

    /// Java `ImageUtils.getPlainMargin`: 上辺も最初の非空白行まで切り取る
    /// (scan_top の代入漏れで上だけ常に 1% のまま残っていた)。
    #[test]
    fn trims_top_margin_like_java() {
        let mut image = image::RgbaImage::from_pixel(200, 300, image::Rgba([255, 255, 255, 255]));
        for y in 40..260 {
            for x in 40..160 {
                image.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        let ini = crate::config::IniSettings::parse(
            "AutoMargin=1\nAutoMarginLimitH=15\nAutoMarginLimitV=30\n",
        )
        .unwrap();
        let margins = plain_margin(
            &DynamicImage::ImageRgba8(image),
            ImageOptions::from_ini(&ini, false, 0),
        )
        .unwrap();
        // 上端は最初の非空白行 (40 行目) 付近まで詰まる。修正前は幅の 1% (2) のままだった。
        assert!(
            margins.top >= 30,
            "top margin was not trimmed: {}",
            margins.top
        );
        // 下端は修正前から正しく詰まっていた (比較対象)
        assert!(margins.bottom >= 30, "bottom margin: {}", margins.bottom);
    }
}
