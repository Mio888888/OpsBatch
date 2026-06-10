use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use dashmap::DashMap;
use interceptor::registry::Registry;
use serde::Serialize;
use tokio::sync::Mutex;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264, MIME_TYPE_PCMU};
use webrtc::api::APIBuilder;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

use super::types::{
    RdpEncodedAudioCodec, RdpEncodedAudioFrame, RdpEncodedVideoCodec, RdpEncodedVideoFrame,
};

pub(super) const DEFAULT_VIDEO_SAMPLE_DURATION: Duration = Duration::from_millis(33);

const H264_START_CODE: [u8; 4] = [0, 0, 0, 1];
const WEBRTC_ICE_GATHER_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RdpWebRtcOffer {
    pub sdp: String,
    pub sdp_type: String,
}

#[derive(Default)]
pub struct RdpWebRtcManager {
    sessions: DashMap<String, Arc<RdpWebRtcSession>>,
}

impl RdpWebRtcManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn create_offer(&self, session_id: String) -> Result<RdpWebRtcOffer, String> {
        self.close(&session_id).await?;

        let mut media_engine = MediaEngine::default();
        media_engine
            .register_default_codecs()
            .map_err(|e| format!("注册 WebRTC 默认 codec 失败: {e}"))?;
        let registry = register_default_interceptors(Registry::new(), &mut media_engine)
            .map_err(|e| format!("注册 WebRTC interceptor 失败: {e}"))?;
        let api = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .build();
        let peer_connection = Arc::new(
            api.new_peer_connection(RTCConfiguration::default())
                .await
                .map_err(|e| format!("创建 WebRTC PeerConnection 失败: {e}"))?,
        );
        let stream_id = format!("rdp-{session_id}");
        let video_track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability {
                mime_type: MIME_TYPE_H264.to_owned(),
                ..Default::default()
            },
            "video".to_owned(),
            stream_id.clone(),
        ));
        let audio_track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability {
                mime_type: MIME_TYPE_PCMU.to_owned(),
                clock_rate: 8_000,
                ..Default::default()
            },
            "audio".to_owned(),
            stream_id,
        ));

        peer_connection
            .add_track(video_track.clone())
            .await
            .map_err(|e| format!("添加 WebRTC H.264 video track 失败: {e}"))?;
        peer_connection
            .add_track(audio_track.clone())
            .await
            .map_err(|e| format!("添加 WebRTC PCMU audio track 失败: {e}"))?;
        let offer = peer_connection
            .create_offer(None)
            .await
            .map_err(|e| format!("创建 WebRTC offer 失败: {e}"))?;
        let mut gather_complete = peer_connection.gathering_complete_promise().await;
        peer_connection
            .set_local_description(offer)
            .await
            .map_err(|e| format!("设置 WebRTC local offer 失败: {e}"))?;
        tokio::time::timeout(WEBRTC_ICE_GATHER_TIMEOUT, gather_complete.recv())
            .await
            .map_err(|_| "WebRTC offer ICE candidate 收集超时".to_string())?;
        let local_description = peer_connection
            .local_description()
            .await
            .ok_or_else(|| "WebRTC local offer 为空".to_string())?;
        self.sessions.insert(
            session_id.clone(),
            Arc::new(RdpWebRtcSession {
                peer_connection,
                video_track,
                audio_track,
                video_clock: Mutex::new(VideoSampleClock::default()),
            }),
        );

        Ok(RdpWebRtcOffer {
            sdp: local_description.sdp,
            sdp_type: local_description.sdp_type.to_string(),
        })
    }

    pub async fn set_answer(&self, session_id: &str, answer_sdp: String) -> Result<(), String> {
        let session = self
            .sessions
            .get(session_id)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| format!("WebRTC 会话不存在: {session_id}"))?;
        let answer = RTCSessionDescription::answer(answer_sdp)
            .map_err(|e| format!("解析 WebRTC answer 失败: {e}"))?;

        session
            .peer_connection
            .set_remote_description(answer)
            .await
            .map_err(|e| format!("设置 WebRTC remote answer 失败: {e}"))?;
        Ok(())
    }

    pub async fn close(&self, session_id: &str) -> Result<(), String> {
        if let Some((_, session)) = self.sessions.remove(session_id) {
            session
                .peer_connection
                .close()
                .await
                .map_err(|e| format!("关闭 WebRTC 会话失败: {e}"))?;
        }

        Ok(())
    }

    pub(super) async fn write_video_frame(
        &self,
        frame: RdpEncodedVideoFrame,
    ) -> Result<(), String> {
        let session = match self.sessions.get(&frame.session_id) {
            Some(entry) => Arc::clone(entry.value()),
            None => return Ok(()),
        };
        let data = match frame.codec {
            RdpEncodedVideoCodec::H264AvcLengthPrefixed => {
                avc_length_prefixed_to_annex_b(&frame.bytes)?
            }
            RdpEncodedVideoCodec::H264AnnexB => frame.bytes.clone(),
        };
        let duration = session
            .video_clock
            .lock()
            .await
            .duration_for_timestamp(frame.timestamp_ms);
        session
            .video_track
            .write_sample(&media::Sample {
                data: Bytes::from(data),
                duration,
                ..Default::default()
            })
            .await
            .map_err(|e| format!("写入 WebRTC H.264 sample 失败: {e}"))?;
        Ok(())
    }

    pub(super) async fn write_audio_frame(
        &self,
        frame: RdpEncodedAudioFrame,
    ) -> Result<(), String> {
        let session = match self.sessions.get(&frame.session_id) {
            Some(entry) => Arc::clone(entry.value()),
            None => return Ok(()),
        };
        if frame.codec != RdpEncodedAudioCodec::Pcmu {
            return Err("WebRTC 音频通道仅支持 G.711 PCMU".to_string());
        }

        let duration = pcmu_sample_duration(frame.bytes.len(), frame.sample_rate, frame.channels)?;
        session
            .audio_track
            .write_sample(&media::Sample {
                data: Bytes::from(frame.bytes),
                duration,
                ..Default::default()
            })
            .await
            .map_err(|e| format!("写入 WebRTC PCMU audio sample 失败: {e}"))
    }
}

