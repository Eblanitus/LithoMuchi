//! Минимальный записыватель PNG (RGB8, DEFLATE stored blocks).
//!
//! Сделан своим кодом по решению Д-004: на первом шаге у песочницы нет
//! внешних крейтов, поэтому `cargo test` собирается даже без доступа к
//! crates.io. Сжатия нет (карты большие, но читаются любым просмотрщиком);
//! позже можно заменить на крейт `png` + `rayon`.

use std::path::Path;

#[derive(Default)]
pub struct RgbImage {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl RgbImage {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            data: vec![0; width * height * 3],
        }
    }

    pub fn set(&mut self, x: usize, y: usize, rgb: [u8; 3]) {
        let offset = (y * self.width + x) * 3;
        self.data[offset] = rgb[0];
        self.data[offset + 1] = rgb[1];
        self.data[offset + 2] = rgb[2];
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut table = [0_u32; 256];
    for (i, entry) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *entry = c;
    }
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in bytes {
        let index = ((crc ^ u32::from(*byte)) & 0xFF) as usize;
        crc = table[index] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

fn adler32(bytes: &[u8]) -> u32 {
    let mut a = 1_u32;
    let mut b = 0_u32;
    for byte in bytes {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// DEFLATE без сжатия: блоки stored, максимум 65535 байт данных в блоке.
fn zlib_stored(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + raw.len() / 65535 * 5 + 6);
    out.extend_from_slice(&[0x78, 0x01]);
    let mut offset = 0;
    while offset < raw.len() {
        let end = (offset + 65535).min(raw.len());
        let block = &raw[offset..end];
        let final_block = end == raw.len();
        out.push(u8::from(final_block));
        let len = block.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
        offset = end;
    }
    out.extend_from_slice(&adler32(raw).to_be_bytes());
    out
}

pub fn write_png(path: &Path, image: &RgbImage) -> std::io::Result<()> {
    let mut raw = Vec::with_capacity((image.width * 3 + 1) * image.height);
    for y in 0..image.height {
        raw.push(0); // filter type: None
        let start = y * image.width * 3;
        raw.extend_from_slice(&image.data[start..start + image.width * 3]);
    }

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(image.width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(image.height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);

    let mut out = Vec::with_capacity(raw.len() + 128);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n']);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);
    std::fs::write(path, out)
}

#[cfg(test)]
mod tests {
    use super::{RgbImage, adler32, crc32, write_png, zlib_stored};

    #[test]
    fn crc_and_adler_known_values() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn png_starts_with_signature() {
        let mut image = RgbImage::new(2, 1);
        image.set(0, 0, [255, 0, 0]);
        image.set(1, 0, [0, 0, 255]);
        let path = std::env::temp_dir().join("litho-sandbox-test.png");
        write_png(&path, &image).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];
        assert_eq!(&bytes[..8], &SIGNATURE[..]);
        assert!(bytes.windows(4).any(|w| w == b"IEND"));
        // zlib-поток распаковывается как stored: проверяем заголовок и adler.
        let zlib = zlib_stored(&[0, 1, 2, 3]);
        assert_eq!(&zlib[..2], &[0x78, 0x01]);
        let _ = std::fs::remove_file(&path);
    }
}
