//! Consumed pixel operations and separable Catmull-Rom with row-sized scratch.
//! The resampler uses image 0.25's arithmetic/order, without its full RGBA-f32
//! intermediate. Downscaling reuses the source allocation; only source rows about
//! to be overwritten and still needed by the next filter support are preserved.
use super::{ImageError, TransformGeometry};
use image::{DynamicImage, GenericImageView, ImageBuffer, Pixel, Primitive};
use std::collections::VecDeque;

trait Sample: Primitive {
    fn as_float(self) -> f32;
    fn from_float(value: f32) -> Self;
}
impl Sample for u8 {
    fn as_float(self) -> f32 { f32::from(self) }
    fn from_float(value: f32) -> Self { value.clamp(0.0, 255.0).round() as u8 }
}
impl Sample for u16 {
    fn as_float(self) -> f32 { f32::from(self) }
    fn from_float(value: f32) -> Self { value.clamp(0.0, 65535.0).round() as u16 }
}
impl Sample for f32 {
    fn as_float(self) -> f32 { self }
    fn from_float(value: f32) -> Self { value.clamp(0.0, 1.0) }
}

macro_rules! map_owned {
    ($source:expr, $function:ident $(, $argument:expr)*) => {
        match $source {
            DynamicImage::ImageLuma8(buffer) => DynamicImage::ImageLuma8($function(buffer $(, $argument)*)?),
            DynamicImage::ImageLumaA8(buffer) => DynamicImage::ImageLumaA8($function(buffer $(, $argument)*)?),
            DynamicImage::ImageRgb8(buffer) => DynamicImage::ImageRgb8($function(buffer $(, $argument)*)?),
            DynamicImage::ImageRgba8(buffer) => DynamicImage::ImageRgba8($function(buffer $(, $argument)*)?),
            DynamicImage::ImageLuma16(buffer) => DynamicImage::ImageLuma16($function(buffer $(, $argument)*)?),
            DynamicImage::ImageLumaA16(buffer) => DynamicImage::ImageLumaA16($function(buffer $(, $argument)*)?),
            DynamicImage::ImageRgb16(buffer) => DynamicImage::ImageRgb16($function(buffer $(, $argument)*)?),
            DynamicImage::ImageRgba16(buffer) => DynamicImage::ImageRgba16($function(buffer $(, $argument)*)?),
            DynamicImage::ImageRgb32F(buffer) => DynamicImage::ImageRgb32F($function(buffer $(, $argument)*)?),
            DynamicImage::ImageRgba32F(buffer) => DynamicImage::ImageRgba32F($function(buffer $(, $argument)*)?),
            other => return Err(ImageError::UnsupportedFormat(format!("pixel format {:?}", other.color()))),
        }
    };
}

pub(super) fn owned(source: DynamicImage, geometry: TransformGeometry) -> Result<DynamicImage, ImageError> {
    Ok(map_owned!(source, owned_buffer, geometry))
}

pub(super) fn borrowed(source: &DynamicImage, geometry: TransformGeometry) -> Result<DynamicImage, ImageError> {
    Ok(map_owned!(source, borrowed_buffer, geometry))
}

fn raw_len(width: u32, height: u32, channels: usize) -> Result<usize, ImageError> {
    (width as usize).checked_mul(height as usize).and_then(|pixels| pixels.checked_mul(channels))
        .ok_or(ImageError::InvalidDimensions)
}

fn finish<P: Pixel>(width: u32, height: u32, mut raw: Vec<P::Subpixel>, color: image::metadata::Cicp)
    -> Result<ImageBuffer<P, Vec<P::Subpixel>>, ImageError>
{
    // A large reduction should release retained capacity. Avoid reallocating a
    // near-original-size image, which would briefly double the live pixel bytes.
    if raw.len() <= raw.capacity() / 4 && raw.capacity() - raw.len() > 1024 * 1024 {
        raw.shrink_to_fit();
    }
    let mut output = ImageBuffer::from_raw(width, height, raw).ok_or(ImageError::InvalidDimensions)?;
    output.set_color_space(color).map_err(|error| ImageError::Encode(error.to_string()))?;
    Ok(output)
}

fn owned_buffer<P>(source: ImageBuffer<P, Vec<P::Subpixel>>, geometry: TransformGeometry)
    -> Result<ImageBuffer<P, Vec<P::Subpixel>>, ImageError>
