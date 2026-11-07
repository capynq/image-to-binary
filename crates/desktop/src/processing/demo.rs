//! Deterministic demo fixture, ported from `frontend/src/processing/demoImage.ts`. The browser
//! and the shell must produce byte-identical RGBA8, so both are checked against one FNV-1a hash.

pub(crate) const DEMO_NAME: &str = "Demo fixture";
pub(crate) const DEMO_WIDTH: u32 = 256;
pub(crate) const DEMO_HEIGHT: u32 = 192;

/// Writes the fixture into `pixels`, which must hold exactly [`DEMO_RGBA_BYTES`] bytes. Tiles, a
/// ring and colour ramps; integer ramps and the single square root match the TypeScript source.
pub(crate) fn fill_demo(pixels: &mut [u8]) {
    let row_bytes = DEMO_WIDTH as usize * 4;
    for (y, row) in pixels.chunks_exact_mut(row_bytes).enumerate() {
        let y = u32::try_from(y).unwrap_or(u32::MAX);
        let ramp_y = to_byte(y * 255 / (DEMO_HEIGHT - 1));
        let dy = f64::from(y) - f64::from(DEMO_HEIGHT) / 2.0;
        for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
            let x = u32::try_from(x).unwrap_or(u32::MAX);
            let ramp_x = to_byte(x * 255 / (DEMO_WIDTH - 1));
            let dx = f64::from(x) - f64::from(DEMO_WIDTH) / 2.0;
            let ring = ((dx * dx + dy * dy).sqrt() - 72.0).abs() < 9.0;
            let tile = ((x >> 5) + (y >> 5)) & 1;
            let luma = if ring {
                16
            } else if tile == 0 {
                ramp_x
            } else {
                255 - ramp_x
            };
            pixel[0] = luma;
            pixel[1] = if ring { 64 } else { luma };
            pixel[2] = ramp_y;
            pixel[3] = 255;
        }
    }
}

fn to_byte(value: u32) -> u8 {
    u8::try_from(value).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEMO_RGBA_BYTES: usize = (DEMO_WIDTH as usize) * (DEMO_HEIGHT as usize) * 4;

    /// FNV-1a 64 of the TypeScript fixture (`createDemoImage().pixels`), recorded in
    /// `frontend/src/processing/demoImage.test.ts` under the same constant.
    const SHARED_FNV1A64: u64 = 0xbe5c_6ff2_f1f9_0429;

    fn fnv1a64(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    #[test]
    fn the_native_fixture_hashes_to_the_browser_value() {
        let mut pixels = vec![0_u8; DEMO_RGBA_BYTES];
        fill_demo(&mut pixels);
        assert_eq!(fnv1a64(&pixels), SHARED_FNV1A64);
    }

    #[test]
    fn the_fixture_has_the_documented_geometry_and_opaque_alpha() {
        assert_eq!(DEMO_RGBA_BYTES, 196_608);
        let mut pixels = vec![0_u8; DEMO_RGBA_BYTES];
        fill_demo(&mut pixels);
        assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
        assert_eq!(pixels[2], 0);
        assert_eq!(
            pixels[(DEMO_HEIGHT as usize - 1) * DEMO_WIDTH as usize * 4 + 2],
            255
        );
    }
}
