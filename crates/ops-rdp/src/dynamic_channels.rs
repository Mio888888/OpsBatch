use ironrdp::displaycontrol::client::DisplayControlClient;
use ironrdp::displaycontrol::pdu::DisplayControlCapabilities;
use ironrdp::dvc::{DvcChannelListener, DvcMessage, DvcProcessor};
use ironrdp::pdu::PduResult;
use ironrdp::svc::impl_as_any;

const GEOMETRY_CHANNEL: &str = "Microsoft::Windows::RDS::Geometry::v08.01";
const INPUT_CHANNEL: &str = "Microsoft::Windows::RDS::Input";

#[cfg(test)]
pub(super) const WINDOWS_H264_DIRECT_DVC_NAMES: &[&str] = &[
    super::rdpevor::VIDEO_CONTROL_CHANNEL,
    super::rdpevor::VIDEO_DATA_CHANNEL,
    GEOMETRY_CHANNEL,
    INPUT_CHANNEL,
    ironrdp::displaycontrol::CHANNEL_NAME,
];

pub(super) fn display_control_client(session_id: String) -> DisplayControlClient {
    DisplayControlClient::new(move |_caps: DisplayControlCapabilities| {
        let _ = &session_id;
        Ok(Vec::new())
    })
}

pub(super) fn geometry_sink(session_id: String) -> DiagnosticDvcSink {
    DiagnosticDvcSink::new(session_id, GEOMETRY_CHANNEL)
}

pub(super) fn input_sink(session_id: String) -> DiagnosticDvcSink {
    DiagnosticDvcSink::new(session_id, INPUT_CHANNEL)
}

pub(super) struct DiagnosticDvcSink {
    session_id: String,
    channel_name: &'static str,
}

impl_as_any!(DiagnosticDvcSink);

impl DiagnosticDvcSink {
    fn new(session_id: String, channel_name: &'static str) -> Self {
        Self {
            session_id,
            channel_name,
        }
    }
}

impl DvcChannelListener for DiagnosticDvcSink {
    fn channel_name(&self) -> &str {
        self.channel_name
    }

    fn create(&mut self) -> Option<Box<dyn DvcProcessor>> {
        Some(Box::new(Self::new(
            self.session_id.clone(),
            self.channel_name,
        )))
    }
}

impl DvcProcessor for DiagnosticDvcSink {
    fn channel_name(&self) -> &str {
        self.channel_name
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, _payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn close(&mut self, _channel_id: u32) {}
}