struct RdpWebRtcSession {
    peer_connection: Arc<RTCPeerConnection>,
    video_track: Arc<TrackLocalStaticSample>,
    audio_track: Arc<TrackLocalStaticSample>,
    video_clock: Mutex<VideoSampleClock>,
}

#[derive(Default)]
pub(super) struct VideoSampleClock {
    last_timestamp_ms: Option<u64>,
}

impl VideoSampleClock {
    pub(super) fn duration_for_timestamp(&mut self, timestamp_ms: u64) -> Duration {
        let duration = self
            .last_timestamp_ms
            .and_then(|last| timestamp_ms.checked_sub(last))
            .filter(|delta| *delta > 0)
            .map(Duration::from_millis)
            .unwrap_or(DEFAULT_VIDEO_SAMPLE_DURATION);
        self.last_timestamp_ms = Some(timestamp_ms);
        duration
    }
}

pub(super) fn avc_length_prefixed_to_annex_b(payload: &[u8]) -> Result<Vec<u8>, String> {
    let mut offset = 0;
    let mut annex_b = Vec::with_capacity(payload.len());

    while offset < payload.len() {
        let length_end = offset + 4;
        if length_end > payload.len() {
            return Err("H.264 AVC NAL length prefix is incomplete".to_string());
        }

        let nal_len = u32::from_be_bytes(
            payload[offset..length_end]
                .try_into()
                .map_err(|_| "H.264 AVC NAL length prefix is invalid".to_string())?,
        ) as usize;
        if nal_len == 0 {
            return Err("H.264 AVC NAL unit is empty".to_string());
        }

        let nal_start = length_end;
        let nal_end = nal_start
            .checked_add(nal_len)
            .ok_or_else(|| "H.264 AVC NAL length overflows".to_string())?;
        if nal_end > payload.len() {
            return Err("H.264 AVC NAL unit is truncated".to_string());
        }

        annex_b.extend_from_slice(&H264_START_CODE);
        annex_b.extend_from_slice(&payload[nal_start..nal_end]);
        offset = nal_end;
    }

    if annex_b.is_empty() {
        return Err("H.264 AVC payload is empty".to_string());
    }

    Ok(annex_b)
}

