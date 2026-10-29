//! What can be known about an image file from its first bytes: the format and the size in
//! pixels, which the API checks against fixed limits and rejects the whole request over.

/// The smallest width or height the API accepts, in pixels.
pub const MIN_IMAGE_SIDE: u32 = 256;

/// The largest width or height the API accepts, in pixels.
pub const MAX_IMAGE_SIDE: u32 = 1568;

/// The fewest pixels (width times height) the API accepts.
pub const MIN_IMAGE_PIXELS: u64 = 78_400;

/// The most pixels (width times height) the API accepts.
pub const MAX_IMAGE_PIXELS: u64 = 2_458_624;

/// The bytes every PNG file starts with.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// The bytes every JPEG file starts with: the start-of-image marker and the start of a segment.
const JPEG_SIGNATURE: [u8; 3] = [0xFF, 0xD8, 0xFF];

/// An image's format and size, read from its header without decoding it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImageInfo {
    /// `png` or `jpg`, as the API spells the format.
    pub format: &'static str,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl ImageInfo {
    /// Reads the format and the size from the first bytes of a PNG or JPEG file. `None` when the
    /// bytes are neither, or the header is cut off.
    ///
    /// ```
    /// use jev_client::ImageInfo;
    ///
    /// let png = [
    ///     0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R',
    ///     0, 0, 1, 144, 0, 0, 1, 44,
    /// ];
    /// let info = ImageInfo::read(&png).unwrap();
    /// assert_eq!((info.format, info.width, info.height), ("png", 400, 300));
    /// ```
    #[must_use]
    pub fn read(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(&PNG_SIGNATURE) {
            // The IHDR chunk comes first: a 4-byte length, the type, then width and height.
            let ihdr = bytes.get(12..)?;
            if ihdr.get(..4)? != b"IHDR" {
                return None;
            }
            return Some(Self {
                format: "png",
                width: be_u32(ihdr.get(4..)?)?,
                height: be_u32(ihdr.get(8..)?)?,
            });
        }
        if bytes.starts_with(&JPEG_SIGNATURE) {
            return jpeg_size(bytes).map(|(width, height)| Self {
                format: "jpg",
                width,
                height,
            });
        }
        None
    }

    /// Why the API would reject an image of this size, if it would.
    #[must_use]
    pub fn size_problem(&self) -> Option<String> {
        let pixels = u64::from(self.width) * u64::from(self.height);
        let side_ok = |side: u32| (MIN_IMAGE_SIDE..=MAX_IMAGE_SIDE).contains(&side);
        if !side_ok(self.width) || !side_ok(self.height) {
            return Some(format!(
                "{}x{} pixels; each side must be {MIN_IMAGE_SIDE} to {MAX_IMAGE_SIDE}",
                self.width, self.height
            ));
        }
        if !(MIN_IMAGE_PIXELS..=MAX_IMAGE_PIXELS).contains(&pixels) {
            return Some(format!(
                "{}x{} = {pixels} pixels; the total must be {MIN_IMAGE_PIXELS} to {MAX_IMAGE_PIXELS}",
                self.width, self.height
            ));
        }
        None
    }
}

/// The big-endian number in the first four bytes, when there are four.
fn be_u32(bytes: &[u8]) -> Option<u32> {
    bytes.get(..4)?.try_into().ok().map(u32::from_be_bytes)
}

/// The big-endian number in the first two bytes, when there are two.
fn be_u16(bytes: &[u8]) -> Option<u16> {
    bytes.get(..2)?.try_into().ok().map(u16::from_be_bytes)
}

/// Walks the segments after the start-of-image marker to the first start-of-frame, which holds
/// the height and the width.
fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    loop {
        // Padding bytes between segments are allowed.
        while *bytes.get(at)? == 0xFF {
            at += 1;
        }
        let marker = *bytes.get(at)?;
        at += 1;
        match marker {
            // Start of frame, baseline through lossless, except the markers that reuse the range.
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                // After the length and the sample precision come the height and the width.
                let frame = bytes.get(at + 3..)?;
                let height = u32::from(be_u16(frame)?);
                let width = u32::from(be_u16(frame.get(2..)?)?);
                return Some((width, height));
            }
            // Stand-alone markers carry no length.
            0xD0..=0xD9 | 0x01 => {}
            // Start of scan: the frame should have come before it.
            0xDA => return None,
            _ => {
                at += usize::from(be_u16(bytes.get(at..)?)?);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ImageInfo, MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE, MIN_IMAGE_SIDE};

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes
    }

    fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8];
        // An APP0 segment first, as every JFIF file has.
        bytes.extend_from_slice(&[0xFF, 0xE0, 0, 16]);
        bytes.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        // Then a baseline start of frame: length, precision, height, width, components.
        bytes.extend_from_slice(&[0xFF, 0xFF, 0xC0, 0, 11, 8]);
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&[1, 1, 0x11, 0]);
        bytes
    }

    #[test]
    fn png_and_jpeg_headers_give_the_format_and_the_size() {
        let info = ImageInfo::read(&png(400, 300)).unwrap();
        assert_eq!((info.format, info.width, info.height), ("png", 400, 300));
        let info = ImageInfo::read(&jpeg(1568, 256)).unwrap();
        assert_eq!((info.format, info.width, info.height), ("jpg", 1568, 256));
    }

    #[test]
    fn anything_else_or_a_cut_off_header_is_none() {
        assert_eq!(ImageInfo::read(b"GIF89a"), None);
        assert_eq!(ImageInfo::read(png(400, 300).get(..20).unwrap()), None);
        assert_eq!(ImageInfo::read(jpeg(400, 300).get(..10).unwrap()), None);
        assert_eq!(ImageInfo::read(&[0xFF, 0xD8, 0xFF, 0xDA, 0, 2]), None);
    }

    #[test]
    fn the_documented_limits_are_checked_on_each_side_and_on_the_total() {
        let problem = |width, height| ImageInfo::read(&png(width, height)).unwrap().size_problem();
        assert_eq!(problem(MIN_IMAGE_SIDE, 320), None);
        assert_eq!(problem(MAX_IMAGE_SIDE, MAX_IMAGE_SIDE), None);
        assert!(problem(255, 400).unwrap().contains("each side"));
        assert!(problem(1569, 400).unwrap().contains("each side"));
        assert!(problem(256, 300).unwrap().contains("the total"));
        assert_eq!(MAX_IMAGE_PIXELS, 1568 * 1568);
    }
}
