use std::collections::{BTreeMap, HashMap};

use ironrdp::core::{impl_as_any, EncodeResult, ReadCursor, WriteCursor};
use ironrdp::dvc::{DvcChannelListener, DvcMessage, DvcProcessor};
use ironrdp::pdu::PduResult;
use tokio::sync::mpsc;

use super::types::{RdpEncodedVideoCodec, RdpEncodedVideoFrame};

pub(super) const VIDEO_CONTROL_CHANNEL: &str = "Microsoft::Windows::RDS::Video::Control::v08.01";
pub(super) const VIDEO_DATA_CHANNEL: &str = "Microsoft::Windows::RDS::Video::Data::v08.01";

const PACKET_TYPE_PRESENTATION_REQUEST: u32 = 1;
const PACKET_TYPE_PRESENTATION_RESPONSE: u32 = 2;
const PACKET_TYPE_VIDEO_DATA: u32 = 4;
const PRESENTATION_COMMAND_START: u8 = 1;
const PRESENTATION_COMMAND_STOP: u8 = 2;
const VIDEO_DATA_FLAG_HAS_TIMESTAMPS: u8 = 0x01;
const VIDEO_DATA_FLAG_KEYFRAME: u8 = 0x02;
const HNS_PER_MS: u64 = 10_000;

// MFVideoFormat_H264 GUID bytes as carried on the wire in RDPEVOR.
const MF_VIDEO_FORMAT_H264: [u8; 16] = [
    0x48, 0x32, 0x36, 0x34, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
];

pub(super) fn video_control_channel(session_id: String) -> RdpevorControlListener {
    RdpevorControlListener { session_id }
}

pub(super) fn video_data_channel(
    session_id: String,
) -> (
    RdpevorDataListener,
    mpsc::UnboundedReceiver<RdpEncodedVideoFrame>,
) {
    let (encoded_tx, encoded_rx) = mpsc::unbounded_channel();
    (
        RdpevorDataListener {
            session_id,
            encoded_tx,
        },
        encoded_rx,
    )
}

pub(super) struct RdpevorControlListener {
    session_id: String,
}

impl_as_any!(RdpevorControlListener);

impl DvcChannelListener for RdpevorControlListener {
    fn channel_name(&self) -> &str {
        VIDEO_CONTROL_CHANNEL
    }

    fn create(&mut self) -> Option<Box<dyn DvcProcessor>> {
        Some(Box::new(RdpevorControlChannel::new(
            self.session_id.clone(),
        )))
    }
}

pub(super) struct RdpevorDataListener {
    session_id: String,
    encoded_tx: mpsc::UnboundedSender<RdpEncodedVideoFrame>,
}

impl_as_any!(RdpevorDataListener);

impl DvcChannelListener for RdpevorDataListener {
    fn channel_name(&self) -> &str {
        VIDEO_DATA_CHANNEL
    }

    fn create(&mut self) -> Option<Box<dyn DvcProcessor>> {
        Some(Box::new(RdpevorDataChannel::with_sender(
            self.session_id.clone(),
            self.encoded_tx.clone(),
        )))
    }
}

pub(super) struct RdpevorControlChannel {
    session_id: String,
    presentations: HashMap<u8, PresentationInfo>,
}

impl_as_any!(RdpevorControlChannel);

impl RdpevorControlChannel {
    pub(super) fn new(session_id: String) -> Self {
        Self {
            session_id,
            presentations: HashMap::new(),
        }
    }
}

impl DvcProcessor for RdpevorControlChannel {
    fn channel_name(&self) -> &str {
        VIDEO_CONTROL_CHANNEL
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let request = match parse_presentation_request(payload) {
            Ok(request) => request,
            Err(error) => {
                eprintln!(
                    "[RDP][backend][{}] rdpevor_control_parse_error error={} bytes={}",
                    self.session_id,
                    error,
                    payload.len(),
                );
                return Ok(Vec::new());
            }
        };

        if request.command == PRESENTATION_COMMAND_STOP {
            self.presentations.remove(&request.presentation_id);
            return Ok(Vec::new());
        }
        if request.command != PRESENTATION_COMMAND_START || !request.is_h264 {
            return Ok(Vec::new());
        }

        self.presentations.insert(
            request.presentation_id,
            PresentationInfo {
                _version: request.version,
                _sequence_header: request.extra_data,
            },
        );
        Ok(vec![Box::new(PresentationResponse {
            presentation_id: request.presentation_id,
        })])
    }

