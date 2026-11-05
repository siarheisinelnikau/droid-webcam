//! Bounded frame packets for the local Windows pipe, and NV12 output conversion.
use crate::{MAX_NV12_SIZE, VideoMode};

pub const HEADER_SIZE: usize = 24;
pub const MAX_PACKET_SIZE: usize = HEADER_SIZE + MAX_NV12_SIZE;

pub fn encode_packet(
    mode: VideoMode,
    captured_ms: u64,
    pixels: &[u8],
    packet: &mut Vec<u8>,
) -> Result<(), &'static str> {
    if pixels.len() != mode.nv12_size() {
        return Err("incorrect NV12 frame length");
    }
    packet.resize(HEADER_SIZE + pixels.len(), 0);
    packet[..4].copy_from_slice(b"DWF2");
    packet[4..10].copy_from_slice(&mode.wire());
    packet[10..12].fill(0);
    packet[12..20].copy_from_slice(&captured_ms.to_le_bytes());
    packet[20..24].copy_from_slice(&(pixels.len() as u32).to_le_bytes());
    packet[HEADER_SIZE..].copy_from_slice(pixels);
    Ok(())
}

pub fn decode_packet(packet: &[u8]) -> Result<(VideoMode, u64, &[u8]), &'static str> {
    if packet.len() < HEADER_SIZE
        || packet.len() > MAX_PACKET_SIZE
        || &packet[..4] != b"DWF2"
        || packet[10..12] != [0, 0]
    {
        return Err("invalid frame packet header");
    }
    let mode = VideoMode::from_wire(packet[4..10].try_into().unwrap())?;
    let size = u32::from_le_bytes(packet[20..24].try_into().unwrap()) as usize;
    if size != mode.nv12_size() || packet.len() != HEADER_SIZE + size {
        return Err("invalid frame packet length");
    }
    let timestamp = u64::from_le_bytes(packet[12..20].try_into().unwrap());
    Ok((mode, timestamp, &packet[HEADER_SIZE..]))
}

pub fn black_frame(mode: VideoMode) -> Vec<u8> {
    let mut pixels = vec![128; mode.nv12_size()];
    pixels[..mode.width() * mode.height()].fill(16);
    pixels
}

/// Fit the input in the application's selected output mode, preserving aspect ratio.
pub fn render_nv12(
    input: &[u8],
    source: VideoMode,
    target: VideoMode,
    output: &mut [u8],
) -> Result<(), &'static str> {
    if input.len() != source.nv12_size() || output.len() != target.nv12_size() {
        return Err("incorrect NV12 buffer length");
    }
    if source.width() == target.width() && source.height() == target.height() {
        output.copy_from_slice(input);
        return Ok(());
    }
    let (sw, sh, tw, th) = (
        source.width(),
        source.height(),
        target.width(),
        target.height(),
    );
    let (width, height) = if sw * th > tw * sh {
        (tw, (tw * sh / sw) & !1)
    } else {
        ((th * sw / sh) & !1, th)
    };
    let left = ((tw - width) / 2) & !1;
    let top = ((th - height) / 2) & !1;
    let (y, uv) = output.split_at_mut(tw * th);
    y.fill(16);
    uv.fill(128);
    for row in 0..height {
        let source_row = (row * sh / height) * sw;
        let output_row = (row + top) * tw + left;
        for col in 0..width {
            y[output_row + col] = input[source_row + col * sw / width];
        }
    }
    let input_uv = &input[sw * sh..];
    for row in 0..height / 2 {
        let source_row = (row * (sh / 2) / (height / 2)) * sw;
        let output_row = (row + top / 2) * tw + left;
        for col in 0..width / 2 {
            let source_col = (col * (sw / 2) / (width / 2)) * 2;
            uv[output_row + col * 2..output_row + col * 2 + 2]
                .copy_from_slice(&input_uv[source_row + source_col..source_row + source_col + 2]);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DEFAULT_MODE, LEGACY_MODE, SUPPORTED_MODES};

    #[test]
    fn packet_rejects_truncation_and_mismatched_mode_or_length() {
        for mode in SUPPORTED_MODES {
            let pixels = black_frame(mode);
            let mut packet = Vec::new();
            encode_packet(mode, 123, &pixels, &mut packet).unwrap();
            let (received_mode, timestamp, received) = decode_packet(&packet).unwrap();
            assert_eq!(
                (received_mode, timestamp, received.len()),
                (mode, 123, mode.nv12_size())
            );
            assert!(decode_packet(&packet[..packet.len() - 1]).is_err());
            packet[20] ^= 1;
            assert!(decode_packet(&packet).is_err());
            packet[20] ^= 1;
            packet[4] = 255;
            assert!(decode_packet(&packet).is_err());
        }
        assert!(decode_packet(&[0; 4]).is_err());
    }

    #[test]
    fn converts_aspect_ratio_with_neutral_borders_and_paired_chroma() {
        let mut input = black_frame(LEGACY_MODE);
        input[..640 * 480].fill(235);
        for pair in input[640 * 480..].as_chunks_mut::<2>().0 {
            pair.copy_from_slice(&[80, 170]);
        }
        let mut output = black_frame(DEFAULT_MODE);
        render_nv12(&input, LEGACY_MODE, DEFAULT_MODE, &mut output).unwrap();
        assert_eq!(output[0], 16);
        assert_eq!(output[160], 235);
        assert_eq!(output[1119], 235);
        assert_eq!(output[1120], 16);
        let uv = &output[1280 * 720..];
        assert_eq!(&uv[..2], &[128, 128]);
        assert_eq!(&uv[160..162], &[80, 170]);
        assert!(render_nv12(&input[..10], LEGACY_MODE, DEFAULT_MODE, &mut output).is_err());
    }
}