where P: Pixel + 'static, P::Subpixel: Sample
{
    let channels = usize::from(P::CHANNEL_COUNT);
    let source_width = source.width() as usize;
    let color = source.color_space();
    let mut raw = source.into_raw();
    let width = geometry.width as usize;
    let height = geometry.height as usize;
    if geometry.left != 0 || geometry.top != 0 || width != source_width {
        for row in 0..height {
            let start = ((row + geometry.top as usize) * source_width + geometry.left as usize) * channels;
            raw.copy_within(start..start + width * channels, row * width * channels);
        }
    }
    raw.truncate(raw_len(geometry.width, geometry.height, channels)?);
    if geometry.rotate == 90 || geometry.rotate == -90 {
        rotate(&mut raw, width, height, channels, geometry.rotate);
    }
    let (width, height) = geometry.rotated_dimensions();
    if (width, height) != (geometry.output_width, geometry.output_height) {
        downscale(&mut raw, width, height, geometry.output_width, geometry.output_height, channels)?;
    }
    finish(geometry.output_width, geometry.output_height, raw, color)
}

fn rotate<S: Copy>(raw: &mut [S], width: usize, height: usize, channels: usize, angle: i32) {
    let pixels = width * height;
    let mut visited = vec![0u8; pixels.div_ceil(8)];
    for start in 0..pixels {
        if visited[start / 8] & (1 << (start % 8)) != 0 { continue; }
        let mut carried = [raw[start * channels]; 4];
        carried[..channels].copy_from_slice(&raw[start * channels..(start + 1) * channels]);
        let mut current = start;
        loop {
            visited[current / 8] |= 1 << (current % 8);
            let x = current % width;
            let y = current / width;
            let next = if angle == 90 { x * height + height - 1 - y } else { (width - 1 - x) * height + y };
            for channel in 0..channels {
                std::mem::swap(&mut carried[channel], &mut raw[next * channels + channel]);
            }
            current = next;
            if current == start { break; }
        }
    }
}

struct Weights {
    starts: Vec<usize>,
    offsets: Vec<usize>,
    values: Vec<f32>,
}
impl Weights {
    fn new(source: u32, output: u32) -> Self {
        let ratio = source as f32 / output as f32;
        let scale = ratio.max(1.0);
        let support = 2.0 * scale;
        let mut weights = Self { starts: Vec::with_capacity(output as usize), offsets: Vec::with_capacity(output as usize + 1), values: Vec::new() };
        for position in 0..output {
            let center = (position as f32 + 0.5) * ratio;
            let left = ((center - support).floor() as i64).clamp(0, i64::from(source) - 1) as usize;
            let right = ((center + support).ceil() as i64).clamp(left as i64 + 1, i64::from(source)) as usize;
            weights.starts.push(left);
            let offset = weights.values.len();
            weights.offsets.push(offset);
            let center = center - 0.5;
            let mut sum = 0.0;
            for input in left..right {
                let value = cubic((input as f32 - center) / scale);
                weights.values.push(value);
                sum += value;
            }
            for value in &mut weights.values[offset..] { *value /= sum; }
        }
        weights.offsets.push(weights.values.len());
        weights
    }
    fn row(&self, index: usize) -> (usize, &[f32]) {
        (self.starts[index], &self.values[self.offsets[index]..self.offsets[index + 1]])
    }
}

fn cubic(x: f32) -> f32 {
    let a = x.abs();
    let value = if a < 1.0 {
        9.0 * a.powi(3) - 15.0 * a.powi(2) + 6.0
    } else if a < 2.0 {
        -3.0 * a.powi(3) + 15.0 * a.powi(2) - 24.0 * a + 12.0
    } else { 0.0 };
    value / 6.0
}

fn horizontal<S: Sample>(vertical: &[f32], output: &mut [S], weights: &Weights, channels: usize) {
    for (x, pixel) in output.chunks_exact_mut(channels).enumerate() {
        let (left, values) = weights.row(x);
        for channel in 0..channels {
            let mut sum = 0.0;
            for (offset, weight) in values.iter().enumerate() {
                sum += vertical[(left + offset) * channels + channel] * weight;
            }
            pixel[channel] = S::from_float(sum);
        }
    }
}