    fn close(&mut self, _channel_id: u32) {}
}

pub(super) struct RdpevorDataChannel {
    session_id: String,
    encoded_tx: mpsc::UnboundedSender<RdpEncodedVideoFrame>,
    partial_samples: HashMap<(u8, u32), PartialVideoSample>,
}

impl_as_any!(RdpevorDataChannel);

impl RdpevorDataChannel {
    #[cfg(test)]
    pub(super) fn new(session_id: String) -> (Self, mpsc::UnboundedReceiver<RdpEncodedVideoFrame>) {
        let (encoded_tx, encoded_rx) = mpsc::unbounded_channel();
        (Self::with_sender(session_id, encoded_tx), encoded_rx)
    }

    fn with_sender(
        session_id: String,
        encoded_tx: mpsc::UnboundedSender<RdpEncodedVideoFrame>,
    ) -> Self {
        Self {
            session_id,
            encoded_tx,
            partial_samples: HashMap::new(),
        }
    }

    fn emit_sample(&mut self, sample: VideoDataPacket, bytes: Vec<u8>) {
        let _ = self.encoded_tx.send(RdpEncodedVideoFrame {
            session_id: self.session_id.clone(),
            codec: RdpEncodedVideoCodec::H264AnnexB,
            frame_id: sample.sample_number,
            timestamp_ms: sample.timestamp_ms,
            is_keyframe: sample.is_keyframe(),
            bytes,
        });
    }
}

impl DvcProcessor for RdpevorDataChannel {
    fn channel_name(&self) -> &str {
        VIDEO_DATA_CHANNEL
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let packet = match parse_video_data(payload) {
            Ok(packet) => packet,
            Err(error) => {
                eprintln!(
                    "[RDP][backend][{}] rdpevor_data_parse_error error={} bytes={}",
                    self.session_id,
                    error,
                    payload.len(),
                );
                return Ok(Vec::new());
            }
        };

        if packet.packets_in_sample <= 1 {
            self.emit_sample(packet.clone(), packet.sample);
            return Ok(Vec::new());
        }

        let key = (packet.presentation_id, packet.sample_number);
        let mut completed = None;
        {
            let entry = self
                .partial_samples
                .entry(key)
                .or_insert_with(|| PartialVideoSample::new(&packet));
            entry.insert_packet(&packet);
            if entry.is_complete() {
                completed = self.partial_samples.remove(&key).map(|sample| {
                    (
                        VideoDataPacket {
                            sample: Vec::new(),
                            ..packet.clone()
                        },
                        sample.join_packets(),
                    )
                });
            }
        }

        if let Some((sample, bytes)) = completed {
            self.emit_sample(sample, bytes);
        }
        Ok(Vec::new())
    }

    fn close(&mut self, _channel_id: u32) {}
}

#[derive(Clone)]
struct VideoDataPacket {
    presentation_id: u8,
    flags: u8,
    timestamp_ms: u64,
    sample_number: u32,
    current_packet_index: u16,
    packets_in_sample: u16,
    sample: Vec<u8>,
}

impl VideoDataPacket {
    fn is_keyframe(&self) -> bool {
        self.flags & VIDEO_DATA_FLAG_KEYFRAME != 0
    }
}

struct PartialVideoSample {
    packets_in_sample: u16,
    packets: BTreeMap<u16, Vec<u8>>,
}

impl PartialVideoSample {
    fn new(packet: &VideoDataPacket) -> Self {
        Self {
            packets_in_sample: packet.packets_in_sample,
            packets: BTreeMap::new(),
        }
    }

    fn insert_packet(&mut self, packet: &VideoDataPacket) {
        self.packets
            .insert(packet.current_packet_index, packet.sample.clone());
    }