pub(super) fn pcmu_sample_duration(
    byte_len: usize,
    sample_rate: u32,
    channels: u16,
) -> Result<Duration, String> {
    if sample_rate == 0 {
        return Err("PCMU sample rate must be greater than zero".to_string());
    }
    if channels == 0 {
        return Err("PCMU channel count must be greater than zero".to_string());
    }

    let samples = byte_len as u128 / u128::from(channels);
    if samples == 0 {
        return Err("PCMU payload is empty".to_string());
    }
    let nanos = samples
        .saturating_mul(1_000_000_000)
        .checked_div(u128::from(sample_rate))
        .ok_or_else(|| "PCMU duration division failed".to_string())?;
    let nanos = u64::try_from(nanos).map_err(|_| "PCMU duration is too large".to_string())?;

    Ok(Duration::from_nanos(nanos))
}

#[tauri::command]
pub async fn rdp_webrtc_create_offer(
    manager: tauri::State<'_, RdpWebRtcManager>,
    session_id: String,
) -> Result<RdpWebRtcOffer, String> {
    manager.create_offer(session_id).await
}

#[tauri::command]
pub async fn rdp_webrtc_set_answer(
    manager: tauri::State<'_, RdpWebRtcManager>,
    session_id: String,
    answer_sdp: String,
) -> Result<(), String> {
    manager.set_answer(&session_id, answer_sdp).await
}

#[tauri::command]
pub async fn rdp_webrtc_close(
    manager: tauri::State<'_, RdpWebRtcManager>,
    session_id: String,
) -> Result<(), String> {
    manager.close(&session_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avc_length_prefixed_payload_converts_to_annex_b() {
        let payload = [
            0, 0, 0, 2, 0x67, 0x42, //
            0, 0, 0, 1, 0x68,
        ];

        assert_eq!(
            avc_length_prefixed_to_annex_b(&payload).unwrap(),
            vec![
                0, 0, 0, 1, 0x67, 0x42, //
                0, 0, 0, 1, 0x68,
            ]
        );
    }

    #[test]
    fn avc_length_prefixed_payload_rejects_truncated_nal_unit() {
        let payload = [0, 0, 0, 4, 0x67, 0x42];

        let error = avc_length_prefixed_to_annex_b(&payload).unwrap_err();

        assert!(error.contains("NAL"));
    }

    #[test]
    fn video_sample_duration_uses_timestamp_delta_when_monotonic() {
        let mut clock = VideoSampleClock::default();

        assert_eq!(
            clock.duration_for_timestamp(1_000),
            DEFAULT_VIDEO_SAMPLE_DURATION
        );
        assert_eq!(clock.duration_for_timestamp(1_033).as_millis(), 33);
        assert_eq!(
            clock.duration_for_timestamp(1_033),
            DEFAULT_VIDEO_SAMPLE_DURATION
        );
    }

    #[test]
    fn pcmu_audio_sample_duration_uses_payload_sample_count() {
        assert_eq!(pcmu_sample_duration(160, 8_000, 1).unwrap().as_millis(), 20);
        assert!(pcmu_sample_duration(160, 0, 1).is_err());
        assert!(pcmu_sample_duration(160, 8_000, 0).is_err());
    }

    #[tokio::test]
    async fn create_offer_completes_without_waiting_forever_for_ice() {
        crate::tls::install_default_crypto_provider();
        let manager = RdpWebRtcManager::new();

        let offer = tokio::time::timeout(
            Duration::from_secs(2),
            manager.create_offer("rdp-offer-timeout-test".to_string()),
        )
        .await
        .expect("create_offer should not wait forever for ICE gathering")
        .unwrap();

        assert_eq!(offer.sdp_type, "offer");
        assert!(offer.sdp.contains("m=video"));
        assert!(offer.sdp.contains("m=audio"));
        manager.close("rdp-offer-timeout-test").await.unwrap();
    }
}