fn downscale<S: Sample>(raw: &mut Vec<S>, width: u32, height: u32, output_width: u32, output_height: u32, channels: usize) -> Result<(), ImageError> {
    if output_width == 0 || output_height == 0 || output_width > width || output_height > height {
        return Err(ImageError::InvalidDimensions);
    }
    let horizontal_weights = Weights::new(width, output_width);
    let vertical_weights = Weights::new(height, output_height);
    let stride = width as usize * channels;
    let output_stride = output_width as usize * channels;
    let mut vertical = vec![0.0; stride];
    let mut output = vec![S::from_float(0.0); output_stride];
    let mut preserved: VecDeque<(usize, Vec<S>)> = VecDeque::new();
    let mut spare: Vec<Vec<S>> = Vec::new();
    for y in 0..output_height as usize {
        let (top, weights) = vertical_weights.row(y);
        while preserved.front().is_some_and(|(row, _)| *row < top) {
            spare.push(preserved.pop_front().expect("front exists").1);
        }
        vertical.fill(0.0);
        for (offset, weight) in weights.iter().enumerate() {
            let row = top + offset;
            let source = preserved.iter().find(|(saved, _)| *saved == row).map(|(_, bytes)| bytes.as_slice())
                .unwrap_or(&raw[row * stride..(row + 1) * stride]);
            for (sum, sample) in vertical.iter_mut().zip(source) { *sum += sample.as_float() * weight; }
        }
        horizontal(&vertical, &mut output, &horizontal_weights, channels);
        let start = y * output_stride;
        let end = start + output_stride;
        let future_top = vertical_weights.starts.get(y + 1).copied().unwrap_or(height as usize);
        for row in start / stride..=(end - 1) / stride {
            if row >= future_top && !preserved.iter().any(|(saved, _)| *saved == row) {
                let mut saved = spare.pop().unwrap_or_else(|| Vec::with_capacity(stride));
                saved.clear();
                saved.extend_from_slice(&raw[row * stride..(row + 1) * stride]);
                preserved.push_back((row, saved));
            }
        }
        raw[start..end].copy_from_slice(&output);
    }
    raw.truncate(raw_len(output_width, output_height, channels)?);
    Ok(())
}

fn borrowed_buffer<P>(source: &ImageBuffer<P, Vec<P::Subpixel>>, geometry: TransformGeometry)
    -> Result<ImageBuffer<P, Vec<P::Subpixel>>, ImageError>
where P: Pixel + 'static, P::Subpixel: Sample
{
    let channels = usize::from(P::CHANNEL_COUNT);
    let (width, height) = geometry.rotated_dimensions();
    let mut output = vec![P::Subpixel::from_float(0.0); raw_len(geometry.output_width, geometry.output_height, channels)?];
    if (width, height) == (geometry.output_width, geometry.output_height) {
        for y in 0..height {
            for x in 0..width {
                let (source_x, source_y) = geometry.source_coordinates(x, y);
                let pixel = source.get_pixel(source_x, source_y).channels();
                let index = (y as usize * width as usize + x as usize) * channels;
                output[index..index + channels].copy_from_slice(pixel);
            }
        }
    } else {
        let horizontal_weights = Weights::new(width, geometry.output_width);
        let vertical_weights = Weights::new(height, geometry.output_height);
        let mut vertical = vec![0.0; width as usize * channels];
        for y in 0..geometry.output_height as usize {
            vertical.fill(0.0);
            let (top, weights) = vertical_weights.row(y);
            for (offset, weight) in weights.iter().enumerate() {
                for x in 0..width {
                    let (source_x, source_y) = geometry.source_coordinates(x, (top + offset) as u32);
                    let pixel = source.get_pixel(source_x, source_y).channels();
                    for channel in 0..channels {
                        vertical[x as usize * channels + channel] += pixel[channel].as_float() * weight;
                    }
                }
            }
            let start = y * geometry.output_width as usize * channels;
            horizontal(&vertical, &mut output[start..start + geometry.output_width as usize * channels], &horizontal_weights, channels);
        }
    }
    finish(geometry.output_width, geometry.output_height, output, source.color_space())
}

/// Upper bound on requested Rust storage for the row iterator, not including
/// the retained source, codec storage, or allocator/runtime overhead.
pub(super) fn row_memory_bytes(geometry: TransformGeometry) -> Result<u64, ImageError> {
    let (width, height) = geometry.rotated_dimensions();
    let mut bytes = std::mem::size_of::<RgbaRows<'static>>() as u64;
    bytes = bytes.checked_add(u64::from(geometry.output_width) * 4)
        .ok_or(ImageError::InvalidDimensions)?;
    if (width, height) != (geometry.output_width, geometry.output_height) {
        bytes = bytes.checked_add(u64::from(width) * 4 * 4)
            .and_then(|bytes| bytes.checked_add(weights_memory_bytes(width, geometry.output_width)?))
            .and_then(|bytes| bytes.checked_add(weights_memory_bytes(height, geometry.output_height)?))
            .ok_or(ImageError::InvalidDimensions)?;
    }
    Ok(bytes)
}