    fn is_complete(&self) -> bool {
        self.packets.len() == usize::from(self.packets_in_sample)
    }

    fn join_packets(self) -> Vec<u8> {
        self.packets.into_values().flatten().collect::<Vec<u8>>()
    }
}

#[derive(Clone)]
struct PresentationInfo {
    _version: u8,
    _sequence_header: Vec<u8>,
}

struct PresentationRequest {
    presentation_id: u8,
    version: u8,
    command: u8,
    is_h264: bool,
    extra_data: Vec<u8>,
}

struct PresentationResponse {
    presentation_id: u8,
}

impl ironrdp::core::Encode for PresentationResponse {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        dst.write_u32(12);
        dst.write_u32(PACKET_TYPE_PRESENTATION_RESPONSE);
        dst.write_u8(self.presentation_id);
        dst.write_u8(0);
        dst.write_u16(0);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "TSMM_PRESENTATION_RESPONSE"
    }

    fn size(&self) -> usize {
        12
    }
}

impl ironrdp::dvc::DvcEncode for PresentationResponse {}

fn parse_presentation_request(payload: &[u8]) -> Result<PresentationRequest, String> {
    let mut cursor = ReadCursor::new(payload);
    let cb_size = cursor.read_u32();
    let packet_type = cursor.read_u32();
    if cb_size as usize != payload.len() {
        return Err(format!(
            "invalid cbSize: declared={} actual={}",
            cb_size,
            payload.len()
        ));
    }
    if packet_type != PACKET_TYPE_PRESENTATION_REQUEST {
        return Err(format!("unexpected packet_type={packet_type}"));
    }

    let presentation_id = cursor.read_u8();
    let version = cursor.read_u8();
    let command = cursor.read_u8();
    let _frame_rate = cursor.read_u8();
    if command == PRESENTATION_COMMAND_STOP {
        return Ok(PresentationRequest {
            presentation_id,
            version,
            command,
            is_h264: false,
            extra_data: Vec::new(),
        });
    }

    let _average_bitrate_kbps = cursor.read_u16();
    let _reserved = cursor.read_u16();
    let _source_width = cursor.read_u32();
    let _source_height = cursor.read_u32();
    let _scaled_width = cursor.read_u32();
    let _scaled_height = cursor.read_u32();
    let _hns_timestamp_offset = cursor.read_u64();
    let _geometry_mapping_id = cursor.read_u64();
    let mut subtype = [0_u8; 16];
    subtype.copy_from_slice(cursor.read_slice(16));
    let cb_extra = cursor.read_u32() as usize;
    if cursor.len() < cb_extra {
        return Err(format!(
            "presentation extra data truncated: declared={} remaining={}",
            cb_extra,
            cursor.len(),
        ));
    }
    let extra_data = cursor.read_slice(cb_extra).to_vec();

    Ok(PresentationRequest {
        presentation_id,
        version,
        command,
        is_h264: subtype == MF_VIDEO_FORMAT_H264,
        extra_data,
    })
}

fn parse_video_data(payload: &[u8]) -> Result<VideoDataPacket, String> {
    let mut cursor = ReadCursor::new(payload);
    let cb_size = cursor.read_u32();
    let packet_type = cursor.read_u32();
    if cb_size as usize != payload.len() {
        return Err(format!(
            "invalid cbSize: declared={} actual={}",
            cb_size,
            payload.len()
        ));
    }
    if packet_type != PACKET_TYPE_VIDEO_DATA {
        return Err(format!("unexpected packet_type={packet_type}"));
    }

    let presentation_id = cursor.read_u8();
    let _version = cursor.read_u8();
    let flags = cursor.read_u8();
    let _reserved = cursor.read_u8();
    let timestamp_ms = if flags & VIDEO_DATA_FLAG_HAS_TIMESTAMPS != 0 {
        cursor.read_u64() / HNS_PER_MS
    } else {
        cursor.read_u64();
        0
    };
    let _duration_hns = cursor.read_u64();
    let current_packet_index = cursor.read_u16();
    let packets_in_sample = cursor.read_u16();
    let sample_number = cursor.read_u32();
    let cb_sample = cursor.read_u32() as usize;
    if cursor.len() < cb_sample {
        return Err(format!(
            "video sample truncated: declared={} remaining={}",
            cb_sample,
            cursor.len(),
        ));
    }
    let sample = cursor.read_slice(cb_sample).to_vec();

    Ok(VideoDataPacket {
        presentation_id,
        flags,
        timestamp_ms,
        sample_number,
        current_packet_index,
        packets_in_sample,
        sample,
    })
}

