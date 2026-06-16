use std::collections::HashMap;

use ironrdp_egfx::client::{
    BitmapUpdate, EncodedAvc420Frame, GraphicsPipelineClient, GraphicsPipelineHandler, Surface,
};
use ironrdp_egfx::pdu::{
    CapabilitiesV103Flags, CapabilitiesV104Flags, CapabilitiesV107Flags, CapabilitiesV10Flags,
    CapabilitiesV81Flags, CapabilitiesV8Flags, CapabilitySet, GfxPdu,
};
use tokio::sync::mpsc;

use super::types::{RdpBitmapFrame, RdpEncodedVideoCodec, RdpEncodedVideoFrame, RdpStatusDetail};

pub(super) struct RdpEgfxBridge {
    session_id: String,
    encoded_tx: mpsc::UnboundedSender<RdpEncodedVideoFrame>,
    bitmap_tx: mpsc::UnboundedSender<RdpBitmapFrame>,
    status_tx: mpsc::UnboundedSender<RdpStatusDetail>,
    surfaces: HashMap<u16, EgfxSurfaceState>,
    desktop_width: u16,
    desktop_height: u16,
    h264_unavailable_reported: bool,
}

#[derive(Debug, Clone, Copy)]
struct EgfxSurfaceState {
    width: u16,
    height: u16,
    origin_x: u16,
    origin_y: u16,
    mapped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct EgfxCapabilityDiagnostics {
    pub(super) avc420: bool,
    pub(super) avc444: bool,
    pub(super) small_cache: bool,
    pub(super) thin_client: bool,
}

pub(super) fn egfx_capability_diagnostics(caps: &CapabilitySet) -> EgfxCapabilityDiagnostics {
    match caps {
        CapabilitySet::V8 { flags } => EgfxCapabilityDiagnostics {
            avc420: false,
            avc444: false,
            small_cache: flags.contains(CapabilitiesV8Flags::SMALL_CACHE),
            thin_client: flags.contains(CapabilitiesV8Flags::THIN_CLIENT),
        },
        CapabilitySet::V8_1 { flags } => EgfxCapabilityDiagnostics {
            avc420: flags.contains(CapabilitiesV81Flags::AVC420_ENABLED),
            avc444: false,
            small_cache: flags.contains(CapabilitiesV81Flags::SMALL_CACHE),
            thin_client: flags.contains(CapabilitiesV81Flags::THIN_CLIENT),
        },
        CapabilitySet::V10 { flags } | CapabilitySet::V10_2 { flags } => {
            let avc_enabled = !flags.contains(CapabilitiesV10Flags::AVC_DISABLED);
            EgfxCapabilityDiagnostics {
                avc420: avc_enabled,
                avc444: avc_enabled,
                small_cache: flags.contains(CapabilitiesV10Flags::SMALL_CACHE),
                thin_client: false,
            }
        }
        CapabilitySet::V10_1 => EgfxCapabilityDiagnostics {
            avc420: true,
            avc444: true,
            small_cache: false,
            thin_client: false,
        },
        CapabilitySet::V10_3 { flags } => {
            let avc_enabled = !flags.contains(CapabilitiesV103Flags::AVC_DISABLED);
            EgfxCapabilityDiagnostics {
                avc420: avc_enabled,
                avc444: avc_enabled,
                small_cache: false,
                thin_client: flags.contains(CapabilitiesV103Flags::AVC_THIN_CLIENT),
            }
        }
        CapabilitySet::V10_4 { flags }
        | CapabilitySet::V10_5 { flags }
        | CapabilitySet::V10_6 { flags }
        | CapabilitySet::V10_6Err { flags } => {
            let avc_enabled = !flags.contains(CapabilitiesV104Flags::AVC_DISABLED);
            EgfxCapabilityDiagnostics {
                avc420: avc_enabled,
                avc444: avc_enabled,
                small_cache: flags.contains(CapabilitiesV104Flags::SMALL_CACHE),
                thin_client: flags.contains(CapabilitiesV104Flags::AVC_THIN_CLIENT),
            }
        }
        CapabilitySet::V10_7 { flags } => {
            let avc_enabled = !flags.contains(CapabilitiesV107Flags::AVC_DISABLED);
            EgfxCapabilityDiagnostics {
                avc420: avc_enabled,
                avc444: avc_enabled,
                small_cache: flags.contains(CapabilitiesV107Flags::SMALL_CACHE),
                thin_client: flags.contains(CapabilitiesV107Flags::AVC_THIN_CLIENT),
            }
        }
    }
}

impl RdpEgfxBridge {
    /// 构建图形管道客户端并返回多条输出 channel。返回的不是 Self（含外部类型
    /// GraphicsPipelineClient），故抑制 new_should_return Self。
    #[allow(clippy::new_ret_no_self)]
    pub(super) fn new(
        session_id: String,
    ) -> (
        GraphicsPipelineClient,
        mpsc::UnboundedReceiver<RdpEncodedVideoFrame>,
        mpsc::UnboundedReceiver<RdpBitmapFrame>,
        mpsc::UnboundedReceiver<RdpStatusDetail>,
    ) {
        let (encoded_tx, encoded_rx) = mpsc::unbounded_channel();
        let (bitmap_tx, bitmap_rx) = mpsc::unbounded_channel();
        let (status_tx, status_rx) = mpsc::unbounded_channel();
        let handler = Self {
            session_id,
            encoded_tx,
            bitmap_tx,
            status_tx,
            surfaces: HashMap::new(),
            desktop_width: 0,
            desktop_height: 0,
            h264_unavailable_reported: false,
        };
        let client = GraphicsPipelineClient::new(Box::new(handler), None);

        (client, encoded_rx, bitmap_rx, status_rx)
    }

