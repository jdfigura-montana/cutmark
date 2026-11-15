//! io::peaks: waveform min/max peak building at four block sizes.

use std::fs;
use std::path::Path;

use super::IoError;
use super::limits::{MAX_PEAK_CHANNELS, MAX_PEAK_LEVELS};
use crate::model::media::{PeakLevel, Peaks};

/// File magic: b"PEAK".
const MAGIC: &[u8; 4] = b"PEAK";
/// Current peaks file format version.
const VERSION: u32 = 1;
/// Samples per block at level 0.
const BASE_BLOCK: u32 = 256;
/// Bytes of one [min, max] f32 pair in the file.
const PAIR_BYTES: u64 = 8;

/// Builds four peak levels (256, 1024, 4096, 16384) from interleaved samples.
/// Level 0 is computed from the samples; each higher level merges 4 blocks of
/// the level below. The last block may be partial. Empty input gives 4 empty levels.
pub fn build(samples: &[f32], channels: u16) -> Peaks {
    let channels = usize::from(channels);
    let frames = samples.len().checked_div(channels).unwrap_or(0) as u64;

    // Level 0: min/max per block per channel.
    let mut level0: Vec<[f32; 2]> = Vec::new();
    if channels > 0 {
        let block_count = frames.div_ceil(u64::from(BASE_BLOCK)) as usize;
        level0 = vec![[f32::INFINITY, f32::NEG_INFINITY]; block_count * channels];
        for (frame, chunk) in samples.chunks_exact(channels).enumerate() {
            let block = frame / BASE_BLOCK as usize;
            for (ch, &s) in chunk.iter().enumerate() {
                let e = &mut level0[block * channels + ch];
                e[0] = e[0].min(s);
                e[1] = e[1].max(s);
            }
        }
    }
    let l0 = PeakLevel { block_size: BASE_BLOCK, data: level0 };

    // Levels 1..3: each merges 4 blocks of the level below per channel.
    let l1 = merge_level(&l0, 1024, channels);
    let l2 = merge_level(&l1, 4096, channels);
    let l3 = merge_level(&l2, 16384, channels);

    Peaks { channels: channels as u16, frames, levels: vec![l0, l1, l2, l3] }
}

/// Merges 4 blocks of `lower` into one block per channel at the given block size.
/// Channel count comes from `channels` because empty levels carry no layout info.
fn merge_level(lower: &PeakLevel, block_size: u32, channels: usize) -> PeakLevel {
    let lower_blocks = lower.data.len().checked_div(channels).unwrap_or(0);
    let out_blocks = lower_blocks.div_ceil(4);
    let mut data = vec![[f32::INFINITY, f32::NEG_INFINITY]; out_blocks * channels];
    for out in 0..out_blocks {
        for child in (out * 4)..((out * 4 + 4).min(lower_blocks)) {
            for ch in 0..channels {
                let c = lower.data[child * channels + ch];
                let e = &mut data[out * channels + ch];
                e[0] = e[0].min(c[0]);
                e[1] = e[1].max(c[1]);
            }
        }
    }
    PeakLevel { block_size, data }
}

/// Writes a peaks file little-endian: magic b"PEAK", version u32, channels u16,
/// frames u64, level count u32, then per level: block_size u32, block count u64,
/// followed by the [min, max] f32 pairs. Writes a temp file, then renames.
pub fn write(path: &Path, peaks: &Peaks) -> Result<(), IoError> {
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&VERSION.to_le_bytes());
    buf.extend_from_slice(&peaks.channels.to_le_bytes());
    buf.extend_from_slice(&peaks.frames.to_le_bytes());
    buf.extend_from_slice(&(peaks.levels.len() as u32).to_le_bytes());
    let channels = usize::from(peaks.channels);
    for level in &peaks.levels {
        buf.extend_from_slice(&level.block_size.to_le_bytes());
        let blocks = level.data.len().checked_div(channels).unwrap_or(0) as u64;
        buf.extend_from_slice(&blocks.to_le_bytes());
        for pair in &level.data {
            buf.extend_from_slice(&pair[0].to_le_bytes());
            buf.extend_from_slice(&pair[1].to_le_bytes());
        }
    }
    let tmp = path.with_extension("peaks.tmp");
    fs::write(&tmp, &buf)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Reads a peaks file written by `write`. Errors on wrong magic, wrong version,