#[cfg(test)]
mod tests {
    use ironrdp::core::encode_vec;
    use ironrdp::dvc::DvcProcessor as _;

    use super::*;
    use crate::commands::rdp::types::RdpEncodedVideoCodec;

    fn h264_presentation_request() -> Vec<u8> {
        [
            0x69, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x03, 0x01, 0x01, 0x1d, 0xc0, 0x12,
            0x00, 0x00, 0xe0, 0x01, 0x00, 0x00, 0xf4, 0x00, 0x00, 0x00, 0xe0, 0x01, 0x00, 0x00,
            0xf4, 0x00, 0x00, 0x00, 0xa4, 0x7a, 0x3b, 0x82, 0x0f, 0x00, 0x00, 0x00, 0x22, 0x02,
            0x04, 0x00, 0xba, 0x7a, 0x00, 0x80, 0x48, 0x32, 0x36, 0x34, 0x00, 0x00, 0x10, 0x00,
            0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71, 0x25, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x01, 0x67, 0x42, 0xc0, 0x15, 0x95, 0xa0, 0x78, 0x21, 0xf9, 0xe1, 0x00, 0x00,
            0x03, 0x00, 0x01, 0x00, 0x00, 0x03, 0x00, 0x3c, 0x0d, 0xa0, 0x88, 0x46, 0xa0, 0x00,
            0x00, 0x00, 0x01, 0x68, 0xce, 0x3c, 0x80,
        ]
        .to_vec()
    }

    fn h264_video_data(sample: &[u8]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&(40_u32.saturating_add(sample.len() as u32)).to_le_bytes());
        payload.extend_from_slice(&4_u32.to_le_bytes());
        payload.push(3);
        payload.push(1);
        payload.push(0x03);
        payload.push(0);
        payload.extend_from_slice(&333_000_u64.to_le_bytes());
        payload.extend_from_slice(&333_000_u64.to_le_bytes());
        payload.extend_from_slice(&1_u16.to_le_bytes());
        payload.extend_from_slice(&1_u16.to_le_bytes());
        payload.extend_from_slice(&1_u32.to_le_bytes());
        payload.extend_from_slice(&(sample.len() as u32).to_le_bytes());
        payload.extend_from_slice(sample);
        payload
    }

    #[test]
    fn rdpevor_control_replies_ready_to_h264_presentation_start() {
        let mut control = RdpevorControlChannel::new("rdp-rdpevor-test".to_string());

        let responses = control.process(8, &h264_presentation_request()).unwrap();

        assert_eq!(responses.len(), 1);
        assert_eq!(
            encode_vec(responses[0].as_ref()).unwrap(),
            vec![12, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0]
        );
    }

    #[test]
    fn rdpevor_data_forwards_complete_h264_sample() {
        let (mut data, mut rx) = RdpevorDataChannel::new("rdp-rdpevor-test".to_string());
        let sample = b"\x00\x00\x00\x01\x67\x42\xc0\x15\x00\x00\x00\x01\x65\x88\x80";

        let responses = data.process(9, &h264_video_data(sample)).unwrap();

        assert!(responses.is_empty());
        let frame = rx.try_recv().unwrap();
        assert_eq!(frame.session_id, "rdp-rdpevor-test");
        assert_eq!(frame.codec, RdpEncodedVideoCodec::H264AnnexB);
        assert_eq!(frame.frame_id, 1);
        assert_eq!(frame.timestamp_ms, 33);
        assert!(frame.is_keyframe);
        assert_eq!(frame.bytes, sample);
    }
}
