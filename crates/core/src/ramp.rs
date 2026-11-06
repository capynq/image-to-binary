use crate::ContractError;

const CAPACITY: usize = 64;

/// Validated printable ASCII ramp with fixed inline storage.
///
/// Invariants: the length is `1..=64` and every stored byte is in `0x20..=0x7E`,
/// so the text is always valid UTF-8. Entries are ordered from least ink to most ink.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ramp {
    bytes: [u8; CAPACITY],
    len: usize,
}

impl Ramp {
    /// Default ASCII ramp, ordered from least ink to most ink.
    pub const DEFAULT_ASCII: Self = match Self::new(" .:-=+*#%@") {
        Ok(ramp) => ramp,
        Err(_) => panic!("default ASCII ramp is valid"),
    };

    /// Validates printable ASCII text as a ramp.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidRamp`] when the text is empty, longer than
    /// 64 bytes, or contains a byte outside `0x20..=0x7E`.
    pub const fn new(text: &str) -> Result<Self, ContractError> {
        let source = text.as_bytes();
        if source.is_empty() || source.len() > CAPACITY {
            return Err(ContractError::InvalidRamp);
        }
        let mut bytes = [0; CAPACITY];
        let mut index = 0;
        while index < source.len() {
            let byte = source[index];
            if byte < 0x20 || byte > 0x7e {
                return Err(ContractError::InvalidRamp);
            }
            bytes[index] = byte;
            index += 1;
        }
        Ok(Self {
            bytes,
            len: source.len(),
        })
    }

    /// Returns the ramp text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(self.as_bytes()).unwrap_or_default()
    }

    /// Returns the ramp bytes, all printable ASCII.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.get(..self.len).unwrap_or_default()
    }

    /// Returns the number of entries, always between 1 and 64.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Always returns `false`; a validated ramp is never empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn printable(count: usize) -> String {
        (0x21..0x21 + count)
            .map(|byte| char::from(u8::try_from(byte).unwrap_or(b'?')))
            .collect()
    }

    #[test]
    fn accepts_lengths_one_through_sixty_four_and_space() {
        assert_eq!(Ramp::new("#").unwrap().len(), 1);
        assert_eq!(Ramp::new(" ").unwrap().as_str(), " ");
        let full = printable(64);
        let ramp = Ramp::new(&full).unwrap();
        assert_eq!(ramp.len(), 64);
        assert_eq!(ramp.as_str(), full);
        assert!(!ramp.is_empty());
    }

    #[test]
    fn rejects_empty_oversized_and_non_printable_text() {
        let oversized = printable(65);
        for text in [
            "",
            oversized.as_str(),
            "\u{e9}",
            "a\u{e9}",
            "\n",
            "\t",
            "\u{1f}",
            "\u{7f}",
        ] {
            assert_eq!(Ramp::new(text), Err(ContractError::InvalidRamp), "{text:?}");
        }
    }

    #[test]
    fn default_ascii_ramp_is_ordered_from_least_to_most_ink() {
        assert_eq!(Ramp::DEFAULT_ASCII.as_str(), " .:-=+*#%@");
        assert_eq!(Ramp::DEFAULT_ASCII.as_bytes().first(), Some(&b' '));
        assert_eq!(Ramp::DEFAULT_ASCII.as_bytes().last(), Some(&b'@'));
    }

    #[test]
    fn ramp_is_copy_and_compares_by_content() {
        let original = Ramp::new("ab").unwrap();
        let copied = original;
        assert_eq!(original, copied);
        assert_ne!(original, Ramp::new("abc").unwrap());
    }
}
