use std::borrow::Cow;
use std::num::NonZero;

use ironrdp::rdpsnd::client::RdpsndClientHandler;
use ironrdp::rdpsnd::pdu::{AudioFormat, PitchPdu, VolumePdu, WaveFormat};
use rodio::buffer::SamplesBuffer;
use rodio::{DeviceSinkBuilder, MixerDeviceSink};

const PCM_CHANNELS: u16 = 2;
const PCM_SAMPLE_RATE: u32 = 44_100;
const PCM_BITS_PER_SAMPLE: u16 = 16;
const PCM_BYTES_PER_SAMPLE: u32 = 2;

static PCM_FORMATS: [AudioFormat; 1] = [AudioFormat {
    format: WaveFormat::PCM,
    n_channels: PCM_CHANNELS,
    n_samples_per_sec: PCM_SAMPLE_RATE,
    n_avg_bytes_per_sec: PCM_SAMPLE_RATE * PCM_CHANNELS as u32 * PCM_BYTES_PER_SAMPLE,
    n_block_align: PCM_CHANNELS * 2,
    bits_per_sample: PCM_BITS_PER_SAMPLE,
    data: None,
}];

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

fn pcm_i16_le_to_f32(data: &[u8]) -> Vec<f32> {
    data.as_chunks::<2>()
        .0
        .iter()
        .map(|chunk| {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            f32::from(sample) / f32::from(i16::MAX)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_samples_are_normalised() {
        let data = [0x00, 0x00, 0xff, 0x7f, 0x01, 0x80];
        let samples = pcm_i16_le_to_f32(&data);
        assert_eq!(samples.len(), 3);
        assert_eq!(samples[0], 0.0);
        assert!((samples[1] - 1.0).abs() < 1e-6);
        assert!(samples[2] < -0.99);
        assert!(pcm_i16_le_to_f32(&[1]).is_empty());
    }

    #[test]
    fn the_client_only_offers_pcm() {
        let handler = PcmAudioHandler::new();
        let formats = handler.get_formats();
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].format, WaveFormat::PCM);
        assert_eq!(formats[0].n_samples_per_sec, 44_100);
    }
}
