//! Goods icons: DDS from the game install, PNG in the blob.
//!
//! Browsers cannot draw DDS, and the icons are block-compressed, so the top
//! mip is decoded to RGBA here and re-encoded as PNG. Anything unrecognized
//! yields `None`: a missing icon degrades the UI, it does not fail a build.

const MAGIC: &[u8; 4] = b"DDS ";
const HEADER_LEN: usize = 124;
const DX10_HEADER_LEN: usize = 20;
const PIXEL_FORMAT: usize = 4 + 72;

const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;

/// Widest icon we will decode. Goods icons are small; this bounds the work a
/// hostile or mistaken file can cause in wasm.
const MAX_DIMENSION: u32 = 512;

/// Decodes one compressed block into RGBA rows of the given pitch.
type BlockDecoder = fn(&[u8], &mut [u8], usize);

enum Format {
    /// Block-compressed: bytes per block and a `bcdec_rs` decoder for one block.
    Block { bytes: usize, decode: BlockDecoder },
    /// Uncompressed 32-bit; byte offset of each channel within a pixel.
    Rgba32 {
        red: usize,
        green: usize,
        blue: usize,
        alpha: Option<usize>,
    },
}

/// Decode the top mip of a DDS image into PNG bytes.
pub(crate) fn dds_to_png(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.get(..4)? != MAGIC || read_u32(bytes, 4)? as usize != HEADER_LEN {
        return None;
    }
    let height = read_u32(bytes, 4 + 8)?;
    let width = read_u32(bytes, 4 + 12)?;
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return None;
    }

    let flags = read_u32(bytes, PIXEL_FORMAT + 4)?;
    let four_cc = bytes.get(PIXEL_FORMAT + 8..PIXEL_FORMAT + 12)?;
    let mut data_start = 4 + HEADER_LEN;
    let format = if flags & DDPF_FOURCC != 0 {
        if four_cc == b"DX10" {
            data_start += DX10_HEADER_LEN;
            dxgi_format(read_u32(bytes, 4 + HEADER_LEN)?)?
        } else {
            four_cc_format(four_cc)?
        }
    } else if flags & DDPF_RGB != 0 {
        uncompressed_format(bytes, flags)?
    } else {
        return None;
    };

    let data = bytes.get(data_start..)?;
    let rgba = match format {
        Format::Block { bytes, decode } => decode_blocks(data, width, height, bytes, decode)?,
        Format::Rgba32 {
            red,
            green,
            blue,
            alpha,
        } => decode_uncompressed(data, width, height, red, green, blue, alpha)?,
    };
    encode_png(width, height, &rgba)
}

fn four_cc_format(four_cc: &[u8]) -> Option<Format> {
    let (bytes, decode): (usize, BlockDecoder) = match four_cc {
        b"DXT1" => (8, bcdec_rs::bc1),
        b"DXT2" | b"DXT3" => (16, bcdec_rs::bc2),
        b"DXT4" | b"DXT5" => (16, bcdec_rs::bc3),
        _ => return None,
    };
    Some(Format::Block { bytes, decode })
}

fn dxgi_format(dxgi: u32) -> Option<Format> {
    // Values from the DXGI_FORMAT enum; each BCn has UNORM and SRGB variants.
    let (bytes, decode): (usize, BlockDecoder) = match dxgi {
        70..=72 => (8, bcdec_rs::bc1),
        73..=75 => (16, bcdec_rs::bc2),
        76..=78 => (16, bcdec_rs::bc3),
        97..=99 => (16, bcdec_rs::bc7),
        // R8G8B8A8 and B8G8R8A8 families.
        27..=31 => {
            return Some(Format::Rgba32 {
                red: 0,
                green: 1,
                blue: 2,
                alpha: Some(3),
            })
        }
        87..=91 => {
            return Some(Format::Rgba32 {
                red: 2,
                green: 1,
                blue: 0,
                alpha: Some(3),
            })
        }
        _ => return None,
    };
    Some(Format::Block { bytes, decode })
}

fn uncompressed_format(bytes: &[u8], flags: u32) -> Option<Format> {
    if read_u32(bytes, PIXEL_FORMAT + 12)? != 32 {
        return None;
    }
    let alpha_mask = read_u32(bytes, PIXEL_FORMAT + 28)?;
    Some(Format::Rgba32 {
        red: channel_offset(read_u32(bytes, PIXEL_FORMAT + 16)?)?,
        green: channel_offset(read_u32(bytes, PIXEL_FORMAT + 20)?)?,
        blue: channel_offset(read_u32(bytes, PIXEL_FORMAT + 24)?)?,
        alpha: (flags & DDPF_ALPHAPIXELS != 0)
            .then(|| channel_offset(alpha_mask))
            .flatten(),
    })
}