    #[cfg(test)]
    pub(super) fn new_for_tests(
        session_id: String,
        bitmap_tx: mpsc::UnboundedSender<RdpBitmapFrame>,
    ) -> Self {
        let (encoded_tx, _) = mpsc::unbounded_channel();
        let (status_tx, _) = mpsc::unbounded_channel();
        Self {
            session_id,
            encoded_tx,
            bitmap_tx,
            status_tx,
            surfaces: HashMap::new(),
            desktop_width: 0,
            desktop_height: 0,
            h264_unavailable_reported: false,
        }
    }

    fn timestamp_ms(frame: &EncodedAvc420Frame) -> u64 {
        frame.timestamp_ms.unwrap_or(0)
    }
}

impl GraphicsPipelineHandler for RdpEgfxBridge {
    fn capabilities(&self) -> Vec<CapabilitySet> {
        let capabilities = vec![
            CapabilitySet::V10_7 {
                flags: CapabilitiesV107Flags::SMALL_CACHE,
            },
            CapabilitySet::V8_1 {
                flags: CapabilitiesV81Flags::SMALL_CACHE | CapabilitiesV81Flags::AVC420_ENABLED,
            },
            CapabilitySet::V8 {
                flags: CapabilitiesV8Flags::SMALL_CACHE,
            },
        ];
        capabilities
    }

    fn wants_encoded_avc420(&self) -> bool {
        true
    }

    fn on_capabilities_confirmed(&mut self, caps: &CapabilitySet) {
        let diagnostics = egfx_capability_diagnostics(caps);
        if !diagnostics.avc420 && !diagnostics.avc444 && !self.h264_unavailable_reported {
            self.h264_unavailable_reported = true;
            let _ = self.status_tx.send(RdpStatusDetail::H264DirectUnavailable {
                reason: "Windows RDPGFX 未协商 AVC/H.264，服务端正在发送 ClearCodec/bitmap。"
                    .to_string(),
            });
        }
    }

    fn on_reset_graphics(&mut self, width: u32, height: u32) {
        self.surfaces.clear();
        self.desktop_width = u16::try_from(width).unwrap_or(u16::MAX);
        self.desktop_height = u16::try_from(height).unwrap_or(u16::MAX);
    }

    fn on_surface_created(&mut self, surface: &Surface) {
        self.surfaces.insert(
            surface.id,
            EgfxSurfaceState {
                width: surface.width,
                height: surface.height,
                origin_x: u16::try_from(surface.output_origin_x).unwrap_or(u16::MAX),
                origin_y: u16::try_from(surface.output_origin_y).unwrap_or(u16::MAX),
                mapped: surface.is_mapped,
            },
        );
    }

    fn on_surface_deleted(&mut self, surface_id: u16) {
        self.surfaces.remove(&surface_id);
    }

    fn on_surface_mapped(&mut self, surface_id: u16, origin_x: u32, origin_y: u32) {
        if let Some(surface) = self.surfaces.get_mut(&surface_id) {
            surface.origin_x = u16::try_from(origin_x).unwrap_or(u16::MAX);
            surface.origin_y = u16::try_from(origin_y).unwrap_or(u16::MAX);
            surface.mapped = true;
        }
    }

    fn on_bitmap_updated(&mut self, update: &BitmapUpdate) {
        let Some(surface) = self.surfaces.get(&update.surface_id).copied() else {
            return;
        };
        if !surface.mapped {
            return;
        }

        let x = surface
            .origin_x
            .saturating_add(update.destination_rectangle.left);
        let y = surface
            .origin_y
            .saturating_add(update.destination_rectangle.top);
        let width = self
            .desktop_width
            .max(surface.origin_x.saturating_add(surface.width));
        let height = self
            .desktop_height
            .max(surface.origin_y.saturating_add(surface.height));
        let _ = self.bitmap_tx.send(RdpBitmapFrame {
            session_id: self.session_id.clone(),
            surface_id: update.surface_id,
            width,
            height,
            x,
            y,
            region_width: update.width,
            region_height: update.height,
            rgba: update.data.clone(),
        });
    }

    fn on_encoded_avc420(&mut self, frame: &EncodedAvc420Frame) {
        let _ = self.encoded_tx.send(RdpEncodedVideoFrame {
            session_id: self.session_id.clone(),
            codec: RdpEncodedVideoCodec::H264AvcLengthPrefixed,
            frame_id: frame.frame_id.unwrap_or(0),
            timestamp_ms: Self::timestamp_ms(frame),
            is_keyframe: false,
            bytes: frame.data.clone(),
        });
    }

    fn on_frame_complete(&mut self, _frame_id: u32) {}

    fn on_unhandled_pdu(&mut self, _pdu: &GfxPdu) {}

    fn on_close(&mut self) {}
}