fn weights_memory_bytes(source: u32, output: u32) -> Option<u64> {
    if source == 0 || output == 0 {
        return None;
    }
    let support = 2.0 * (source as f32 / output as f32).max(1.0);
    // A tap interval is ceil(center+support)-floor(center-support).
    // Bound the two f32 additions' rounding before their integer rounding;
    // this also remains conservative for dimensions above f32's exact range.
    let largest_coordinate = (f64::from(source) + f64::from(support)) * 2.0;
    let float_slack = largest_coordinate * f64::from(f32::EPSILON) * 2.0;
    let taps = ((2.0 * f64::from(support) + float_slack).ceil() as u64)
        .checked_add(2)?.min(u64::from(source));
    let samples = u64::from(output).checked_mul(taps)?;
    // Starts reserves output usize entries; offsets reserves output+1.
    let indices = (u64::from(output) * 2 + 1)
        .checked_mul(std::mem::size_of::<usize>() as u64)?;
    // Vec<f32> grows geometrically from a four-element minimum. At growth,
    // old+new capacity is bounded by max(4, 3*final_sample_count).
    let sample_bytes = samples.checked_mul(3)?.max(4)
        .checked_mul(std::mem::size_of::<f32>() as u64)?;
    indices.checked_add(sample_bytes)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowDepth {
    Eight,
    Sixteen,
}

impl RowDepth {
    fn bytes(self) -> u64 {
        match self { Self::Eight => 1, Self::Sixteen => 2 }
    }
}

struct ActiveRow {
    target: usize,
    samples: Vec<f32>,
}

/// Source-row-fed Catmull-Rom resampler. Drain `next_row` after each push;
/// no complete source or target image is retained. Sixteen-bit output is PNG
/// big-endian RGBA, and alpha is never gamma-corrected.
pub struct SourceRows {
    source_width: u32,
    source_height: u32,
    output_width: u32,
    output_height: u32,
    depth: RowDepth,
    horizontal: Weights,
    vertical: Weights,
    active: VecDeque<ActiveRow>,
    spare: Vec<Vec<f32>>,
    row: Vec<u8>,
    gamma: Option<f32>,
    gamma8: Option<[u8; 256]>,
    source_y: usize,
    next_target: usize,
    emitted: usize,
    finished: bool,
}

impl SourceRows {
    pub(super) fn new(dimensions: (u32, u32), geometry: TransformGeometry, depth: RowDepth, gamma: Option<f32>) -> Result<Self, ImageError> {
        validate_source_geometry(dimensions, geometry)?;
        let row_length = raw_len(geometry.output_width, 1, 4)?
            .checked_mul(depth.bytes() as usize).ok_or(ImageError::InvalidDimensions)?;
        let gamma8 = gamma.filter(|_| depth == RowDepth::Eight).map(|value| {
            let mut table = [0u8; 256];
            for (channel, output) in table.iter_mut().enumerate() {
                *output = (255.0 * (channel as f32 / 255.0).powf(1.0 / value))
                    .round().clamp(0.0, 255.0) as u8;
            }
            table
        });
        Ok(Self {
            source_width: dimensions.0, source_height: dimensions.1,
            output_width: geometry.output_width, output_height: geometry.output_height,
            depth,
            horizontal: Weights::new(dimensions.0, geometry.output_width),
            vertical: Weights::new(dimensions.1, geometry.output_height),
            active: VecDeque::new(), spare: Vec::new(), row: vec![0; row_length],
            gamma, gamma8, source_y: 0, next_target: 0, emitted: 0, finished: false,
        })
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.output_width, self.output_height)
    }

    pub fn push_rgba8(&mut self, row: &[u8]) -> Result<(), ImageError> {
        if self.depth != RowDepth::Eight {
            return Err(ImageError::Decode("expected a sixteen-bit source row".to_owned()));
        }
        self.push(row)
    }

    pub fn push_rgba16(&mut self, row: &[u16]) -> Result<(), ImageError> {
        if self.depth != RowDepth::Sixteen {
            return Err(ImageError::Decode("expected an eight-bit source row".to_owned()));
        }
        self.push(row)
    }

    fn ready(&self) -> bool {
        self.active.front().is_some_and(|row| {
            let (top, weights) = self.vertical.row(row.target);
            top + weights.len() <= self.source_y
        })
    }

    fn push<S: Sample>(&mut self, row: &[S]) -> Result<(), ImageError> {
        if self.finished || self.source_y >= self.source_height as usize {
            return Err(ImageError::Decode("source row supplied after EOF".to_owned()));
        }
        if self.ready() {
            return Err(ImageError::Decode("drain completed output rows before supplying input".to_owned()));
        }
        let samples = raw_len(self.source_width, 1, 4)?;
        if row.len() != samples {
            return Err(ImageError::InvalidDimensions);
        }
        while self.next_target < self.output_height as usize
            && self.vertical.starts[self.next_target] <= self.source_y
        {
            let mut buffer = self.spare.pop().unwrap_or_else(|| vec![0.0; samples]);
            buffer.fill(0.0);
            self.active.push_back(ActiveRow { target: self.next_target, samples: buffer });
            self.next_target += 1;
        }
        for active in &mut self.active {
            let (top, weights) = self.vertical.row(active.target);
            let weight = weights.get(self.source_y - top)
                .ok_or_else(|| ImageError::Decode("invalid active source-row interval".to_owned()))?;
            for (sum, sample) in active.samples.iter_mut().zip(row) {
                *sum += sample.as_float() * weight;
            }
        }
        self.source_y += 1;
        Ok(())
    }

    /// `None` means more source rows are needed (or all output has been drained).
    /// `finish` distinguishes complete input from a truncated source.
    pub fn next_row(&mut self) -> Option<&[u8]> {
        if !self.ready() {
            return None;
        }
        let active = self.active.pop_front().expect("ready output row exists");
        let bytes = self.depth.bytes() as usize;
        for x in 0..self.output_width as usize {
            let (left, weights) = self.horizontal.row(x);
            for channel in 0..4 {
                let mut sum = 0.0;
                for (offset, weight) in weights.iter().enumerate() {
                    sum += active.samples[(left + offset) * 4 + channel] * weight;
                }
                let index = (x * 4 + channel) * bytes;
                match self.depth {
                    RowDepth::Eight => {
                        let mut value = u8::from_float(sum);
                        if channel != 3 && let Some(gamma) = &self.gamma8 {
                            value = gamma[usize::from(value)];
                        }
                        self.row[index] = value;
                    }
                    RowDepth::Sixteen => {
                        let mut value = u16::from_float(sum);
                        if channel != 3 && let Some(gamma) = self.gamma {
                            value = (65535.0 * (f32::from(value) / 65535.0).powf(1.0 / gamma))
                                .round().clamp(0.0, 65535.0) as u16;
                        }
                        self.row[index..index + 2].copy_from_slice(&value.to_be_bytes());
                    }
                }
            }
        }
        self.spare.push(active.samples);
        self.emitted += 1;
        Some(&self.row)
    }

    pub fn finish(&mut self) -> Result<(), ImageError> {
        if self.finished || self.source_y != self.source_height as usize
            || self.emitted != self.output_height as usize || !self.active.is_empty()
        {
            return Err(ImageError::Decode("source rows are incomplete or output has not been drained".to_owned()));
        }
        self.finished = true;
        self.spare = Vec::new();
        self.active = VecDeque::new();
        self.row = Vec::new();
        self.horizontal = Weights { starts: Vec::new(), offsets: Vec::new(), values: Vec::new() };
        self.vertical = Weights { starts: Vec::new(), offsets: Vec::new(), values: Vec::new() };
        Ok(())
    }
}

