//! Deterministic binary glyph rasterization over an immutable sampled grid.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::Arc;

use fontdue::{Font, FontSettings};
use image_to_binary_core::{ContractError, Sampling};

const FONT_BYTES: &[u8] = include_bytes!("../../../assets/fonts/IBMPlexMono-Regular.ttf");
const DEFAULT_CACHE_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_OUTPUT_BYTES: u64 = 256 * 1024 * 1024;

/// Errors returned by the renderer before a frame is exposed to callers.
#[derive(Debug)]
pub enum RenderError {
    /// A public rendering value is outside its contract.
    Contract(ContractError),
    /// The bundled font could not be parsed.
    InvalidFont(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => error.fmt(formatter),
            Self::InvalidFont(error) => write!(formatter, "bundled font is invalid: {error}"),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<ContractError> for RenderError {
    fn from(error: ContractError) -> Self {
        Self::Contract(error)
    }
}

/// Immutable settings for classic binary rendering.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderOptions {
    /// Source cell size and output tile size in pixels.
    pub cell_size: u8,
    /// Font size in pixels before preview fitting.
    pub font_size: u8,
    /// Luminance below this value selects `1`.
    pub threshold: u8,
    /// Sampling geometry used to produce the supplied grid.
    pub sampling: Sampling,
    /// Maximum owned output bytes.
    pub max_output_bytes: u64,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            cell_size: 20,
            font_size: 20,
            threshold: 128,
            sampling: Sampling::Classic,
            max_output_bytes: DEFAULT_OUTPUT_BYTES,
        }
    }
}

impl RenderOptions {
    fn validate(self) -> Result<(), RenderError> {
        if !(5..=30).contains(&self.cell_size) {
            return Err(ContractError::InvalidSettings("cell_size").into());
        }
        if !(8..=30).contains(&self.font_size) {
            return Err(ContractError::InvalidSettings("font_size").into());
        }
        if self.max_output_bytes == 0 {
            return Err(ContractError::InvalidSettings("output limit").into());
        }
        Ok(())
    }
}

/// Owned RGBA8 output with dimensions checked before allocation.
#[derive(Debug, Eq, PartialEq)]
pub struct RenderedFrame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl RenderedFrame {
    /// Returns output width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }
    /// Returns output height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }
    /// Borrows the owned RGBA8 pixels.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

/// Bounded glyph-cache accounting for diagnostics and acceptance checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RendererCacheStats {
    entries: usize,
    bytes: u64,
    capacity: u64,
}

/// Geometry for the non-classic font-aspect-corrected sampling preset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AspectGeometry {
    /// Horizontal source-cell width in source pixels.
    pub source_cell_width: u8,
    /// Rounded vertical source-cell height in source pixels.
    pub source_cell_height: u16,
    /// Output glyph-cell width in pixels.
    pub output_cell_width: u16,
    /// Output glyph-cell height in pixels.
    pub output_cell_height: u16,
}

impl RendererCacheStats {
    #[must_use]
    pub const fn entries(self) -> usize {
        self.entries
    }
    #[must_use]
    pub const fn bytes(self) -> u64 {
        self.bytes
    }
    #[must_use]
    pub const fn capacity(self) -> u64 {
        self.capacity
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct GlyphKey {
    character: u8,
    size_bits: u32,
}

#[derive(Clone, Debug)]
struct GlyphMask {
    width: usize,
    height: usize,
    xmin: i32,
    ymin: i32,
    bitmap: Vec<u8>,
    bytes: u64,
}

#[derive(Debug)]
struct GlyphAtlas {
    entries: HashMap<GlyphKey, Arc<GlyphMask>>,
    order: VecDeque<GlyphKey>,
    bytes: u64,
    capacity: u64,
}

impl GlyphAtlas {
    fn new(capacity: u64) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            capacity,
        }
    }

