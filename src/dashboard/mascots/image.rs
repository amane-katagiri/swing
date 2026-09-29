const MAX_SPRITE_SIDE: u32 = 4096;
const MAX_SPRITE_PIXELS: u64 = 2048 * 2048;

fn sniff_image(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    let u16_le = |at: usize| -> Option<u32> {
        Some(u32::from(u16::from_le_bytes(
            bytes.get(at..at + 2)?.try_into().ok()?,
        )))
    };
    let u24_le = |at: usize| -> Option<u32> {
        let b = bytes.get(at..at + 3)?;
        Some(u32::from(b[0]) | u32::from(b[1]) << 8 | u32::from(b[2]) << 16)
    };
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        if bytes.get(12..16)? != b"IHDR" {
            return None;
        }
        let width = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
        let height = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
        return Some(("image/png", width, height));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(("image/gif", u16_le(6)?, u16_le(8)?));
    }
    if bytes.get(0..4)? == b"RIFF" && bytes.get(8..12)? == b"WEBP" {
        let (width, height) = match bytes.get(12..16)? {
            b"VP8 " => {
                if bytes.get(23..26)? != [0x9d, 0x01, 0x2a] {
                    return None;
                }
                (u16_le(26)? & 0x3fff, u16_le(28)? & 0x3fff)
            }
            b"VP8L" => {
                if *bytes.get(20)? != 0x2f {
                    return None;
                }
                let bits = u32::from_le_bytes(bytes.get(21..25)?.try_into().ok()?);
                ((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1)
            }
            b"VP8X" => (u24_le(24)? + 1, u24_le(27)? + 1),
            _ => return None,
        };
        return Some(("image/webp", width, height));
    }
    None
}

pub fn check_sprite(bytes: &[u8], content_type: &str) -> Result<(), String> {
    let Some((detected, width, height)) = sniff_image(bytes) else {
        return Err("is not a readable PNG/GIF/WebP image".to_string());
    };
    if detected != content_type {
        return Err(format!(
            "is {detected} but its extension says {content_type}"
        ));
    }
    if width == 0 || height == 0 {
        return Err("has a zero width or height".to_string());
    }
    if width > MAX_SPRITE_SIDE
        || height > MAX_SPRITE_SIDE
        || u64::from(width) * u64::from(height) > MAX_SPRITE_PIXELS
    {
        return Err(format!(
            "is {width}x{height} px, over the {MAX_SPRITE_SIDE} px side / {MAX_SPRITE_PIXELS} pixel limit"
        ));
    }
    Ok(())
}

#[cfg(test)]
pub fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 3, 0, 0, 0]);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn webp(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut bytes = b"RIFF\0\0\0\0WEBP".to_vec();
        bytes.extend_from_slice(fourcc);
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn image_headers_are_parsed() {
        assert_eq!(sniff_image(&png(384, 96)), Some(("image/png", 384, 96)));
        assert_eq!(
            sniff_image(b"GIF89a\x40\x01\xf0\x00\x80\0\0"),
            Some(("image/gif", 320, 240))
        );
        assert_eq!(
            sniff_image(b"GIF87a\x08\0\x10\0"),
            Some(("image/gif", 8, 16))
        );
        assert_eq!(
            sniff_image(&webp(
                b"VP8 ",
                &[0x10, 0x02, 0x00, 0x9d, 0x01, 0x2a, 0x40, 0x01, 0xf0, 0x00]
            )),
            Some(("image/webp", 320, 240))
        );
        let (w, h) = (320u32 - 1, 240u32 - 1);
        let bits = w | h << 14;
        let mut vp8l = vec![0x2f];
        vp8l.extend_from_slice(&bits.to_le_bytes());
        assert_eq!(
            sniff_image(&webp(b"VP8L", &vp8l)),
            Some(("image/webp", 320, 240))
        );
        assert_eq!(
            sniff_image(&webp(
                b"VP8X",
                &[0x10, 0, 0, 0, 0x3f, 0x01, 0x00, 0xef, 0x00, 0x00]
            )),
            Some(("image/webp", 320, 240))
        );
    }

    #[test]
    fn truncated_or_garbage_image_headers_are_rejected() {
        let full = png(8, 8);
        for len in 0..24 {
            assert_eq!(sniff_image(&full[..len]), None, "png truncated to {len}");
        }
        assert_eq!(sniff_image(b"GIF89a\x08\0\x08"), None);
        assert_eq!(sniff_image(b"GIF88a\x08\0\x08\0"), None);
        let vp8 = webp(
            b"VP8 ",
            &[0x10, 0x02, 0x00, 0x9d, 0x01, 0x2a, 0x40, 0x01, 0xf0, 0x00],
        );
        for len in 0..30 {
            assert_eq!(sniff_image(&vp8[..len]), None, "vp8 truncated to {len}");
        }
        assert_eq!(
            sniff_image(&webp(
                b"VP8 ",
                &[0x10, 0x02, 0x00, 0, 0, 0, 0x40, 0x01, 0xf0, 0x00]
            )),
            None
        );
        assert_eq!(sniff_image(&webp(b"VP8L", &[0x00, 0, 0, 0, 0])), None);
        assert_eq!(sniff_image(&webp(b"VP9 ", &[0; 10])), None);
        assert_eq!(sniff_image(b"not an image at all, just text"), None);
        let mut bad_chunk = png(8, 8);
        bad_chunk[12..16].copy_from_slice(b"IDAT");
        assert_eq!(sniff_image(&bad_chunk), None);
    }

    #[test]
    fn sprite_dimensions_are_limited() {
        assert!(check_sprite(&png(4096, 1024), "image/png").is_ok());
        assert!(check_sprite(&png(2048, 2048), "image/png").is_ok());
        assert!(check_sprite(&png(4097, 8), "image/png").is_err());
        assert!(check_sprite(&png(8, 4097), "image/png").is_err());
        assert!(check_sprite(&png(4096, 2048), "image/png").is_err());
        assert!(check_sprite(&png(u32::MAX, u32::MAX), "image/png").is_err());
        assert!(check_sprite(&png(0, 8), "image/png").is_err());
        assert!(check_sprite(&png(8, 8), "image/gif").is_err());
        assert!(check_sprite(b"x", "image/png").is_err());
    }
}