fn validate_source_geometry(dimensions: (u32, u32), geometry: TransformGeometry) -> Result<(), ImageError> {
    if dimensions.0 == 0 || dimensions.1 == 0 || geometry.output_width == 0 || geometry.output_height == 0
        || geometry.output_width > dimensions.0 || geometry.output_height > dimensions.1 {
        return Err(ImageError::InvalidDimensions);
    }
    if geometry.left != 0 || geometry.top != 0 || geometry.rotate != 0
        || (geometry.width, geometry.height) != dimensions {
        return Err(ImageError::Decode("source-row sampling requires uncropped, unrotated input".to_owned()));
    }
    Ok(())
}

pub(super) fn source_row_memory_bytes(dimensions: (u32, u32), geometry: TransformGeometry, depth: RowDepth) -> Result<u64, ImageError> {
    validate_source_geometry(dimensions, geometry)?;
    let ratio = (dimensions.1 as f32 / geometry.output_height as f32).max(1.0);
    let support = 2.0 * ratio;
    let slack = (f64::from(dimensions.1) + f64::from(support)) * 4.0 * f64::from(f32::EPSILON);
    let overlapping_rows = ((2.0 * f64::from(support) + slack) / f64::from(ratio)).ceil() as u64 + 4;
    let active = overlapping_rows.min(u64::from(geometry.output_height));
    let accumulator_bytes = active.checked_mul(u64::from(dimensions.0) * 4 * 4);
    // Active/spare containers retain no more than the peak number of buffers.
    // Account conservatively for both containers' metadata and growth copies.
    let container_bytes = active.checked_mul(6)
        .and_then(|count| count.checked_mul(std::mem::size_of::<ActiveRow>() as u64));
    let bytes = (std::mem::size_of::<SourceRows>() as u64)
        .checked_add(u64::from(geometry.output_width) * 4 * depth.bytes())
        .and_then(|bytes| bytes.checked_add(weights_memory_bytes(dimensions.0, geometry.output_width)?))
        .and_then(|bytes| bytes.checked_add(weights_memory_bytes(dimensions.1, geometry.output_height)?))
        .and_then(|bytes| bytes.checked_add(accumulator_bytes?))
        .and_then(|bytes| bytes.checked_add(container_bytes?))
        .ok_or(ImageError::InvalidDimensions)?;
    Ok(bytes)
}

