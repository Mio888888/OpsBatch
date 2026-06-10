use super::types::{RdpFramePayload, RectRegion};

pub(super) fn build_frame_payload(
    session_id: &str,
    width: u16,
    height: u16,
    image: &[u8],
    region: RectRegion,
) -> Result<RdpFramePayload, String> {
    let bytes_per_pixel = 4usize;
    let expected_len = usize::from(width) * usize::from(height) * bytes_per_pixel;
    if image.len() != expected_len {
        return Err(format!(
            "RDP framebuffer 大小不匹配: expected={}, actual={}",
            expected_len,
            image.len()
        ));
    }
    if region.width == 0 || region.height == 0 {
        return Err("RDP framebuffer 更新区域为空".to_string());
    }
    let right = region
        .x
        .checked_add(region.width)
        .ok_or_else(|| "RDP framebuffer 更新区域超出范围".to_string())?;
    let bottom = region
        .y
        .checked_add(region.height)
        .ok_or_else(|| "RDP framebuffer 更新区域超出范围".to_string())?;
    if right > width || bottom > height {
        return Err("RDP framebuffer 更新区域超出画面尺寸".to_string());
    }

    let mut rgba = Vec::with_capacity(usize::from(region.width) * usize::from(region.height) * 4);
    let stride = usize::from(width) * bytes_per_pixel;
    let row_len = usize::from(region.width) * bytes_per_pixel;
    for row in region.y..bottom {
        let start = usize::from(row) * stride + usize::from(region.x) * bytes_per_pixel;
        rgba.extend_from_slice(&image[start..start + row_len]);
    }

    Ok(RdpFramePayload {
        session_id: session_id.to_string(),
        width,
        height,
        x: region.x,
        y: region.y,
        region_width: region.width,
        region_height: region.height,
        rgba,
    })
}
