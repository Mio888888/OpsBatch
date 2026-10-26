//! Minimal CSS colour handling for theme data.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// 0.0..=1.0
    pub a: f32,
}

impl Rgba8 {
    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// 0xRRGGBB (alpha dropped).
    pub fn to_u32(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }

    /// WCAG relative luminance.
    pub fn relative_luminance(self) -> f32 {
        let lin = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b)
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb(r, g, b)`, `rgba(r, g, b, a)`.
pub fn parse_css_color(s: &str) -> Option<Rgba8> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        let h = |i: usize, n: usize| u8::from_str_radix(hex.get(i..i + n)?, 16).ok();
        return match hex.len() {
            3 => {
                let d = |i: usize| h(i, 1).map(|v| v * 17);
                Some(Rgba8::rgb(d(0)?, d(1)?, d(2)?))
            }
            6 => Some(Rgba8::rgb(h(0, 2)?, h(2, 2)?, h(4, 2)?)),
            8 => Some(Rgba8 { r: h(0, 2)?, g: h(2, 2)?, b: h(4, 2)?, a: h(6, 2)? as f32 / 255.0 }),
            _ => None,
        };
    }
    let inner = s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb("))?.strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() < 3 || parts.len() > 4 {
        return None;
    }
    let c = |i: usize| parts[i].parse::<f32>().ok().map(|v| v.round().clamp(0.0, 255.0) as u8);
    let a = parts.get(3).map(|a| a.parse::<f32>().ok()).unwrap_or(Some(1.0))?;
    Some(Rgba8 { r: c(0)?, g: c(1)?, b: c(2)?, a: a.clamp(0.0, 1.0) })
}

pub fn contrast_ratio(a: Rgba8, b: Rgba8) -> f32 {
    let (la, lb) = (a.relative_luminance(), b.relative_luminance());
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Channel-wise subtraction (same as the Tauri `darken(hex, amount)`).
pub fn darken(c: Rgba8, amount: u8) -> Rgba8 {
    Rgba8 { r: c.r.saturating_sub(amount), g: c.g.saturating_sub(amount), b: c.b.saturating_sub(amount), a: c.a }
}

pub fn lighten(c: Rgba8, amount: u8) -> Rgba8 {
    Rgba8 { r: c.r.saturating_add(amount), g: c.g.saturating_add(amount), b: c.b.saturating_add(amount), a: c.a }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_short_hex_and_rgba() {
        assert_eq!(parse_css_color("#1677ff"), Some(Rgba8::rgb(0x16, 0x77, 0xff)));
        assert_eq!(parse_css_color("#fff"), Some(Rgba8::rgb(255, 255, 255)));
        let c = parse_css_color("rgba(122, 162, 247, 0.1)").unwrap();
        assert_eq!((c.r, c.g, c.b), (122, 162, 247));
        assert!((c.a - 0.1).abs() < 1e-6);
        assert_eq!(parse_css_color("rgb(1,2,3)"), Some(Rgba8::rgb(1, 2, 3)));
        assert!(parse_css_color("nope").is_none());
        assert!(parse_css_color("#12345").is_none());
    }

    #[test]
    fn contrast_and_hex_roundtrip() {
        let black = Rgba8::rgb(0, 0, 0);
        let white = Rgba8::rgb(255, 255, 255);
        assert!((contrast_ratio(black, white) - 21.0).abs() < 0.01);
        assert_eq!(white.to_hex(), "#ffffff");
        assert_eq!(darken(Rgba8::rgb(10, 100, 200), 20), Rgba8::rgb(0, 80, 180));
        assert_eq!(lighten(Rgba8::rgb(250, 100, 200), 20), Rgba8::rgb(255, 120, 220));
    }
}
