//! Decide whether an external file drop belongs to a target (port of `utils/dragDropTarget.ts`).

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragRect {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

fn inside(p: DragPoint, r: DragRect) -> bool {
    p.x >= r.left && p.x <= r.right && p.y >= r.top && p.y <= r.bottom
}

/// Platforms report drag coordinates either in logical or in physical pixels.
pub fn is_point_inside_rect_in_either_space(
    point: DragPoint,
    rect: DragRect,
    device_pixel_ratio: f32,
) -> bool {
    inside(point, rect)
        || inside(
            DragPoint {
                x: point.x / device_pixel_ratio,
                y: point.y / device_pixel_ratio,
            },
            rect,
        )
}

pub fn should_accept_external_file_drop(
    path_count: usize,
    is_over_target: bool,
    position: Option<DragPoint>,
    target: Option<DragRect>,
    device_pixel_ratio: f32,
) -> bool {
    if path_count == 0 {
        return false;
    }
    if is_over_target {
        return true;
    }
    match (position, target) {
        (Some(p), Some(r)) => is_point_inside_rect_in_either_space(p, r, device_pixel_ratio),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECT: DragRect = DragRect {
        left: 500.0,
        right: 900.0,
        top: 120.0,
        bottom: 520.0,
    };

    #[test]
    fn accepts_when_an_earlier_over_event_marked_the_target() {
        assert!(should_accept_external_file_drop(1, true, None, None, 1.0));
    }

    #[test]
    fn rejects_without_target_evidence_or_paths() {
        assert!(!should_accept_external_file_drop(1, false, None, None, 1.0));
        assert!(!should_accept_external_file_drop(0, true, None, None, 1.0));
    }

    #[test]
    fn accepts_logical_and_physical_pixel_coordinates() {
        assert!(is_point_inside_rect_in_either_space(
            DragPoint { x: 640.0, y: 300.0 },
            RECT,
            2.0
        ));
        assert!(is_point_inside_rect_in_either_space(
            DragPoint {
                x: 1280.0,
                y: 600.0
            },
            RECT,
            2.0
        ));
        assert!(!is_point_inside_rect_in_either_space(
            DragPoint { x: 10.0, y: 10.0 },
            RECT,
            2.0
        ));
    }
}
