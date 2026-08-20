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
const DDSD_MIPMAPCOUNT: u32 = 0x20000;

/// Widest icon we will decode. Goods icons are small; this bounds the work a
/// hostile or mistaken file can cause in wasm.
const MAX_DIMENSION: u32 = 512;
const MAX_COA_DIMENSION: u32 = 1024;

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

/// Top mip of a DDS image as RGBA rows.
pub(crate) struct Decoded {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Decode the top mip of a DDS image into PNG bytes.
pub(crate) fn dds_to_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let image = dds_to_rgba_with_limit(bytes, MAX_DIMENSION)?;
    encode_png(image.width, image.height, &image.data)
}

/// Decode the smallest mip whose width/height are at least `min_width`/`min_height`.
///
/// CoA source art is often 768×512. Flags are composited at 64×42, so callers
/// pass that cover size and skip decoding the top mip when a smaller one exists.
pub(crate) fn coa_dds_to_rgba_covering(
    bytes: &[u8],
    min_width: u32,
    min_height: u32,
) -> Option<Decoded> {
    dds_to_rgba(bytes, MAX_COA_DIMENSION, Some((min_width, min_height)))
}

fn dds_to_rgba_with_limit(bytes: &[u8], max_dimension: u32) -> Option<Decoded> {
    dds_to_rgba(bytes, max_dimension, None)
}

fn dds_to_rgba(bytes: &[u8], max_dimension: u32, covering: Option<(u32, u32)>) -> Option<Decoded> {
    if bytes.get(..4)? != MAGIC || read_u32(bytes, 4)? as usize != HEADER_LEN {
        return None;
    }
    let height = read_u32(bytes, 4 + 8)?;
    let width = read_u32(bytes, 4 + 12)?;
    if width == 0 || height == 0 || width > max_dimension || height > max_dimension {
        return None;
    }

    let header_flags = read_u32(bytes, 8)?;
    let mip_count = if header_flags & DDSD_MIPMAPCOUNT != 0 {
        read_u32(bytes, 28)?.max(1)
    } else {
        1
    };

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
    let (mip_w, mip_h, mip_off, mip_len) = select_mip(width, height, mip_count, &format, covering)?;
    let mip = data.get(mip_off..mip_off + mip_len)?;
    let rgba = match format {
        Format::Block { bytes, decode } => decode_blocks(mip, mip_w, mip_h, bytes, decode)?,
        Format::Rgba32 {
            red,
            green,
            blue,
            alpha,
        } => decode_uncompressed(mip, mip_w, mip_h, red, green, blue, alpha)?,
    };
    Some(Decoded {
        width: mip_w,
        height: mip_h,
        data: rgba,
    })
}

fn mip_dims(width: u32, height: u32, level: u32) -> (u32, u32) {
    ((width >> level).max(1), (height >> level).max(1))
}

fn mip_byte_len(width: u32, height: u32, format: &Format) -> usize {
    match format {
        Format::Block { bytes, .. } => {
            let blocks_w = (width as usize).div_ceil(4);
            let blocks_h = (height as usize).div_ceil(4);
            blocks_w * blocks_h * bytes
        }
        Format::Rgba32 { .. } => width as usize * height as usize * 4,
    }
}

/// Smallest mip that still covers `covering`, or mip 0 if none do / no cover asked.
fn select_mip(
    width: u32,
    height: u32,
    mip_count: u32,
    format: &Format,
    covering: Option<(u32, u32)>,
) -> Option<(u32, u32, usize, usize)> {
    let size0 = mip_byte_len(width, height, format);
    let mut best = (width, height, 0usize, size0);
    let Some((min_w, min_h)) = covering else {
        return Some(best);
    };
    let mut offset = size0;
    for level in 1..mip_count {
        let (mw, mh) = mip_dims(width, height, level);
        let size = mip_byte_len(mw, mh, format);
        if mw >= min_w && mh >= min_h {
            best = (mw, mh, offset, size);
        } else {
            break;
        }
        offset += size;
    }
    Some(best)
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
    for pixel in data.as_chunks::<4>().0 {
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
        encoder.set_compression(png::Compression::Fast);
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
        dds_with_mips(width, height, 1, pixel_format, data)
    }

    fn dds_with_mips(
        width: u32,
        height: u32,
        mip_count: u32,
        pixel_format: [u32; 8],
        data: &[u8],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        let mut header = [0u32; 31];
        header[0] = HEADER_LEN as u32;
        if mip_count > 1 {
            header[1] = DDSD_MIPMAPCOUNT;
            header[6] = mip_count;
        }
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

    #[test]
    fn coa_limit_accepts_768_by_512_without_relaxing_icons() {
        let data = vec![0u8; (768 / 4) * (512 / 4) * 8];
        let image = dds(768, 512, four_cc(b"DXT1"), &data);
        assert!(dds_to_png(&image).is_none());
        let decoded =
            coa_dds_to_rgba_covering(&image, 768, 512).expect("real CoA dimensions are allowed");
        assert_eq!((decoded.width, decoded.height), (768, 512));
        assert_eq!(decoded.data.len(), 768 * 512 * 4);
    }

    #[test]
    fn coa_covering_picks_smallest_mip_that_covers_flag_size() {
        // 256×128 mip0, 128×64 mip1, 64×32 mip2. Covering 64×42 should take 128×64.
        let mip0 = (256 / 4) * (128 / 4) * 8;
        let mip1 = (128 / 4) * (64 / 4) * 8;
        let mip2 = (64 / 4) * (32 / 4) * 8;
        let data = vec![0u8; mip0 + mip1 + mip2];
        let image = dds_with_mips(256, 128, 3, four_cc(b"DXT1"), &data);
        let decoded = coa_dds_to_rgba_covering(&image, 64, 42).expect("covering mip");
        assert_eq!((decoded.width, decoded.height), (128, 64));
        assert_eq!(decoded.data.len(), 128 * 64 * 4);
    }
}
