use std::borrow::Cow;
use std::num::NonZero;

use ironrdp::rdpsnd::client::RdpsndClientHandler;
use ironrdp::rdpsnd::pdu::{AudioFormat, PitchPdu, VolumePdu, WaveFormat};
use rodio::buffer::SamplesBuffer;
use rodio::{DeviceSinkBuilder, MixerDeviceSink};
use tokio::sync::mpsc;

use super::types::{RdpEncodedAudioCodec, RdpEncodedAudioFrame};

const PCM_CHANNELS: u16 = 2;
const PCM_SAMPLE_RATE: u32 = 44_100;
const PCM_BITS_PER_SAMPLE: u16 = 16;
const PCM_BYTES_PER_SAMPLE: u32 = 2;
const WEBRTC_AUDIO_CHANNELS: u16 = 1;
const WEBRTC_AUDIO_SAMPLE_RATE: u32 = 8_000;
const WEBRTC_AUDIO_BITS_PER_SAMPLE: u16 = 8;
const WEBRTC_AUDIO_BYTES_PER_SAMPLE: u32 = 1;

static PCM_FORMATS: [AudioFormat; 1] = [AudioFormat {
    format: WaveFormat::PCM,
    n_channels: PCM_CHANNELS,
    n_samples_per_sec: PCM_SAMPLE_RATE,
    n_avg_bytes_per_sec: PCM_SAMPLE_RATE * PCM_CHANNELS as u32 * PCM_BYTES_PER_SAMPLE,
    n_block_align: PCM_CHANNELS * 2,
    bits_per_sample: PCM_BITS_PER_SAMPLE,
    data: None,
}];

static WEBRTC_AUDIO_FORMATS: [AudioFormat; 1] = [AudioFormat {
    format: WaveFormat::MULAW,
    n_channels: WEBRTC_AUDIO_CHANNELS,
    n_samples_per_sec: WEBRTC_AUDIO_SAMPLE_RATE,
    n_avg_bytes_per_sec: WEBRTC_AUDIO_SAMPLE_RATE
        * WEBRTC_AUDIO_CHANNELS as u32
        * WEBRTC_AUDIO_BYTES_PER_SAMPLE,
    n_block_align: WEBRTC_AUDIO_CHANNELS,
    bits_per_sample: WEBRTC_AUDIO_BITS_PER_SAMPLE,
    data: None,
}];

pub(super) fn webrtc_audio_formats() -> &'static [AudioFormat] {
    &WEBRTC_AUDIO_FORMATS
}

#[derive(Debug)]
pub(super) struct PcmAudioHandler {
    sink: Option<MixerDeviceSink>,
}

impl PcmAudioHandler {
    pub(super) fn new() -> Self {
        Self { sink: None }
    }

    fn ensure_sink(&mut self) -> Option<&MixerDeviceSink> {
        if self.sink.is_none() {
            self.sink = match DeviceSinkBuilder::open_default_sink() {
                Ok(mut sink) => {
                    sink.log_on_drop(false);
                    Some(sink)
                }
                Err(error) => {
                    eprintln!("[RDP] 打开默认音频输出失败，远程声音将被丢弃: {error}");
                    None
                }
            };
        }
        self.sink.as_ref()
    }
}

impl RdpsndClientHandler for PcmAudioHandler {
    fn get_formats(&self) -> &[AudioFormat] {
        &PCM_FORMATS
    }

    fn wave(&mut self, format_no: usize, _ts: u32, data: Cow<'_, [u8]>) {
        if format_no != 0 {
            return;
        }
        let samples = pcm_i16_le_to_f32(&data);
        if samples.is_empty() {
            return;
        }
        let Some(sink) = self.ensure_sink() else {
            return;
        };

        let Some(channels) = NonZero::new(PCM_CHANNELS) else {
            return;
        };
        let Some(sample_rate) = NonZero::new(PCM_SAMPLE_RATE) else {
            return;
        };
        sink.mixer()
            .add(SamplesBuffer::new(channels, sample_rate, samples));
    }

    fn set_volume(&mut self, _volume: VolumePdu) {}

    fn set_pitch(&mut self, _pitch: PitchPdu) {}

    fn close(&mut self) {}
}

#[derive(Debug)]
pub(super) struct WebRtcAudioHandler {
    session_id: String,
    audio_tx: mpsc::UnboundedSender<RdpEncodedAudioFrame>,
}

impl WebRtcAudioHandler {
    pub(super) fn new(session_id: String) -> (Self, mpsc::UnboundedReceiver<RdpEncodedAudioFrame>) {
        let (audio_tx, audio_rx) = mpsc::unbounded_channel();
        (
            Self {
                session_id,
                audio_tx,
            },
            audio_rx,
        )
    }
}

impl RdpsndClientHandler for WebRtcAudioHandler {
    fn get_formats(&self) -> &[AudioFormat] {
        webrtc_audio_formats()
    }

    fn wave(&mut self, _format_no: usize, ts: u32, data: Cow<'_, [u8]>) {
        if data.is_empty() {
            return;
        }

        let _ = self.audio_tx.send(RdpEncodedAudioFrame {
            session_id: self.session_id.clone(),
            codec: RdpEncodedAudioCodec::Pcmu,
            timestamp_ms: u64::from(ts),
            sample_rate: WEBRTC_AUDIO_SAMPLE_RATE,
            channels: WEBRTC_AUDIO_CHANNELS,
            bytes: data.into_owned(),
        });
    }

    fn set_volume(&mut self, _volume: VolumePdu) {}

    fn set_pitch(&mut self, _pitch: PitchPdu) {}

    fn close(&mut self) {}
}

fn pcm_i16_le_to_f32(data: &[u8]) -> Vec<f32> {
    data.chunks_exact(2)
        .map(|chunk| {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            f32::from(sample) / f32::from(i16::MAX)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;
    use crate::commands::rdp::types::RdpEncodedAudioCodec;

    #[test]
    fn webrtc_audio_formats_advertise_pcmu_for_browser_audio_track() {
        let formats = webrtc_audio_formats();

        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].format, WaveFormat::MULAW);
        assert_eq!(formats[0].n_channels, 1);
        assert_eq!(formats[0].n_samples_per_sec, 8_000);
        assert_eq!(formats[0].bits_per_sample, 8);
    }

    #[test]
    fn webrtc_audio_handler_forwards_pcmu_frames_with_rdpsnd_timestamp() {
        let (mut handler, mut audio_rx) = WebRtcAudioHandler::new("rdp-session".to_string());

        handler.wave(9, 1234, Cow::Borrowed(&[0x7f; 160]));
        let frame = audio_rx.try_recv().unwrap();

        assert_eq!(frame.session_id, "rdp-session");
        assert_eq!(frame.codec, RdpEncodedAudioCodec::Pcmu);
        assert_eq!(frame.timestamp_ms, 1234);
        assert_eq!(frame.sample_rate, 8_000);
        assert_eq!(frame.channels, 1);
        assert_eq!(frame.bytes, vec![0x7f; 160]);
    }
}
