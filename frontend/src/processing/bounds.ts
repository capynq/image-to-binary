/**
 * Numeric limits of the render settings. The Rust core is the single source of these values:
 * a contract test parses each `export const NAME = <integer>` line, so keep that exact form.
 */
export const MIN_CELL_SIZE = 5
export const MAX_CELL_SIZE = 30
export const MIN_FONT_SIZE = 8
export const MAX_FONT_SIZE = 30
export const MIN_THRESHOLD = 0
export const MAX_THRESHOLD = 255
export const MIN_CONTRAST = -100
export const MAX_CONTRAST = 100
export const MAX_RAMP_LENGTH = 64
