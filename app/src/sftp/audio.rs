//! Audio preview playback (the original used an HTML `<audio controls>`): decode with rodio and play on the default
//! output device, with pause / seek / position.

use std::io::Cursor;
use std::time::Duration;

use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};

#[derive(Debug, Clone, PartialEq)]
pub struct AudioInfo {
    pub duration: Option<Duration>,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Decode the header only: does rodio understand this file, and how long is it?
pub fn probe(bytes: &[u8]) -> Result<AudioInfo, String> {
    let decoder = Decoder::new(Cursor::new(bytes.to_vec())).map_err(|e| e.to_string())?;
    Ok(AudioInfo { duration: decoder.total_duration(), sample_rate: u32::from(decoder.sample_rate().get()), channels: decoder.channels().get() })
}

pub struct AudioPlayback {
    // the sink must outlive the player
    _sink: MixerDeviceSink,
    player: Player,
    info: AudioInfo,
}

impl AudioPlayback {
    /// Open the default output device and start playing `bytes` from the beginning.
    pub fn start(bytes: Vec<u8>) -> Result<Self, String> {
        let info = probe(&bytes)?;
        let decoder = Decoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
        let mut sink = DeviceSinkBuilder::open_default_sink().map_err(|e| e.to_string())?;
        sink.log_on_drop(false);
        let player = Player::connect_new(sink.mixer());
        player.append(decoder);
        Ok(Self { _sink: sink, player, info })
    }

    pub fn info(&self) -> &AudioInfo {
        &self.info
    }

    pub fn play(&self) {
        self.player.play();
    }

    pub fn pause(&self) {
        self.player.pause();
    }

    pub fn is_paused(&self) -> bool {
        self.player.is_paused()
    }

    pub fn finished(&self) -> bool {
        self.player.empty()
    }

    pub fn position(&self) -> Duration {
        self.player.get_pos()
    }

    pub fn seek(&self, to: Duration) -> Result<(), String> {
        self.player.try_seek(to).map_err(|e| e.to_string())
    }
}

pub fn format_clock(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::{format_clock, probe};
    use std::time::Duration;

    /// A 0.5 s, 8 kHz, mono, 16-bit PCM WAV of a sine tone.
    pub fn wav_bytes() -> Vec<u8> {
        let (rate, secs) = (8000u32, 0.5f32);
        let n = (rate as f32 * secs) as usize;
        let mut data = Vec::new();
        for i in 0..n {
            let v = ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 8000.0) as i16;
            data.extend_from_slice(&v.to_le_bytes());
        }
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes()); // PCM
        w.extend_from_slice(&1u16.to_le_bytes()); // mono
        w.extend_from_slice(&rate.to_le_bytes());
        w.extend_from_slice(&(rate * 2).to_le_bytes());
        w.extend_from_slice(&2u16.to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&(data.len() as u32).to_le_bytes());
        w.extend_from_slice(&data);
        w
    }

    #[test]
    fn probe_reads_format_and_length() {
        let info = probe(&wav_bytes()).expect("wav decodes");
        assert_eq!((info.sample_rate, info.channels), (8000, 1));
        let d = info.duration.expect("wav knows its length");
        assert!((d.as_secs_f32() - 0.5).abs() < 0.01, "{d:?}");
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(probe(b"definitely not audio").is_err());
        assert!(probe(&[]).is_err());
    }

    #[test]
    fn clock_is_minutes_and_seconds() {
        assert_eq!(format_clock(Duration::from_secs(5)), "0:05");
        assert_eq!(format_clock(Duration::from_secs(754)), "12:34");
    }
}
