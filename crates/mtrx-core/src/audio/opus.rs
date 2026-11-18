//! Ogg Opus decoded to WAV (RFC 7845), for players that can't play it:
//! libopus decodes, `ogg` reads the pages.

use std::{
    fs::File,
    io::{BufReader, BufWriter, Cursor, Seek, SeekFrom, Write},
    path::Path,
    sync::Arc,
};

use ::opus::{Channels, Decoder};
use ogg::PacketReader;
use tokio::io::AsyncReadExt;

use crate::fetch::{BUFFER, DAMAGED, part_path, write_failed};

/// Opus decodes at 48 kHz.
const RATE: u32 = 48_000;
/// Samples per channel in the longest packet: 120 ms.
const MAX_FRAMES: usize = 5760;
/// Bytes read to tell Ogg Opus: its first page holds the head alone.
const START: u64 = 1024;
/// The size of a WAV header.
const HEADER: u64 = 44;

pub(crate) const CANT_PLAY: &str = "This audio can't be played";

/// Whether the file at `path` is Ogg Opus: its first packet is an
/// `OpusHead`.
pub(crate) async fn is_ogg_opus(path: &Path) -> bool {
    let Ok(file) = tokio::fs::File::open(path).await else {
        return false;
    };
    let mut start = Vec::new();
    if file.take(START).read_to_end(&mut start).await.is_err() {
        return false;
    }
    starts_ogg_opus(&start)
}

/// Whether a file's first bytes are an Ogg page with an `OpusHead`.
fn starts_ogg_opus(start: &[u8]) -> bool {
    matches!(
        PacketReader::new(Cursor::new(start)).read_packet(),
        Ok(Some(packet)) if packet.data.starts_with(b"OpusHead")
    )
}

/// Decode the Ogg Opus file at `from` to a WAV at `to`, in a blocking
/// thread. It stops if this future is dropped; its partial file goes.
pub(crate) async fn to_wav(from: &Path, to: &Path) -> Result<(), String> {
    let running = Arc::new(());
    let still = Arc::downgrade(&running);
    let (from, to) = (from.to_owned(), to.to_owned());
    tokio::task::spawn_blocking(move || {
        let part = part_path(&to);
        let decoded = decode(&from, &part, || still.strong_count() > 0)
            .and_then(|()| std::fs::rename(&part, &to).map_err(write_failed));
        if decoded.is_err() {
            let _ = std::fs::remove_file(&part);
        }
        decoded
    })
    .await
    .map_err(|e| e.to_string())?
}

/// What an `OpusHead` says.
struct Head {
    channels: Channels,
    /// Samples (per channel) to drop at the start.
    pre_skip: u64,
    /// In Q7.8 dB.
    gain: i16,
}

impl Head {
    /// Mono or stereo (mapping family 0) only.
    fn parse(packet: &[u8]) -> Result<Self, String> {
        if packet.len() < 19 || !packet.starts_with(b"OpusHead") {
            return Err(DAMAGED.into());
        }
        // Versions 0 to 15 read the same.
        if packet[8] >> 4 != 0 {
            return Err(CANT_PLAY.into());
        }
        let channels = match (packet[18], packet[9]) {
            (0, 1) => Channels::Mono,
            (0, 2) => Channels::Stereo,
            _ => return Err(CANT_PLAY.into()),
        };
        Ok(Self {
            channels,
            pre_skip: u16::from_le_bytes([packet[10], packet[11]]).into(),
            gain: i16::from_le_bytes([packet[16], packet[17]]),
        })
    }
}