enum RgbaSource<'a> {
    Borrowed(&'a image::RgbaImage),
    Owned(std::sync::Arc<DynamicImage>),
}

impl RgbaSource<'_> {
    fn get_pixel(&self, x: u32, y: u32) -> image::Rgba<u8> {
        match self {
            Self::Borrowed(source) => *source.get_pixel(x, y),
            Self::Owned(source) => source.get_pixel(x, y),
        }
    }
}

/// Pull-driven transformed RGBA8 rows over a retained compositing canvas.
/// The returned row borrows reusable storage and is invalidated by the next pull.
pub struct RgbaRows<'a> {
    source: RgbaSource<'a>,
    geometry: TransformGeometry,
    horizontal: Option<Weights>,
    vertical_weights: Option<Weights>,
    vertical: Vec<f32>,
    row: Vec<u8>,
    gamma: Option<[u8; 256]>,
    next_y: u32,
}

impl<'a> RgbaRows<'a> {
    pub(super) fn new(source: &'a image::RgbaImage, geometry: TransformGeometry, gamma: Option<f32>) -> Result<Self, ImageError> {
        Self::build(RgbaSource::Borrowed(source), geometry, gamma)
    }

    pub(super) fn new_owned(
        source: std::sync::Arc<DynamicImage>, geometry: TransformGeometry, gamma: Option<f32>,
    ) -> Result<RgbaRows<'static>, ImageError> {
        if !matches!(source.color(), image::ColorType::L8 | image::ColorType::La8
            | image::ColorType::Rgb8 | image::ColorType::Rgba8)
        {
            return Err(ImageError::UnsupportedFormat(format!(
                "RGBA8 row output requires an 8-bit source, got {:?}", source.color())));
        }
        RgbaRows::build(RgbaSource::Owned(source), geometry, gamma)
    }

    fn build(source: RgbaSource<'a>, geometry: TransformGeometry, gamma: Option<f32>) -> Result<Self, ImageError> {
        let (width, height) = geometry.rotated_dimensions();
        let resize = (width, height) != (geometry.output_width, geometry.output_height);
        let gamma = gamma.map(|value| {
            let mut table = [0u8; 256];
            for (channel, output) in table.iter_mut().enumerate() {
                *output = (255.0 * (channel as f32 / 255.0).powf(1.0 / value))
                    .round().clamp(0.0, 255.0) as u8;
            }
            table
        });
        Ok(Self {
            source, geometry,
            horizontal: resize.then(|| Weights::new(width, geometry.output_width)),
            vertical_weights: resize.then(|| Weights::new(height, geometry.output_height)),
            vertical: if resize { vec![0.0; raw_len(width, 1, 4)?] } else { Vec::new() },
            row: vec![0; raw_len(geometry.output_width, 1, 4)?],
            gamma,
            next_y: 0,
        })
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.geometry.output_width, self.geometry.output_height)
    }

    pub fn next_row(&mut self) -> Option<&[u8]> {
        if self.next_y == self.geometry.output_height {
            return None;
        }
        if let (Some(horizontal_weights), Some(vertical_weights)) = (&self.horizontal, &self.vertical_weights) {
            let (width, _) = self.geometry.rotated_dimensions();
            self.vertical.fill(0.0);
            let (top, weights) = vertical_weights.row(self.next_y as usize);
            for (offset, weight) in weights.iter().enumerate() {
                for x in 0..width {
                    let (source_x, source_y) = self.geometry.source_coordinates(x, (top + offset) as u32);
                    let pixel = self.source.get_pixel(source_x, source_y).0;
                    for channel in 0..4 {
                        self.vertical[x as usize * 4 + channel] += f32::from(pixel[channel]) * weight;
                    }
                }
            }
            horizontal(&self.vertical, &mut self.row, horizontal_weights, 4);
        } else {
            for x in 0..self.geometry.output_width {
                let (source_x, source_y) = self.geometry.source_coordinates(x, self.next_y);
                self.row[x as usize * 4..(x as usize + 1) * 4]
                    .copy_from_slice(&self.source.get_pixel(source_x, source_y).0);
            }
        }
        if let Some(gamma) = &self.gamma {
            for pixel in self.row.chunks_exact_mut(4) {
                for channel in &mut pixel[..3] {
                    *channel = gamma[usize::from(*channel)];
                }
            }
        }
        self.next_y += 1;
        Some(&self.row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn geometry(width: u32, height: u32, output_width: u32, output_height: u32, rotate: i32) -> TransformGeometry {
        TransformGeometry { left: 0, top: 0, width, height, rotate, output_width, output_height }
    }

    #[test]
    fn consumed_and_borrowed_resampling_match_image_catmullrom() {
        for (width, height) in [(7, 9), (31, 19), (64, 32), (4, 2)] {
            let source = RgbaImage::from_fn(width, height, |x, y| Rgba([
                (x.wrapping_mul(37) + y.wrapping_mul(13)) as u8,
                (x.wrapping_mul(19) + y.wrapping_mul(57)) as u8,
                (x.wrapping_mul(97) + y.wrapping_mul(31)) as u8,
                (x.wrapping_mul(71) + y.wrapping_mul(17)) as u8,
            ]));
            for (output_width, output_height) in [(1, 1), (width - 1, height - 1), (width / 2, height / 2), (width, 1), (1, height)] {
                let expected = image::imageops::resize(&source, output_width, output_height, image::imageops::FilterType::CatmullRom);
                let plan = geometry(width, height, output_width, output_height, 0);
                assert_eq!(owned_buffer(source.clone(), plan).unwrap(), expected);
                assert_eq!(borrowed_buffer(&source, plan).unwrap(), expected);
            }
        }
    }

    #[test]
    fn crop_and_rectangular_rotation_reuse_the_pixel_allocation() {
        let source = RgbaImage::from_fn(11, 7, |x, y| Rgba([x as u8, y as u8, 53, 127]));
        for rotate in [-90, 90] {
            let mut plan = geometry(7, 4, 4, 7, rotate);
            plan.left = 2;
            plan.top = 1;
            let cropped = image::imageops::crop_imm(&source, 2, 1, 7, 4).to_image();
            let expected = if rotate == 90 { image::imageops::rotate90(&cropped) } else { image::imageops::rotate270(&cropped) };
            let owned = source.clone();
            let allocation = owned.as_ptr();
            let actual = owned_buffer(owned, plan).unwrap();
            assert_eq!(actual.as_ptr(), allocation);
            assert_eq!(actual, expected);
            assert_eq!(borrowed_buffer(&source, plan).unwrap(), expected);
        }
    }

    #[test]
    fn sixteen_bit_resampling_keeps_channel_precision() {
        let source = ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_fn(17, 13, |x, y| {
            image::Rgba([(x * 1319 + y * 79) as u16, (x * 173 + y * 3917) as u16, 12345, 43210])
        });
        let expected = image::imageops::resize(&source, 11, 7, image::imageops::FilterType::CatmullRom);
        let actual = owned_buffer(source, geometry(17, 13, 11, 7, 0)).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn four_k_crop_does_not_allocate_a_second_frame() {
        let source = RgbaImage::from_pixel(4096, 4096, Rgba([13, 27, 41, 127]));
        let allocation = source.as_ptr();
        let mut plan = geometry(4000, 4000, 4000, 4000, 0);
        plan.left = 48;
        plan.top = 48;
        let output = owned_buffer(source, plan).unwrap();
        assert_eq!(output.as_ptr(), allocation);
        assert_eq!(output.dimensions(), (4000, 4000));
        assert_eq!(*output.get_pixel(3999, 3999), Rgba([13, 27, 41, 127]));
    }

    #[test]
    fn pulled_rows_match_crop_rotate_resize_and_gamma_without_a_target_frame() {
        let source = RgbaImage::from_fn(8, 5, |x, y| Rgba([
            (x * 23 + y * 17) as u8, (x * 9 + y * 31) as u8, 73, 127,
        ]));
        let mut plan = geometry(6, 3, 2, 4, 90);
        plan.left = 1;
        plan.top = 1;
        let cropped = image::imageops::crop_imm(&source, 1, 1, 6, 3).to_image();
        let rotated = image::imageops::rotate90(&cropped);
        let mut expected = image::imageops::resize(&rotated, 2, 4, image::imageops::FilterType::CatmullRom);
        for pixel in expected.pixels_mut() {
            for value in &mut pixel.0[..3] {
                *value = (255.0 * (f32::from(*value) / 255.0).powf(1.0 / 1.5))
                    .round().clamp(0.0, 255.0) as u8;
            }
        }
        let mut rows = RgbaRows::new(&source, plan, Some(1.5)).unwrap();
        assert_eq!(rows.dimensions(), (2, 4));
        assert_eq!(rows.vertical.len(), 3 * 4);
        assert_eq!(rows.row.len(), 2 * 4);
        let mut actual = Vec::new();
        while let Some(row) = rows.next_row() {
            actual.extend_from_slice(row);
        }
        assert!(rows.next_row().is_none());
        assert_eq!(actual, expected.into_raw());
        assert_eq!(*source.get_pixel(0, 0), Rgba([0, 0, 73, 127]));
    }

    #[test]
    fn source_rows_match_catmullrom_without_source_or_target_storage() {
        let source = RgbaImage::from_fn(31, 19, |x, y| Rgba([
            (x * 37 + y * 13) as u8, (x * 19 + y * 57) as u8,
            (x * 97 + y * 31) as u8, (x * 71 + y * 17) as u8,
        ]));
        for (width, height) in [(1, 1), (30, 18), (17, 7), (31, 19), (1, 19)] {
            let plan = geometry(31, 19, width, height, 0);
            let mut rows = SourceRows::new((31, 19), plan, RowDepth::Eight, None).unwrap();
            let mut actual = Vec::new();
            for row in source.as_raw().chunks_exact(31 * 4) {
                rows.push_rgba8(row).unwrap();
                while let Some(row) = rows.next_row() {
                    actual.extend_from_slice(row);
                }
            }
            rows.finish().unwrap();
            let expected = image::imageops::resize(&source, width, height, image::imageops::FilterType::CatmullRom);
            assert_eq!(actual, expected.into_raw());
        }
    }

    #[test]
    fn source_rows_preserve_sixteen_bit_samples_and_byte_order() {
        let source = ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_fn(14, 9, |x, y| {
            image::Rgba([(x * 1319 + y * 79) as u16, (x * 173 + y * 3917) as u16, 12345, 43210])
        });
        let mut rows = SourceRows::new((14, 9), geometry(14, 9, 7, 5, 0), RowDepth::Sixteen, None).unwrap();
        let mut actual = Vec::new();
        for row in source.as_raw().chunks_exact(14 * 4) {
            rows.push_rgba16(row).unwrap();
            while let Some(row) = rows.next_row() {
                actual.extend(row.chunks_exact(2).map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]])));
            }
        }
        rows.finish().unwrap();
        let expected = image::imageops::resize(&source, 7, 5, image::imageops::FilterType::CatmullRom);
        assert_eq!(actual, expected.into_raw());
    }

    #[test]
    fn source_rows_enforce_backpressure_and_complete_input() {
        let mut rows = SourceRows::new((2, 8), geometry(2, 8, 2, 8, 0), RowDepth::Eight, None).unwrap();
        let input = [23; 8];
        for _ in 0..3 { rows.push_rgba8(&input).unwrap(); }
        assert!(rows.push_rgba8(&input).is_err());
        assert!(rows.finish().is_err());
        assert!(rows.next_row().is_some());
        for _ in 3..8 {
            rows.push_rgba8(&input).unwrap();
            while rows.next_row().is_some() {}
        }
        rows.finish().unwrap();
        assert!(rows.push_rgba8(&input).is_err());
        assert!(rows.next_row().is_none());
    }

    #[test]
    fn six_k_source_keeps_only_overlapping_target_rows() {
        let plan = geometry(6000, 6000, 600, 600, 0);
        let admitted = source_row_memory_bytes((6000, 6000), plan, RowDepth::Eight).unwrap();
        assert!(admitted < 2 * 1024 * 1024);
        let mut rows = SourceRows::new((6000, 6000), plan, RowDepth::Eight, None).unwrap();
        let input = vec![127; 6000 * 4];
        for _ in 0..40 {
            rows.push_rgba8(&input).unwrap();
            assert!(rows.active.len() + rows.spare.len() <= 5);
            while let Some(row) = rows.next_row() {
                assert_eq!(row.len(), 600 * 4);
                assert!(row.iter().all(|sample| *sample == 127));
            }
        }
        assert!(rows.finish().is_err(), "truncated source must not become successful EOF");
    }
}