/// Byte index of a channel mask within a little-endian 32-bit pixel.
fn channel_offset(mask: u32) -> Option<usize> {
    match mask {
        0x0000_00FF => Some(0),
        0x0000_FF00 => Some(1),
        0x00FF_0000 => Some(2),
        0xFF00_0000 => Some(3),
        _ => None,
    }
}

fn decode_blocks(
    data: &[u8],
    width: u32,
    height: u32,
    block_bytes: usize,
    decode: fn(&[u8], &mut [u8], usize),
) -> Option<Vec<u8>> {
    let width = width as usize;
    let height = height as usize;
    let mut out = vec![0u8; width * height * 4];
    // Decode into a full 4x4 tile, then copy only the pixels the image has:
    // dimensions need not be block-aligned.
    let mut tile = [0u8; 4 * 4 * 4];
    let mut offset = 0;
    for top in (0..height).step_by(4) {
        for left in (0..width).step_by(4) {
            let block = data.get(offset..offset + block_bytes)?;
            offset += block_bytes;
            decode(block, &mut tile, 4 * 4);
            for row in 0..4.min(height - top) {
                let columns = 4.min(width - left);
                let source = row * 16;
                let target = ((top + row) * width + left) * 4;
                out[target..target + columns * 4]
                    .copy_from_slice(&tile[source..source + columns * 4]);
            }
        }
    }
    Some(out)
}

fn decode_uncompressed(
    data: &[u8],
    width: u32,
    height: u32,
    red: usize,
    green: usize,
    blue: usize,
    alpha: Option<usize>,
) -> Option<Vec<u8>> {
    let pixels = (width as usize).checked_mul(height as usize)?;
    let data = data.get(..pixels * 4)?;
    let mut out = Vec::with_capacity(pixels * 4);
    for pixel in data.chunks_exact(4) {
        out.push(pixel[red]);
        out.push(pixel[green]);
        out.push(pixel[blue]);
        out.push(alpha.map_or(0xFF, |index| pixel[index]));
    }
    Some(out)
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
        writer.finish().ok()?;
    }
    Some(out)
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let raw: [u8; 4] = bytes.get(offset..offset + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal DDS: header, then one block or pixel run of image data.
    fn dds(width: u32, height: u32, pixel_format: [u32; 8], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        let mut header = [0u32; 31];
        header[0] = HEADER_LEN as u32;
        header[2] = height;
        header[3] = width;
        header[18..26].copy_from_slice(&pixel_format);
        for word in header {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out.extend_from_slice(data);
        out
    }

    fn four_cc(tag: &[u8; 4]) -> [u32; 8] {
        [32, DDPF_FOURCC, u32::from_le_bytes(*tag), 0, 0, 0, 0, 0]
    }

    fn png_dimensions(png: &[u8]) -> (u32, u32) {
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        (width, height)
    }

    #[test]
    fn decodes_dxt5_to_png() {
        let image = dds(4, 4, four_cc(b"DXT5"), &[0u8; 16]);
        let png = dds_to_png(&image).expect("DXT5 is the common icon format");
        assert_eq!(png.get(1..4), Some(&b"PNG"[..]));
        assert_eq!(png_dimensions(&png), (4, 4));
    }

    #[test]
    fn decodes_uncompressed_bgra_in_channel_order() {
        let format = [
            32,
            DDPF_RGB | DDPF_ALPHAPIXELS,
            0,
            32,
            0x00FF_0000,
            0x0000_FF00,
            0x0000_00FF,
            0xFF00_0000,
        ];
        // One pixel stored B, G, R, A.
        let image = dds(1, 1, format, &[0x33, 0x22, 0x11, 0x44]);
        let png = dds_to_png(&image).expect("uncompressed BGRA is supported");
        assert_eq!(png_dimensions(&png), (1, 1));
    }

    #[test]
    fn decodes_dimensions_that_are_not_block_aligned() {
        let image = dds(3, 3, four_cc(b"DXT1"), &[0u8; 8]);
        let png = dds_to_png(&image).expect("partial blocks must not overflow");
        assert_eq!(png_dimensions(&png), (3, 3));
    }

    #[test]
    fn rejects_truncated_and_unknown_images() {
        assert!(dds_to_png(b"not a dds").is_none());
        // Header promises 4x4 DXT5 but carries no block data.
        assert!(dds_to_png(&dds(4, 4, four_cc(b"DXT5"), &[])).is_none());
        assert!(dds_to_png(&dds(4, 4, four_cc(b"WXYZ"), &[0u8; 16])).is_none());
    }
}