/// a truncated file, trailing bytes, a channel count outside 1..=2, a level
/// count outside 1..=8, a zero block size, or a block count that needs more
/// bytes than the file has left (checked before any allocation).
pub fn read(path: &Path) -> Result<Peaks, IoError> {
    let bytes = fs::read(path)?;
    let mut pos = 0usize;
    let take = |n: usize, pos: &mut usize| -> Result<&[u8], IoError> {
        if *pos + n > bytes.len() {
            return Err(IoError::CorruptPeaks("truncated file".to_string()));
        }
        let slice = &bytes[*pos..*pos + n];
        *pos += n;
        Ok(slice)
    };
    let u16_le = |pos: &mut usize| -> Result<u16, IoError> {
        Ok(u16::from_le_bytes(take(2, pos)?.try_into().map_err(|_| IoError::CorruptPeaks("truncated file".to_string()))?))
    };
    let u32_le = |pos: &mut usize| -> Result<u32, IoError> {
        Ok(u32::from_le_bytes(take(4, pos)?.try_into().map_err(|_| IoError::CorruptPeaks("truncated file".to_string()))?))
    };
    let u64_le = |pos: &mut usize| -> Result<u64, IoError> {
        Ok(u64::from_le_bytes(take(8, pos)?.try_into().map_err(|_| IoError::CorruptPeaks("truncated file".to_string()))?))
    };
    let f32_le = |pos: &mut usize| -> Result<f32, IoError> {
        Ok(f32::from_le_bytes(take(4, pos)?.try_into().map_err(|_| IoError::CorruptPeaks("truncated file".to_string()))?))
    };

    if take(4, &mut pos)? != MAGIC {
        return Err(IoError::CorruptPeaks("bad magic".to_string()));
    }
    let version = u32_le(&mut pos)?;
    if version != VERSION {
        return Err(IoError::CorruptPeaks(format!("unsupported version {version}")));
    }
    let channels = u16_le(&mut pos)?;
    if !(1..=MAX_PEAK_CHANNELS).contains(&channels) {
        return Err(IoError::CorruptPeaks(format!("unsupported channel count {channels}")));
    }
    let frames = u64_le(&mut pos)?;
    let level_count = u32_le(&mut pos)?;
    if !(1..=MAX_PEAK_LEVELS).contains(&level_count) {
        return Err(IoError::CorruptPeaks(format!("unsupported level count {level_count}")));
    }
    let mut levels = Vec::new();
    for _ in 0..level_count {
        let block_size = u32_le(&mut pos)?;
        if block_size == 0 {
            return Err(IoError::CorruptPeaks("block size is zero".to_string()));
        }
        let blocks = u64_le(&mut pos)?;
        // The block count is untrusted: it must fit in the bytes that are
        // actually left before anything is allocated or looped over.
        let bytes_needed = blocks
            .checked_mul(u64::from(channels))
            .and_then(|pairs| pairs.checked_mul(PAIR_BYTES))
            .ok_or_else(|| IoError::CorruptPeaks("block count overflows".to_string()))?;
        if bytes_needed > (bytes.len() - pos) as u64 {
            return Err(IoError::CorruptPeaks("truncated file".to_string()));
        }
        // Cannot overflow: bytes_needed is at most the file length.
        let pair_count = (bytes_needed / PAIR_BYTES) as usize;
        let mut data = Vec::with_capacity(pair_count);
        for _ in 0..pair_count {
            let min = f32_le(&mut pos)?;
            let max = f32_le(&mut pos)?;
            data.push([min, max]);
        }
        levels.push(PeakLevel { block_size, data });
    }
    if pos != bytes.len() {
        return Err(IoError::CorruptPeaks("trailing bytes".to_string()));
    }
    Ok(Peaks { channels, frames, levels })
}
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn ramp(len: usize) -> Vec<f32> {
        (0..len).map(|i| -1.0 + 2.0 * i as f32 / (len - 1) as f32).collect()
    }

    #[test]
    fn build_mono_ramp_levels() {
        let peaks = build(&ramp(1_000), 1);
        assert_eq!(peaks.channels, 1);
        assert_eq!(peaks.frames, 1_000);
        assert_eq!(peaks.levels.len(), 4);
        let l0 = &peaks.levels[0];
        assert_eq!(l0.block_size, 256);
        assert_eq!(l0.data.len(), 4);
        assert_eq!(l0.data[0][0], -1.0);
        assert_eq!(l0.data[3][1], 1.0);
        let l1 = &peaks.levels[1];
        assert_eq!(l1.block_size, 1024);
        assert_eq!(l1.data.len(), 1);
        assert_eq!(l1.data[0][0], -1.0);
        assert_eq!(l1.data[0][1], 1.0);
        // Coarser levels are also single blocks with the full range.
        for l in &peaks.levels[2..] {
            assert_eq!(l.data.len(), 1);
            assert_eq!(l.data[0][0], -1.0);
            assert_eq!(l.data[0][1], 1.0);
        }
    }

    #[test]
    fn build_stereo_constant_channels() {
        let samples: Vec<f32> = (0..512).flat_map(|_| [0.5f32, -0.5]).collect();
        let peaks = build(&samples, 2);
        assert_eq!(peaks.frames, 512);
        let l0 = &peaks.levels[0];
        assert_eq!(l0.data.len(), 4); // 2 blocks x 2 channels
        for pair in l0.data.as_chunks::<2>().0 {
            assert_eq!(pair[0], [0.5, 0.5]);
            assert_eq!(pair[1], [-0.5, -0.5]);
        }
    }

    #[test]
    fn build_empty_input_gives_empty_levels() {
        let peaks = build(&[], 2);
        assert_eq!(peaks.frames, 0);
        for level in &peaks.levels {
            assert!(level.data.is_empty());
        }
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.peaks");
        let peaks = build(&ramp(1_000), 2);
        write(&path, &peaks).unwrap();
        assert!(!dir.path().join("test.peaks.tmp").exists());
        let loaded = read(&path).unwrap();
        assert_eq!(loaded, peaks);
    }

    #[test]
    fn read_errors_on_bad_magic() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.peaks");
        let peaks = build(&ramp(300), 1);
        write(&path, &peaks).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        bytes[0] = b'X';
        fs::write(&path, &bytes).unwrap();
        assert!(matches!(read(&path), Err(IoError::CorruptPeaks(_))));
    }

    #[test]
    fn read_errors_on_truncated_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cut.peaks");
        let peaks = build(&ramp(1_000), 2);
        write(&path, &peaks).unwrap();
        let bytes = fs::read(&path).unwrap();
        fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
        assert!(matches!(read(&path), Err(IoError::CorruptPeaks(_))));
    }

    #[test]
    fn read_errors_on_bad_version() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ver.peaks");
        let peaks = build(&ramp(300), 1);
        write(&path, &peaks).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        // version u32 starts right after the 4-byte magic.
        bytes[4] = 9;
        fs::write(&path, &bytes).unwrap();
        assert!(matches!(read(&path), Err(IoError::CorruptPeaks(_))));
    }

    /// A peaks header (magic, version 1, `channels`, `frames`, `levels`).
    fn header(channels: u16, frames: u64, levels: u32) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&frames.to_le_bytes());
        bytes.extend_from_slice(&levels.to_le_bytes());
        bytes
    }

    /// The 34 byte file from the audit: one level that claims `blocks` blocks
    /// and carries none of them.
    fn lying_file(channels: u16, block_size: u32, blocks: u64) -> Vec<u8> {
        let mut bytes = header(channels, 0, 1);
        bytes.extend_from_slice(&block_size.to_le_bytes());
        bytes.extend_from_slice(&blocks.to_le_bytes());
        bytes
    }

    /// Reads `bytes` as a peaks file and returns the error text.
    fn read_err(bytes: &[u8]) -> String {
        let dir = tempdir().unwrap();
        let path = dir.path().join("crafted.peaks");
        fs::write(&path, bytes).unwrap();
        match read(&path) {
            Err(IoError::CorruptPeaks(reason)) => reason,
            other => panic!("expected CorruptPeaks, got {other:?}"),
        }
    }

    #[test]
    fn a_huge_block_count_is_rejected_before_any_allocation() {
        assert_eq!(lying_file(1, 256, 0).len(), 34);
        // 2^40 blocks used to ask for an 8 TB buffer (a process abort), 2^62
        // overflowed Vec::with_capacity (a panic). Both come back as errors,
        // and this test finishing at all shows nothing that size was allocated.
        for blocks in [1u64 << 40, 1 << 62, 1 << 61, u64::MAX, 1] {
            for channels in [1u16, 2] {
                let reason = read_err(&lying_file(channels, 256, blocks));
                assert!(reason == "truncated file" || reason == "block count overflows", "{blocks} blocks, {channels} channels: {reason}");
            }
        }
    }

    #[test]
    fn block_count_overflow_is_a_checked_multiply() {
        // blocks * channels * 8 wraps u64 for these, which would look small.
        assert_eq!(read_err(&lying_file(2, 256, 1 << 62)), "block count overflows");
        assert_eq!(read_err(&lying_file(1, 256, u64::MAX / 4)), "block count overflows");
    }

    #[test]
    fn one_block_more_than_the_file_holds_is_rejected_and_exact_fits() {
        let mut bytes = lying_file(1, 256, 2);
        bytes.extend_from_slice(&[0; 16]); // exactly two [min, max] pairs
        let dir = tempdir().unwrap();
        let path = dir.path().join("exact.peaks");
        fs::write(&path, &bytes).unwrap();
        let peaks = read(&path).unwrap();
        assert_eq!(peaks.levels[0].data.len(), 2);

        let mut short = lying_file(1, 256, 3);
        short.extend_from_slice(&[0; 16]);
        assert_eq!(read_err(&short), "truncated file");
    }

    #[test]
    fn a_zero_block_size_is_rejected() {
        assert_eq!(read_err(&lying_file(1, 0, 0)), "block size is zero");
    }

    #[test]
    fn channels_must_be_one_or_two() {
        for channels in [0u16, 3, 8, u16::MAX] {
            let reason = read_err(&lying_file(channels, 256, 0));
            assert_eq!(reason, format!("unsupported channel count {channels}"));
        }
    }

    #[test]
    fn level_count_must_be_one_to_eight() {
        for levels in [0u32, 9, 1_000, u32::MAX] {
            let reason = read_err(&header(1, 0, levels));
            assert_eq!(reason, format!("unsupported level count {levels}"));
        }
        // 8 levels is the most allowed (here all empty).
        let mut bytes = header(1, 0, 8);
        for _ in 0..8 {
            bytes.extend_from_slice(&256u32.to_le_bytes());
            bytes.extend_from_slice(&0u64.to_le_bytes());
        }
        let dir = tempdir().unwrap();
        let path = dir.path().join("eight.peaks");
        fs::write(&path, &bytes).unwrap();
        assert_eq!(read(&path).unwrap().levels.len(), 8);
    }

    #[test]
    fn peaks_of_empty_audio_still_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("empty.peaks");
        for channels in [1u16, 2] {
            let peaks = build(&[], channels);
            write(&path, &peaks).unwrap();
            assert_eq!(read(&path).unwrap(), peaks);
        }
    }
}