    fn get(
        &mut self,
        font: &Font,
        character: u8,
        size: f32,
    ) -> Result<Arc<GlyphMask>, RenderError> {
        let key = GlyphKey {
            character,
            size_bits: size.to_bits(),
        };
        if self.entries.contains_key(&key) {
            self.order.retain(|item| *item != key);
            self.order.push_back(key);
            return self
                .entries
                .get(&key)
                .cloned()
                .ok_or(ContractError::ArithmeticOverflow.into());
        }
        let (metrics, bitmap) = font.rasterize(char::from(character), size);
        let bytes =
            u64::try_from(bitmap.capacity()).map_err(|_| ContractError::ArithmeticOverflow)?;
        if self.capacity != 0 && bytes <= self.capacity {
            while self.bytes > self.capacity - bytes {
                if let Some(old) = self.order.pop_front() {
                    if let Some(entry) = self.entries.remove(&old) {
                        self.bytes = self
                            .bytes
                            .checked_sub(entry.bytes)
                            .ok_or(ContractError::ArithmeticOverflow)?;
                    }
                } else {
                    break;
                }
            }
            self.bytes = self
                .bytes
                .checked_add(bytes)
                .ok_or(ContractError::ArithmeticOverflow)?;
            self.order.push_back(key);
            let mask = Arc::new(GlyphMask {
                width: metrics.width,
                height: metrics.height,
                xmin: metrics.xmin,
                ymin: metrics.ymin,
                bitmap,
                bytes,
            });
            self.entries.insert(key, Arc::clone(&mask));
            return Ok(mask);
        }
        Ok(Arc::new(GlyphMask {
            width: metrics.width,
            height: metrics.height,
            xmin: metrics.xmin,
            ymin: metrics.ymin,
            bitmap,
            bytes,
        }))
    }
}

/// Owns the parsed bundled font and private bounded glyph masks.
#[derive(Debug)]
pub struct Renderer {
    font: Font,
    atlas: GlyphAtlas,
}

impl Renderer {
    fn validate_grid(
        &self,
        grid: &crate::SampleGrid,
        options: RenderOptions,
    ) -> Result<(), RenderError> {
        if grid.sampling() != options.sampling || grid.source_cell_width() != options.cell_size {
            return Err(ContractError::InvalidSettings("sampling geometry").into());
        }
        match options.sampling {
            Sampling::Classic if grid.source_cell_height() != u32::from(options.cell_size) => {
                Err(ContractError::InvalidSettings("sampling geometry").into())
            }
            Sampling::Average => {
                let geometry = self.aspect_geometry(options.cell_size, options.font_size)?;
                if grid.source_cell_height() != u32::from(geometry.source_cell_height) {
                    return Err(ContractError::InvalidSettings("sampling geometry").into());
                }
                Ok(())
            }
            Sampling::Classic => Ok(()),
        }
    }

    /// Creates a renderer using the bundled IBM Plex Mono font.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::InvalidFont`] if the embedded font is malformed.
    pub fn new() -> Result<Self, RenderError> {
        let font = Font::from_bytes(FONT_BYTES, FontSettings::default())
            .map_err(|error| RenderError::InvalidFont(error.to_string()))?;
        Ok(Self {
            font,
            atlas: GlyphAtlas::new(DEFAULT_CACHE_BYTES),
        })
    }

    /// Creates a renderer with an explicit glyph-mask byte ceiling.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::InvalidFont`] if the embedded font is malformed.
    pub fn with_cache_capacity(capacity: u64) -> Result<Self, RenderError> {
        let mut renderer = Self::new()?;
        renderer.atlas = GlyphAtlas::new(capacity);
        Ok(renderer)
    }

    /// Returns private cache accounting without exposing cache entries.
    #[must_use]
    pub fn cache_stats(&self) -> RendererCacheStats {
        RendererCacheStats {
            entries: self.atlas.entries.len(),
            bytes: self.atlas.bytes,
            capacity: self.atlas.capacity,
        }
    }

