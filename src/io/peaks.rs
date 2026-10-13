//! io::peaks: waveform min/max peak building at four block sizes.

use std::fs;
use std::path::Path;

use super::IoError;
use crate::model::media::{PeakLevel, Peaks};

/// File magic: b"PEAK".
const MAGIC: &[u8; 4] = b"PEAK";
/// Current peaks file format version.
const VERSION: u32 = 1;
/// Samples per block at level 0.
const BASE_BLOCK: u32 = 256;

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
    let l0 = PeakLevel {
        block_size: BASE_BLOCK,
        data: level0,
    };

    // Levels 1..3: each merges 4 blocks of the level below per channel.
    let l1 = merge_level(&l0, 1024, channels);
    let l2 = merge_level(&l1, 4096, channels);
    let l3 = merge_level(&l2, 16384, channels);

    Peaks {
        channels: channels as u16,
        frames,
        levels: vec![l0, l1, l2, l3],
    }
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

/// Reads a peaks file written by `write`. Errors on wrong magic, wrong version
/// or a truncated file.
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
        Ok(u16::from_le_bytes(take(2, pos)?.try_into().map_err(
            |_| IoError::CorruptPeaks("truncated file".to_string()),
        )?))
    };
    let u32_le = |pos: &mut usize| -> Result<u32, IoError> {
        Ok(u32::from_le_bytes(take(4, pos)?.try_into().map_err(
            |_| IoError::CorruptPeaks("truncated file".to_string()),
        )?))
    };
    let u64_le = |pos: &mut usize| -> Result<u64, IoError> {
        Ok(u64::from_le_bytes(take(8, pos)?.try_into().map_err(
            |_| IoError::CorruptPeaks("truncated file".to_string()),
        )?))
    };
    let f32_le = |pos: &mut usize| -> Result<f32, IoError> {
        Ok(f32::from_le_bytes(take(4, pos)?.try_into().map_err(
            |_| IoError::CorruptPeaks("truncated file".to_string()),
        )?))
    };

    if take(4, &mut pos)? != MAGIC {
        return Err(IoError::CorruptPeaks("bad magic".to_string()));
    }
    let version = u32_le(&mut pos)?;
    if version != VERSION {
        return Err(IoError::CorruptPeaks(format!(
            "unsupported version {version}"
        )));
    }
    let channels = u16_le(&mut pos)?;
    let frames = u64_le(&mut pos)?;
    let level_count = u32_le(&mut pos)?;
    let mut levels = Vec::new();
    for _ in 0..level_count {
        let block_size = u32_le(&mut pos)?;
        let blocks = u64_le(&mut pos)?;
        let mut data = Vec::with_capacity(blocks as usize * usize::from(channels));
        for _ in 0..(blocks as usize * usize::from(channels)) {
            let min = f32_le(&mut pos)?;
            let max = f32_le(&mut pos)?;
            data.push([min, max]);
        }
        levels.push(PeakLevel { block_size, data });
    }
    if pos != bytes.len() {
        return Err(IoError::CorruptPeaks("trailing bytes".to_string()));
    }
    Ok(Peaks {
        channels,
        frames,
        levels,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn ramp(len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| -1.0 + 2.0 * i as f32 / (len - 1) as f32)
            .collect()
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
}