/// Decode `from` to a WAV at `to` (16-bit, 48 kHz), while `go_on`.
fn decode(from: &Path, to: &Path, go_on: impl Fn() -> bool) -> Result<(), String> {
    let input = File::open(from).map_err(|e| format!("Couldn't read the file: {e}"))?;
    let mut packets = PacketReader::new(BufReader::with_capacity(BUFFER, input));
    let mut next = || packets.read_packet().map_err(|_| DAMAGED.to_owned());
    let first = next()?.ok_or(DAMAGED)?;
    let (head, serial) = (Head::parse(&first.data)?, first.stream_serial());
    let mut decoder = Decoder::new(RATE, head.channels).map_err(|_| CANT_PLAY)?;
    decoder.set_gain(head.gain.into()).map_err(|_| CANT_PLAY)?;
    let channels = head.channels as usize;

    let mut output = BufWriter::with_capacity(BUFFER, File::create(to).map_err(write_failed)?);
    output
        .write_all(&[0; HEADER as usize])
        .map_err(write_failed)?;
    let mut samples = vec![0i16; MAX_FRAMES * channels];
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    let (mut skip, mut written, mut end, mut tags) = (head.pre_skip, 0u64, None, false);
    while let Some(packet) = next()? {
        if !go_on() {
            return Err("Cancelled".into());
        }
        if packet.stream_serial() != serial || packet.data.is_empty() {
            continue;
        }
        // The first packet after the head: `OpusTags`.
        if !tags {
            tags = true;
            continue;
        }
        if packet.last_in_stream() {
            end = Some(packet.absgp_page());
        }
        let frames = (decoder.decode(&packet.data, &mut samples, false)).map_err(|_| DAMAGED)?;
        let dropped = skip.min(frames as u64) as usize;
        skip -= dropped as u64;
        bytes.clear();
        for sample in &samples[dropped * channels..frames * channels] {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        output.write_all(&bytes).map_err(write_failed)?;
        written += (frames - dropped) as u64;
    }
    // The last page's granule position cuts the end (RFC 7845, 4).
    let frames = match end {
        Some(end) => written.min(end.saturating_sub(head.pre_skip)),
        None => written,
    };
    let data = frames * channels as u64 * 2;
    output.flush().map_err(write_failed)?;
    let mut file = (output.into_inner()).map_err(|e| write_failed(e.into_error()))?;
    file.set_len(HEADER + data).map_err(write_failed)?;
    file.seek(SeekFrom::Start(0)).map_err(write_failed)?;
    file.write_all(&wav_header(channels as u16, data))
        .map_err(write_failed)
}

/// The header of a 16-bit WAV at 48 kHz with `data` bytes of samples.
fn wav_header(channels: u16, data: u64) -> Vec<u8> {
    let data = u32::try_from(data).unwrap_or(u32::MAX);
    let block = channels * 2;
    let parts: [&[u8]; 13] = [
        b"RIFF",
        &data.saturating_add(36).to_le_bytes(),
        b"WAVE",
        b"fmt ",
        &16u32.to_le_bytes(),
        // PCM.
        &1u16.to_le_bytes(),
        &channels.to_le_bytes(),
        &RATE.to_le_bytes(),
        &(RATE * u32::from(block)).to_le_bytes(),
        &block.to_le_bytes(),
        &16u16.to_le_bytes(),
        b"data",
        &data.to_le_bytes(),
    ];
    parts.concat()
}

/// One channel of Ogg Opus: a `hz` sine for `seconds`, as libopus encodes
/// it, its head saying `mapping_family`. For tests.
#[cfg(any(test, feature = "test-support"))]
pub fn ogg_opus_sine(hz: f32, seconds: u32, mapping_family: u8) -> Vec<u8> {
    use ::opus::{Application, Encoder};
    use ogg::{PacketWriteEndInfo, PacketWriter};

    /// 20 ms.
    const FRAME: usize = 960;
    let mut encoder = Encoder::new(RATE, Channels::Mono, Application::Audio).expect("an encoder");
    let pre_skip = encoder.get_lookahead().expect("its delay") as u64;
    let frames = u64::from(seconds) * u64::from(RATE);

    let mut head = b"OpusHead".to_vec();
    head.extend_from_slice(&[1, 1]);
    head.extend_from_slice(&(pre_skip as u16).to_le_bytes());
    head.extend_from_slice(&RATE.to_le_bytes());
    head.extend_from_slice(&0i16.to_le_bytes());
    head.push(mapping_family);
    if mapping_family != 0 {
        // One stream, uncoupled.
        head.extend_from_slice(&[1, 0, 0]);
    }
    let mut tags = b"OpusTags".to_vec();
    tags.extend_from_slice(&4u32.to_le_bytes());
    tags.extend_from_slice(b"mtrx");
    tags.extend_from_slice(&0u32.to_le_bytes());

    let serial = 7;
    let mut writer = PacketWriter::new(Vec::new());
    for packet in [head, tags] {
        (writer.write_packet(packet, serial, PacketWriteEndInfo::EndPage, 0)).expect("written");
    }
    // The sine, then silence for the encoder's delay.
    let samples: Vec<i16> = (0..frames + pre_skip)
        .map(|i| {
            if i < frames {
                let t = i as f32 / RATE as f32;
                ((std::f32::consts::TAU * hz * t).sin() * 10_000.0) as i16
            } else {
                0
            }
        })
        .collect();
    let packets = samples.len().div_ceil(FRAME);
    for (n, chunk) in samples.chunks(FRAME).enumerate() {
        let mut frame = chunk.to_vec();
        frame.resize(FRAME, 0);
        let packet = encoder.encode_vec(&frame, 4000).expect("encoded");
        let (end, granule) = if n + 1 == packets {
            (PacketWriteEndInfo::EndStream, pre_skip + frames)
        } else {
            (
                PacketWriteEndInfo::NormalPacket,
                (n as u64 + 1) * FRAME as u64,
            )
        };
        (writer.write_packet(packet, serial, end, granule)).expect("written");
    }
    writer.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The samples of a 16-bit WAV, checking its header.
    fn wav_samples(path: &Path, channels: u16) -> Vec<i16> {
        let wav = std::fs::read(path).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), channels);
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), RATE);
        let data = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(data + 44, wav.len());
        assert_eq!(
            u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize,
            wav.len() - 8
        );
        let (samples, rest) = wav[44..].as_chunks::<2>();
        assert!(rest.is_empty());
        samples.iter().map(|b| i16::from_le_bytes(*b)).collect()
    }

    /// The power of `samples` at `hz` (Goertzel).
    fn power(samples: &[i16], hz: f64) -> f64 {
        let coefficient = 2.0 * (std::f64::consts::TAU * hz / RATE as f64).cos();
        let (mut s1, mut s2) = (0.0, 0.0);
        for &x in samples {
            let s = f64::from(x) + coefficient * s1 - s2;
            (s2, s1) = (s1, s);
        }
        s1 * s1 + s2 * s2 - coefficient * s1 * s2
    }

    fn write(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn a_sine_decodes_to_exactly_its_samples() {
        let dir = tempfile::tempdir().unwrap();
        let from = write(&dir, "sine.ogg", &ogg_opus_sine(440.0, 1, 0));
        let to = dir.path().join("sine.wav");
        decode(&from, &to, || true).unwrap();
        let samples = wav_samples(&to, 1);
        // The encoder's delay dropped, the end cut.
        assert_eq!(samples.len(), 48_000);
        let strongest = (100..2000)
            .step_by(10)
            .max_by(|&a, &b| power(&samples, a as f64).total_cmp(&power(&samples, b as f64)))
            .unwrap();
        assert_eq!(strongest, 440);
    }

    #[tokio::test]
    async fn only_ogg_opus_is_converted() {
        let dir = tempfile::tempdir().unwrap();
        let opus = write(&dir, "a.ogg", &ogg_opus_sine(440.0, 1, 0));
        assert!(is_ogg_opus(&opus).await);

        let mut vorbis = ogg::PacketWriter::new(Vec::new());
        let head = b"\x01vorbis\0\0\0\0\x01\x44\xac\0\0".to_vec();
        (vorbis.write_packet(head, 1, ogg::PacketWriteEndInfo::EndStream, 0)).unwrap();
        let vorbis = write(&dir, "b.ogg", &vorbis.into_inner());
        assert!(!is_ogg_opus(&vorbis).await);

        let mp3 = write(&dir, "c.mp3", b"ID3\x04\0\0\0\0\0\0\xff\xfb\x90\x64");
        assert!(!is_ogg_opus(&mp3).await);
        assert!(!is_ogg_opus(&dir.path().join("missing.ogg")).await);

        let to = dir.path().join("a.wav");
        to_wav(&opus, &to).await.unwrap();
        assert!(to.exists());
        assert!(!part_path(&to).exists());
    }

    #[test]
    fn a_cut_off_file_is_damaged() {
        let dir = tempfile::tempdir().unwrap();
        let sine = ogg_opus_sine(440.0, 1, 0);
        let from = write(&dir, "cut.ogg", &sine[..sine.len() * 2 / 3]);
        let to = dir.path().join("cut.wav");
        assert_eq!(decode(&from, &to, || true), Err(DAMAGED.to_owned()));
    }

    #[tokio::test]
    async fn only_mono_and_stereo_play() {
        let dir = tempfile::tempdir().unwrap();
        let from = write(&dir, "surround.ogg", &ogg_opus_sine(440.0, 1, 1));
        let to = dir.path().join("surround.wav");
        assert_eq!(to_wav(&from, &to).await, Err(CANT_PLAY.to_owned()));
        assert!(!to.exists());
        assert!(!part_path(&to).exists());
    }

    #[test]
    fn a_cancelled_conversion_stops() {
        let dir = tempfile::tempdir().unwrap();
        let from = write(&dir, "sine.ogg", &ogg_opus_sine(440.0, 1, 0));
        let to = dir.path().join("sine.wav");
        assert_eq!(decode(&from, &to, || false), Err("Cancelled".to_owned()));
    }
}