    /// Measures the bundled monospace advance and line height for new presets.
    ///
    /// Classic rendering does not use this geometry. The returned source-cell
    /// height is rounded and bounded for rectangular average sampling; callers
    /// should expect at most one source-cell of aspect-ratio edge rounding.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Contract`] when either input is outside its range.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub fn aspect_geometry(
        &self,
        cell_width: u8,
        font_size: u8,
    ) -> Result<AspectGeometry, RenderError> {
        if !(5..=30).contains(&cell_width) {
            return Err(ContractError::InvalidSettings("cell_size").into());
        }
        if !(8..=30).contains(&font_size) {
            return Err(ContractError::InvalidSettings("font_size").into());
        }
        let size = f32::from(font_size);
        let (metrics, _) = self.font.rasterize('0', size);
        let line_height = self
            .font
            .horizontal_line_metrics(size)
            .map_or(metrics.height as f32, |value| value.new_line_size);
        let advance = metrics.advance_width.max(1.0);
        let source_height = (f32::from(cell_width) * line_height / advance)
            .round()
            .clamp(1.0, 1024.0) as u16;
        let output_height = line_height.round().clamp(1.0, f32::from(u16::MAX)) as u16;
        Ok(AspectGeometry {
            source_cell_width: cell_width,
            source_cell_height: source_height,
            output_cell_width: metrics.advance_width.round().max(1.0) as u16,
            output_cell_height: output_height,
        })
    }

    /// Renders a complete RGBA8 frame from the caller-owned source grid.
    ///
    /// # Errors
    ///
    /// Returns a contract error when settings, dimensions, or output allocation
    /// exceed their configured limits.
    pub fn render(
        &mut self,
        grid: &crate::SampleGrid,
        options: RenderOptions,
    ) -> Result<RenderedFrame, RenderError> {
        options.validate()?;
        self.validate_grid(grid, options)?;
        self.render_scaled(grid, options, 1.0)
    }

    /// Renders a bounded preview while preserving the source grid geometry.
    ///
    /// # Errors
    ///
    /// Returns a contract error when settings, dimensions, or output allocation
    /// exceed their configured limits.
    pub fn render_preview(
        &mut self,
        grid: &crate::SampleGrid,
        options: RenderOptions,
    ) -> Result<RenderedFrame, RenderError> {
        options.validate()?;
        self.validate_grid(grid, options)?;
        let (export_tile_width, export_tile_height) = self.tile_dimensions(options)?;
        let export_width = u64::from(grid.width())
            .checked_mul(u64::from(export_tile_width))
            .ok_or(ContractError::ArithmeticOverflow)?;
        let export_height = u64::from(grid.height())
            .checked_mul(u64::from(export_tile_height))
            .ok_or(ContractError::ArithmeticOverflow)?;
        let longest = export_width.max(export_height);
        let target_width = if longest > 1600 {
            export_width.saturating_mul(1600) / longest
        } else {
            export_width
        };
        let target_height = if longest > 1600 {
            export_height.saturating_mul(1600) / longest
        } else {
            export_height
        };
        self.render_preview_inverse(
            grid,
            options,
            u32::try_from(target_width.max(1)).map_err(|_| ContractError::ResourceLimitExceeded)?,
            u32::try_from(target_height.max(1))
                .map_err(|_| ContractError::ResourceLimitExceeded)?,
            u32::try_from(export_width).map_err(|_| ContractError::ResourceLimitExceeded)?,
            u32::try_from(export_height).map_err(|_| ContractError::ResourceLimitExceeded)?,
        )
    }

    /// Renders a contiguous row batch for streaming encoders.
    ///
    /// `start` and `count` address source-grid rows. The returned buffer
    /// contains `count * output_cell_height` pixel rows laid out as RGBA8
    /// pixels with the full frame width.
    ///
    /// # Errors
    ///
    /// Returns a contract error when the row range, settings, or output buffer
    /// exceeds its configured limits.
    pub fn render_rows(
        &mut self,
        grid: &crate::SampleGrid,
        options: RenderOptions,
        start: u32,
        count: u32,
    ) -> Result<Vec<u8>, RenderError> {
        options.validate()?;
        self.validate_grid(grid, options)?;
        let end = start
            .checked_add(count)
            .ok_or(ContractError::ArithmeticOverflow)?;
        if end > grid.height() {
            return Err(ContractError::InvalidDimensions.into());
        }
        let (tile_width, tile_height) = self.tile_dimensions(options)?;
        let width = grid
            .width()
            .checked_mul(tile_width)
            .ok_or(ContractError::ArithmeticOverflow)?;
        let height = count
            .checked_mul(tile_height)
            .ok_or(ContractError::ArithmeticOverflow)?;
        let bytes = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|value| value.checked_mul(4))
            .ok_or(ContractError::ArithmeticOverflow)?;
        if bytes > options.max_output_bytes {
            return Err(ContractError::ResourceLimitExceeded.into());
        }
        let mut pixels = Vec::new();
        let byte_count =
            usize::try_from(bytes).map_err(|_| ContractError::ResourceLimitExceeded)?;
        pixels
            .try_reserve_exact(byte_count)
            .map_err(|_| ContractError::ResourceLimitExceeded)?;
        pixels.resize(byte_count, 0);
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = u8::MAX;
        }
        for source_row in start..end {
            let output_row = source_row - start;
            self.render_row(
                grid,
                options,
                f32::from(options.font_size),
                tile_width,
                tile_height,
                width,
                height,
                source_row,
                output_row,
                &mut pixels,
            )?;
        }
        Ok(pixels)
    }

    /// Returns checked export dimensions without allocating a frame.
    ///
    /// # Errors
    ///
    /// Returns a contract error when grid geometry does not match options or
    /// the derived dimensions overflow.
    pub fn frame_dimensions(
        &self,
        grid: &crate::SampleGrid,
        options: RenderOptions,
    ) -> Result<(u32, u32), RenderError> {
        options.validate()?;
        self.validate_grid(grid, options)?;
        let (tile_width, tile_height) = self.tile_dimensions(options)?;
        Ok((
            grid.width()
                .checked_mul(tile_width)
                .ok_or(ContractError::ArithmeticOverflow)?,
            grid.height()
                .checked_mul(tile_height)
                .ok_or(ContractError::ArithmeticOverflow)?,
        ))
    }

    fn tile_dimensions(&self, options: RenderOptions) -> Result<(u32, u32), RenderError> {
        if options.sampling == Sampling::Average {
            let geometry = self.aspect_geometry(options.cell_size, options.font_size)?;
            Ok((
                u32::from(geometry.output_cell_width),
                u32::from(geometry.output_cell_height),
            ))
        } else {
            Ok((u32::from(options.cell_size), u32::from(options.cell_size)))
        }
    }

    fn render_preview_inverse(
        &mut self,
        grid: &crate::SampleGrid,
        options: RenderOptions,
        width: u32,
        height: u32,
        export_width: u32,
        export_height: u32,
    ) -> Result<RenderedFrame, RenderError> {
        let bytes = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|value| value.checked_mul(4))
            .ok_or(ContractError::ArithmeticOverflow)?;
        if bytes > options.max_output_bytes {
            return Err(ContractError::ResourceLimitExceeded.into());
        }
        let count = usize::try_from(bytes).map_err(|_| ContractError::ResourceLimitExceeded)?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(count)
            .map_err(|_| ContractError::ResourceLimitExceeded)?;
        pixels.resize(count, 0);
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = u8::MAX;
        }
        let (tile_width, tile_height) = self.tile_dimensions(options)?;
        let mask_zero = self
            .atlas
            .get(&self.font, b'0', f32::from(options.font_size))?;
        let mask_one = self
            .atlas
            .get(&self.font, b'1', f32::from(options.font_size))?;
        for y in 0..height {
            let source_y = u32::try_from(
                (u64::from(y) * 2 + 1) * u64::from(export_height) / (2 * u64::from(height)),
            )
            .map_err(|_| ContractError::ArithmeticOverflow)?;
            for x in 0..width {
                let source_x = u32::try_from(
                    (u64::from(x) * 2 + 1) * u64::from(export_width) / (2 * u64::from(width)),
                )
                .map_err(|_| ContractError::ArithmeticOverflow)?;
                let green = Self::export_pixel(
                    grid,
                    options,
                    tile_width,
                    tile_height,
                    &mask_zero,
                    &mask_one,
                    source_x.min(export_width - 1),
                    source_y.min(export_height - 1),
                )?;
                let index = (usize::try_from(y).map_err(|_| ContractError::ArithmeticOverflow)?
                    * usize::try_from(width).map_err(|_| ContractError::ArithmeticOverflow)?
                    + usize::try_from(x).map_err(|_| ContractError::ArithmeticOverflow)?)
                    * 4;
                pixels[index + 1] = green;
            }
        }
        Ok(RenderedFrame {
            width,
            height,
            pixels,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn export_pixel(
        grid: &crate::SampleGrid,
        options: RenderOptions,
        tile_width: u32,
        tile_height: u32,
        mask_zero: &Arc<GlyphMask>,
        mask_one: &Arc<GlyphMask>,
        x: u32,
        y: u32,
    ) -> Result<u8, RenderError> {
        let column = x / tile_width;
        let row = y / tile_height;
        let cell = grid
            .cell(column, row)
            .ok_or(ContractError::InvalidDimensions)?;
        let mask = if cell.binary(options.threshold) == b'1' {
            mask_one
        } else {
            mask_zero
        };
        let local_x = x - column * tile_width;
        let local_y = y - row * tile_height;
        let glyph_left = (i64::from(tile_width)
            - i64::try_from(mask.width).map_err(|_| ContractError::ArithmeticOverflow)?)
            / 2
            + i64::from(mask.xmin);
        let baseline = i64::midpoint(
            i64::from(tile_height),
            i64::try_from(mask.height).map_err(|_| ContractError::ArithmeticOverflow)?,
        ) + i64::from(mask.ymin);
        let glyph_x = i64::from(local_x) - glyph_left;
        let glyph_y = i64::from(local_y)
            - (baseline
                - i64::from(mask.ymin)
                - i64::try_from(mask.height).map_err(|_| ContractError::ArithmeticOverflow)?);
        if glyph_x < 0
            || glyph_y < 0
            || glyph_x
                >= i64::try_from(mask.width).map_err(|_| ContractError::ArithmeticOverflow)?
            || glyph_y
                >= i64::try_from(mask.height).map_err(|_| ContractError::ArithmeticOverflow)?
        {
            return Ok(0);
        }
        let index = usize::try_from(glyph_y)
            .map_err(|_| ContractError::ArithmeticOverflow)?
            .checked_mul(mask.width)
            .and_then(|value| value.checked_add(usize::try_from(glyph_x).ok()?))
            .ok_or(ContractError::ArithmeticOverflow)?;
        Ok(mask.bitmap[index])
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    fn render_scaled(
        &mut self,
        grid: &crate::SampleGrid,
        options: RenderOptions,
        scale: f32,
    ) -> Result<RenderedFrame, RenderError> {
        let (base_width, base_height) = self.tile_dimensions(options)?;
        let width = (base_width as f32 * scale).floor().max(1.0) as u32;
        let height = (base_height as f32 * scale).floor().max(1.0) as u32;
        self.render_dimensions(
            grid,
            options,
            grid.width()
                .checked_mul(width)
                .ok_or(ContractError::ArithmeticOverflow)?,
            grid.height()
                .checked_mul(height)
                .ok_or(ContractError::ArithmeticOverflow)?,
        )
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn render_dimensions(
        &mut self,
        grid: &crate::SampleGrid,
        options: RenderOptions,
        width: u32,
        height: u32,
    ) -> Result<RenderedFrame, RenderError> {
        let bytes = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|value| value.checked_mul(4))
            .ok_or(ContractError::ArithmeticOverflow)?;
        if bytes > options.max_output_bytes {
            return Err(ContractError::ResourceLimitExceeded.into());
        }
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(
                usize::try_from(bytes).map_err(|_| ContractError::ResourceLimitExceeded)?,
            )
            .map_err(|_| ContractError::ResourceLimitExceeded)?;
        pixels.resize(
            usize::try_from(bytes).map_err(|_| ContractError::ResourceLimitExceeded)?,
            0,
        );
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = u8::MAX;
        }
        let (base_width, base_height) = self.tile_dimensions(options)?;
        let base_total_width = grid
            .width()
            .checked_mul(base_width)
            .ok_or(ContractError::ArithmeticOverflow)?;
        #[allow(clippy::cast_precision_loss)]
        let size = f32::from(options.font_size) * width as f32 / base_total_width as f32;
        for row in 0..grid.height() {
            self.render_row(
                grid,
                options,
                size,
                base_width,
                base_height,
                width,
                height,
                row,
                row,
                &mut pixels,
            )?;
        }
        Ok(RenderedFrame {
            width,
            height,
            pixels,
        })
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_lines)]
    fn render_row(
        &mut self,
        grid: &crate::SampleGrid,
        options: RenderOptions,
        size: f32,
        tile: u32,
        tile_height: u32,
        frame_width: u32,
        frame_height: u32,
        row: u32,
        output_row: u32,
        pixels: &mut [u8],
    ) -> Result<(), RenderError> {
        for column in 0..grid.width() {
            let cell = grid
                .cell(column, row)
                .ok_or(ContractError::InvalidDimensions)?;
            let character = cell.binary(options.threshold);
            let mask = self.atlas.get(&self.font, character, size)?;
            let mapped = frame_width
                != grid
                    .width()
                    .checked_mul(tile)
                    .ok_or(ContractError::ArithmeticOverflow)?;
            let left = if mapped {
                u32::try_from(u64::from(column) * u64::from(frame_width) / u64::from(grid.width()))
                    .map_err(|_| ContractError::ArithmeticOverflow)?
            } else {
                column
                    .checked_mul(tile)
                    .ok_or(ContractError::ArithmeticOverflow)?
            };
            let right = if mapped {
                u32::try_from(
                    u64::from(column + 1) * u64::from(frame_width) / u64::from(grid.width()),
                )
                .map_err(|_| ContractError::ArithmeticOverflow)?
            } else {
                left.checked_add(tile)
                    .ok_or(ContractError::ArithmeticOverflow)?
            };
            let top = if mapped {
                u32::try_from(
                    u64::from(output_row) * u64::from(frame_height) / u64::from(grid.height()),
                )
                .map_err(|_| ContractError::ArithmeticOverflow)?
            } else {
                output_row
                    .checked_mul(tile_height)
                    .ok_or(ContractError::ArithmeticOverflow)?
            };
            let bottom = if mapped {
                u32::try_from(
                    u64::from(output_row + 1) * u64::from(frame_height) / u64::from(grid.height()),
                )
                .map_err(|_| ContractError::ArithmeticOverflow)?
            } else {
                top.checked_add(tile_height)
                    .ok_or(ContractError::ArithmeticOverflow)?
            };
            let cell_width = right.saturating_sub(left).max(1);
            let cell_height = bottom.saturating_sub(top).max(1);
            let glyph_left = i64::from(left)
                + (i64::from(cell_width)
                    - i64::try_from(mask.width).map_err(|_| ContractError::ArithmeticOverflow)?)
                    / 2
                + i64::from(mask.xmin);
            let baseline = i64::from(top)
                + i64::midpoint(
                    i64::from(cell_height),
                    i64::try_from(mask.height).map_err(|_| ContractError::ArithmeticOverflow)?,
                )
                + i64::from(mask.ymin);
            for y in 0..mask.height {
                let destination_y = baseline
                    - i64::from(mask.ymin)
                    - i64::try_from(mask.height - y)
                        .map_err(|_| ContractError::ArithmeticOverflow)?;
                if destination_y < i64::from(top) || destination_y >= i64::from(bottom) {
                    continue;
                }
                for x in 0..mask.width {
                    let destination_x = glyph_left
                        + i64::try_from(x).map_err(|_| ContractError::ArithmeticOverflow)?;
                    if destination_x < i64::from(left)
                        || destination_x >= i64::from(right)
                        || destination_x < 0
                        || destination_x >= i64::from(frame_width)
                    {
                        continue;
                    }
                    let alpha = mask.bitmap[y
                        .checked_mul(mask.width)
                        .and_then(|value| value.checked_add(x))
                        .ok_or(ContractError::ArithmeticOverflow)?];
                    if alpha == 0 {
                        continue;
                    }
                    let destination_y = usize::try_from(destination_y)
                        .map_err(|_| ContractError::ArithmeticOverflow)?;
                    let destination_x = usize::try_from(destination_x)
                        .map_err(|_| ContractError::ArithmeticOverflow)?;
                    let frame_width = usize::try_from(frame_width)
                        .map_err(|_| ContractError::ArithmeticOverflow)?;
                    let index = destination_y
                        .checked_mul(frame_width)
                        .and_then(|value| value.checked_add(destination_x))
                        .and_then(|value| value.checked_mul(4))
                        .ok_or(ContractError::ArithmeticOverflow)?;
                    let end = index
                        .checked_add(4)
                        .ok_or(ContractError::ArithmeticOverflow)?;
                    pixels[index..end].copy_from_slice(&[0, alpha, 0, u8::MAX]);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode, DecodeLimits, Sampler};
    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
    use std::io::Cursor;
    use std::sync::Arc;

    fn grid_with_height(
        width: u32,
        height: u32,
        dark: bool,
        sampling: Sampling,
        cell_height: u8,
    ) -> Arc<crate::SampleGrid> {
        let image = ImageBuffer::from_fn(width, height, |_x, _y| {
            if dark {
                Rgba([0, 0, 0, 255])
            } else {
                Rgba([255, 255, 255, 255])
            }
        });
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        let source = Arc::new(decode(&bytes.into_inner(), DecodeLimits::default()).unwrap());
        let mut sampler = Sampler::new(source);
        if sampling == Sampling::Average {
            sampler
                .sample_average_rect(5, u32::from(cell_height))
                .unwrap()
        } else {
            sampler.sample(5, sampling).unwrap()
        }
    }

    fn grid(width: u32, height: u32, dark: bool, sampling: Sampling) -> Arc<crate::SampleGrid> {
        grid_with_height(width, height, dark, sampling, 5)
    }

    #[test]
    fn repeated_classic_render_is_byte_stable_and_opaque() {
        let grid = grid(10, 10, true, Sampling::Classic);
        let options = RenderOptions {
            cell_size: 5,
            font_size: 8,
            ..RenderOptions::default()
        };
        let mut renderer = Renderer::new().unwrap();
        let first = renderer.render(&grid, options).unwrap();
        let second = renderer.render(&grid, options).unwrap();
        assert_eq!(
            &first.pixels()[..16],
            &[0, 64, 0, 255, 0, 140, 0, 255, 0, 173, 0, 255, 0, 0, 0, 255]
        );
        assert_eq!(first, second);
        assert!(first.pixels().chunks_exact(4).all(|pixel| pixel[3] == 255));
        assert!(first.pixels().chunks_exact(4).any(|pixel| pixel[1] > 0));
    }

    #[test]
    fn preview_longest_edge_is_bounded_for_narrow_grid() {
        let grid = grid(8_050, 5, false, Sampling::Classic);
        let options = RenderOptions {
            cell_size: 5,
            font_size: 8,
            ..RenderOptions::default()
        };
        let mut renderer = Renderer::new().unwrap();
        let preview = renderer.render_preview(&grid, options).unwrap();
        assert!(preview.width().max(preview.height()) <= 1600);
    }

    #[test]
    fn rows_concatenate_to_full_frame() {
        let grid = grid(20, 20, true, Sampling::Classic);
        let options = RenderOptions {
            cell_size: 5,
            font_size: 30,
            ..RenderOptions::default()
        };
        let mut renderer = Renderer::new().unwrap();
        let full = renderer.render(&grid, options).unwrap();
        let mut rows = Vec::new();
        rows.extend(renderer.render_rows(&grid, options, 0, 2).unwrap());
        rows.extend(renderer.render_rows(&grid, options, 2, 2).unwrap());
        assert_eq!(full.pixels(), rows.as_slice());
    }

    #[test]
    fn cache_capacity_is_respected_and_options_are_checked() {
        let grid = grid(10, 10, true, Sampling::Classic);
        let options = RenderOptions {
            cell_size: 5,
            font_size: 8,
            ..RenderOptions::default()
        };
        let mut renderer = Renderer::with_cache_capacity(1).unwrap();
        renderer.render(&grid, options).unwrap();
        assert!(renderer.cache_stats().bytes() <= renderer.cache_stats().capacity());
        assert!(renderer
            .render(
                &grid,
                RenderOptions {
                    font_size: 7,
                    ..options
                }
            )
            .is_err());
        assert!(renderer
            .render(
                &grid,
                RenderOptions {
                    max_output_bytes: 1,
                    ..options
                }
            )
            .is_err());
    }

    #[test]
    fn average_geometry_is_rectangular_and_measured() {
        let renderer = Renderer::new().unwrap();
        let geometry = renderer.aspect_geometry(5, 20).unwrap();
        assert!(geometry.source_cell_height > 0);
        assert!(geometry.output_cell_height > geometry.output_cell_width);
        let grid = grid_with_height(
            20,
            20,
            true,
            Sampling::Average,
            u8::try_from(geometry.source_cell_height).unwrap(),
        );
        let mut renderer = Renderer::new().unwrap();
        let frame = renderer
            .render(
                &grid,
                RenderOptions {
                    cell_size: 5,
                    sampling: Sampling::Average,
                    ..RenderOptions::default()
                },
            )
            .unwrap();
        assert_eq!(
            frame.width(),
            grid.width() * u32::from(geometry.output_cell_width)
        );
    }
}
