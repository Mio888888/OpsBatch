//! Minimal async RFB client: handshake (3.3/3.7/3.8 incl. Apple's "003.889"), security None / VNC / ARD,
//! 32-bit BGRX true-colour framebuffer, Raw + CopyRect + ZRLE + DesktopSize, input and clipboard messages.

use anyhow::{anyhow, bail, Result};
use flate2::{Decompress, FlushDecompress};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::auth::{ard_client_response, vnc_auth_response, ArdServerHello};

pub const SEC_NONE: u8 = 1;
pub const SEC_VNC: u8 = 2;
pub const SEC_ARD: u8 = 30;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AuthPreference {
    Vnc,
    Ard,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub username: Option<String>,
    pub password: Option<String>,
    pub prefer: AuthPreference,
    pub shared: bool,
}

/// Order the server's offered types by our preference (mirrors OpsBatch's `order_vnc_security_types`).
pub fn choose_security(offered: &[u8], prefer: AuthPreference, has_password: bool) -> Option<u8> {
    let order: &[u8] = match (prefer, has_password) {
        (AuthPreference::Ard, _) => &[SEC_ARD, SEC_VNC, SEC_NONE],
        (AuthPreference::Vnc, true) => &[SEC_VNC, SEC_ARD, SEC_NONE],
        (AuthPreference::Vnc, false) => &[SEC_NONE, SEC_VNC, SEC_ARD],
    };
    order.iter().copied().find(|t| offered.contains(t))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

#[derive(Debug)]
pub enum ServerEvent {
    Update(Vec<Rect>),
    Bell,
    CutText(String),
    ColorMap,
}

/// BGRA framebuffer (matches GPUI's `RenderImage` byte order).
pub struct Framebuffer {
    pub width: u16,
    pub height: u16,
    pub data: Vec<u8>,
}

impl Framebuffer {
    fn new(width: u16, height: u16) -> Self {
        Self { width, height, data: vec![0; width as usize * height as usize * 4] }
    }
}

pub struct RfbClient<S> {
    stream: S,
    pub fb: Framebuffer,
    pub name: String,
    pub version: (u32, u32),
    zrle: Decompress,
}

async fn read_u8<S: AsyncRead + Unpin>(s: &mut S) -> Result<u8> {
    Ok(s.read_u8().await?)
}

async fn read_reason<S: AsyncRead + Unpin>(s: &mut S) -> Result<String> {
    let len = s.read_u32().await? as usize;
    if len > 4096 {
        bail!("reason string too long");
    }
    let mut buf = vec![0; len];
    s.read_exact(&mut buf).await?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

impl<S: AsyncRead + AsyncWrite + Unpin> RfbClient<S> {
    pub async fn connect(mut s: S, opts: &Options) -> Result<Self> {
        // --- version ---
        let mut ver = [0u8; 12];
        s.read_exact(&mut ver).await?;
        let text = std::str::from_utf8(&ver)?;
        if !text.starts_with("RFB ") || ver[11] != b'\n' {
            bail!("not an RFB server: {text:?}");
        }
        let major: u32 = text[4..7].parse()?;
        let minor: u32 = text[8..11].parse()?;
        // Apple reports 003.889; anything >= 3.8 is spoken as 3.8, 3.7 as 3.7, else 3.3.
        let (cm, cn) = match (major, minor) {
            (3, n) if n >= 8 => (3, 8),
            (3, 7) => (3, 7),
            (m, _) if m > 3 => (3, 8),
            _ => (3, 3),
        };
        s.write_all(format!("RFB {cm:03}.{cn:03}\n").as_bytes()).await?;

        // --- security ---
        let has_pw = opts.password.as_deref().is_some_and(|p| !p.is_empty());
        let sec = if cn == 3 {
            let t = s.read_u32().await?;
            if t == 0 {
                bail!("server refused: {}", read_reason(&mut s).await?);
            }
            t as u8
        } else {
            let n = read_u8(&mut s).await? as usize;
            if n == 0 {
                bail!("server refused: {}", read_reason(&mut s).await?);
            }
            let mut types = vec![0u8; n];
            s.read_exact(&mut types).await?;
            let chosen = choose_security(&types, opts.prefer, has_pw)
                .ok_or_else(|| anyhow!("no supported security type in {types:?}"))?;
            s.write_all(&[chosen]).await?;
            chosen
        };

        match sec {
            SEC_NONE => {}
            SEC_VNC => {
                let mut ch = [0u8; 16];
                s.read_exact(&mut ch).await?;
                let pw = opts.password.clone().unwrap_or_default();
                s.write_all(&vnc_auth_response(&pw, &ch)).await?;
            }
            SEC_ARD => {
                let generator = s.read_u16().await?;
                let key_len = s.read_u16().await? as usize;
                if key_len == 0 || key_len > 1024 {
                    bail!("bad ARD key length {key_len}");
                }
                let mut prime = vec![0; key_len];
                s.read_exact(&mut prime).await?;
                let mut server_public = vec![0; key_len];
                s.read_exact(&mut server_public).await?;
                let (enc, cpub) = ard_client_response(
                    &ArdServerHello { generator, prime, server_public },
                    opts.username.as_deref().unwrap_or(""),
                    opts.password.as_deref().unwrap_or(""),
                )?;
                s.write_all(&enc).await?;
                s.write_all(&cpub).await?;
            }
            other => bail!("unsupported security type {other}"),
        }
        // SecurityResult is sent for every type in 3.8, and for non-None types in 3.3/3.7.
        if cn == 8 || sec != SEC_NONE {
            if s.read_u32().await? != 0 {
                let reason = if cn == 8 { read_reason(&mut s).await.unwrap_or_default() } else { String::new() };
                bail!("authentication failed{}", if reason.is_empty() { String::new() } else { format!(": {reason}") });
            }
        }

        // --- init ---
        s.write_all(&[opts.shared as u8]).await?;
        let width = s.read_u16().await?;
        let height = s.read_u16().await?;
        let mut _pf = [0u8; 16];
        s.read_exact(&mut _pf).await?;
        let name = {
            let len = s.read_u32().await? as usize;
            if len > 1 << 16 {
                bail!("server name too long");
            }
            let mut b = vec![0; len];
            s.read_exact(&mut b).await?;
            String::from_utf8_lossy(&b).into_owned()
        };

        let mut client = Self {
            stream: s,
            fb: Framebuffer::new(width, height),
            name,
            version: (cm, cn),
            zrle: Decompress::new(true),
        };
        client.set_pixel_format().await?;
        client.set_encodings(&[16, 1, 0, -223]).await?; // ZRLE, CopyRect, Raw, DesktopSize
        Ok(client)
    }

    async fn set_pixel_format(&mut self) -> Result<()> {
        // 32bpp depth 24 little-endian true colour, R<<16 G<<8 B => bytes in memory: B G R X (BGRA).
        let mut m = vec![0u8, 0, 0, 0];
        m.extend_from_slice(&[32, 24, 0, 1]);
        m.extend_from_slice(&255u16.to_be_bytes());
        m.extend_from_slice(&255u16.to_be_bytes());
        m.extend_from_slice(&255u16.to_be_bytes());
        m.extend_from_slice(&[16, 8, 0, 0, 0, 0]);
        self.stream.write_all(&m).await?;
        Ok(())
    }

    async fn set_encodings(&mut self, encs: &[i32]) -> Result<()> {
        let mut m = vec![2u8, 0];
        m.extend_from_slice(&(encs.len() as u16).to_be_bytes());
        for e in encs {
            m.extend_from_slice(&e.to_be_bytes());
        }
        self.stream.write_all(&m).await?;
        Ok(())
    }

    pub async fn request_update(&mut self, incremental: bool) -> Result<()> {
        let mut m = vec![3u8, incremental as u8];
        for v in [0u16, 0, self.fb.width, self.fb.height] {
            m.extend_from_slice(&v.to_be_bytes());
        }
        self.stream.write_all(&m).await?;
        Ok(())
    }

    pub async fn send_key(&mut self, keysym: u32, down: bool) -> Result<()> {
        let mut m = vec![4u8, down as u8, 0, 0];
        m.extend_from_slice(&keysym.to_be_bytes());
        self.stream.write_all(&m).await?;
        Ok(())
    }

    pub async fn send_pointer(&mut self, x: u16, y: u16, buttons: u8) -> Result<()> {
        let mut m = vec![5u8, buttons];
        m.extend_from_slice(&x.to_be_bytes());
        m.extend_from_slice(&y.to_be_bytes());
        self.stream.write_all(&m).await?;
        Ok(())
    }

    pub async fn send_cut_text(&mut self, text: &str) -> Result<()> {
        // Latin-1 only per RFC 6143; non-Latin-1 needs the extended clipboard pseudo-encoding (TODO M8).
        let bytes: Vec<u8> = text.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect();
        let mut m = vec![6u8, 0, 0, 0];
        m.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        m.extend_from_slice(&bytes);
        self.stream.write_all(&m).await?;
        Ok(())
    }

    /// Read one server message and apply it to the framebuffer.
    pub async fn next_event(&mut self) -> Result<ServerEvent> {
        match read_u8(&mut self.stream).await? {
            0 => {
                let _pad = read_u8(&mut self.stream).await?;
                let n = self.stream.read_u16().await?;
                let mut rects = Vec::new();
                for _ in 0..n {
                    let r = Rect {
                        x: self.stream.read_u16().await?,
                        y: self.stream.read_u16().await?,
                        w: self.stream.read_u16().await?,
                        h: self.stream.read_u16().await?,
                    };
                    let enc = self.stream.read_i32().await?;
                    match enc {
                        0 => self.read_raw(r).await?,
                        1 => self.read_copyrect(r).await?,
                        16 => self.read_zrle(r).await?,
                        -223 => {
                            self.fb = Framebuffer::new(r.w, r.h);
                        }
                        other => bail!("unsupported encoding {other}"),
                    }
                    rects.push(r);
                }
                Ok(ServerEvent::Update(rects))
            }
            1 => {
                let mut hdr = [0u8; 5];
                self.stream.read_exact(&mut hdr).await?;
                let n = u16::from_be_bytes([hdr[1], hdr[2]]) as usize;
                let mut skip = vec![0; n * 6];
                self.stream.read_exact(&mut skip).await?;
                Ok(ServerEvent::ColorMap)
            }
            2 => Ok(ServerEvent::Bell),
            3 => {
                let mut pad = [0u8; 3];
                self.stream.read_exact(&mut pad).await?;
                let len = self.stream.read_u32().await? as usize;
                if len > 16 << 20 {
                    bail!("cut text too large");
                }
                let mut b = vec![0; len];
                self.stream.read_exact(&mut b).await?;
                Ok(ServerEvent::CutText(b.iter().map(|&c| c as char).collect()))
            }
            other => bail!("unknown server message {other}"),
        }
    }

    fn clip(&self, r: Rect) -> Result<()> {
        if r.x as u32 + r.w as u32 > self.fb.width as u32 || r.y as u32 + r.h as u32 > self.fb.height as u32 {
            bail!("rect {r:?} outside {}x{} framebuffer", self.fb.width, self.fb.height);
        }
        Ok(())
    }

    async fn read_raw(&mut self, r: Rect) -> Result<()> {
        self.clip(r)?;
        let mut row = vec![0u8; r.w as usize * 4];
        for y in 0..r.h as usize {
            self.stream.read_exact(&mut row).await?;
            let o = ((r.y as usize + y) * self.fb.width as usize + r.x as usize) * 4;
            for (i, px) in row.chunks_exact(4).enumerate() {
                self.fb.data[o + i * 4..o + i * 4 + 3].copy_from_slice(&px[..3]);
                self.fb.data[o + i * 4 + 3] = 255;
            }
        }
        Ok(())
    }

    async fn read_copyrect(&mut self, r: Rect) -> Result<()> {
        self.clip(r)?;
        let sx = self.stream.read_u16().await? as usize;
        let sy = self.stream.read_u16().await? as usize;
        let src = Rect { x: sx as u16, y: sy as u16, w: r.w, h: r.h };
        self.clip(src)?;
        let stride = self.fb.width as usize * 4;
        let tmp: Vec<u8> = (0..r.h as usize)
            .flat_map(|y| {
                let o = (sy + y) * stride + sx * 4;
                self.fb.data[o..o + r.w as usize * 4].to_vec()
            })
            .collect();
        for y in 0..r.h as usize {
            let o = (r.y as usize + y) * stride + r.x as usize * 4;
            self.fb.data[o..o + r.w as usize * 4].copy_from_slice(&tmp[y * r.w as usize * 4..(y + 1) * r.w as usize * 4]);
        }
        Ok(())
    }

    async fn read_zrle(&mut self, r: Rect) -> Result<()> {
        self.clip(r)?;
        let len = self.stream.read_u32().await? as usize;
        if len > 64 << 20 {
            bail!("ZRLE payload too large");
        }
        let mut comp = vec![0; len];
        self.stream.read_exact(&mut comp).await?;
        // One persistent zlib stream for the whole connection. Output bound: 64x64 tiles, <= 4 bytes/px worst case + headers.
        let tiles = (r.w as usize).div_ceil(64) * (r.h as usize).div_ceil(64);
        let cap = r.w as usize * r.h as usize * 4 + tiles * 8 + 64;
        let mut out = Vec::with_capacity(cap);
        let before_in = self.zrle.total_in();
        while (self.zrle.total_in() - before_in) < comp.len() as u64 {
            let consumed = (self.zrle.total_in() - before_in) as usize;
            if out.capacity() - out.len() < 4096 {
                out.reserve(cap);
            }
            self.zrle
                .decompress_vec(&comp[consumed..], &mut out, FlushDecompress::Sync)
                .map_err(|e| anyhow!("zlib: {e}"))?;
        }
        decode_zrle_tiles(&out, r, &mut self.fb)
    }
}

fn decode_zrle_tiles(data: &[u8], r: Rect, fb: &mut Framebuffer) -> Result<()> {
    let mut p = 0usize;
    let need = |p: usize, n: usize| -> Result<()> {
        if p + n > data.len() {
            bail!("ZRLE truncated")
        } else {
            Ok(())
        }
    };
    let cpix = |d: &[u8]| -> [u8; 4] { [d[0], d[1], d[2], 255] }; // CPIXEL = B,G,R (3 bytes) for our format
    let stride = fb.width as usize * 4;
    let mut ty = 0u16;
    while ty < r.h {
        let th = (r.h - ty).min(64) as usize;
        let mut tx = 0u16;
        while tx < r.w {
            let tw = (r.w - tx).min(64) as usize;
            need(p, 1)?;
            let sub = data[p];
            p += 1;
            let packed = sub & 0x80 != 0; // RLE
            let pal_size = (sub & 0x7f) as usize;
            let mut pixels = vec![[0u8; 4]; tw * th];
            match (packed, pal_size) {
                (false, 0) => {
                    need(p, tw * th * 3)?;
                    for px in pixels.iter_mut() {
                        *px = cpix(&data[p..]);
                        p += 3;
                    }
                }
                (false, 1) => {
                    need(p, 3)?;
                    let c = cpix(&data[p..]);
                    p += 3;
                    pixels.fill(c);
                }
                (false, 2..=16) => {
                    need(p, pal_size * 3)?;
                    let pal: Vec<[u8; 4]> = (0..pal_size).map(|i| cpix(&data[p + i * 3..])).collect();
                    p += pal_size * 3;
                    let bits = match pal_size {
                        2 => 1,
                        3..=4 => 2,
                        _ => 4,
                    };
                    let row_bytes = (tw * bits).div_ceil(8);
                    need(p, row_bytes * th)?;
                    for y in 0..th {
                        for x in 0..tw {
                            let bit = x * bits;
                            let byte = data[p + y * row_bytes + bit / 8];
                            let idx = ((byte >> (8 - bits - (bit % 8))) & ((1 << bits) - 1)) as usize;
                            pixels[y * tw + x] = *pal.get(idx).ok_or_else(|| anyhow!("bad palette index"))?;
                        }
                    }
                    p += row_bytes * th;
                }
                (true, 0) => {
                    let mut i = 0;
                    while i < pixels.len() {
                        need(p, 3)?;
                        let c = cpix(&data[p..]);
                        p += 3;
                        let run = read_run(data, &mut p)?;
                        if i + run > pixels.len() {
                            bail!("RLE overrun");
                        }
                        pixels[i..i + run].fill(c);
                        i += run;
                    }
                }
                (true, 2..=127) => {
                    need(p, pal_size * 3)?;
                    let pal: Vec<[u8; 4]> = (0..pal_size).map(|i| cpix(&data[p + i * 3..])).collect();
                    p += pal_size * 3;
                    let mut i = 0;
                    while i < pixels.len() {
                        need(p, 1)?;
                        let b = data[p];
                        p += 1;
                        let idx = (b & 0x7f) as usize;
                        let c = *pal.get(idx).ok_or_else(|| anyhow!("bad palette index"))?;
                        let run = if b & 0x80 != 0 { read_run(data, &mut p)? } else { 1 };
                        if i + run > pixels.len() {
                            bail!("RLE overrun");
                        }
                        pixels[i..i + run].fill(c);
                        i += run;
                    }
                }
                _ => bail!("invalid ZRLE subencoding {sub}"),
            }
            for y in 0..th {
                let o = (r.y as usize + ty as usize + y) * stride + (r.x as usize + tx as usize) * 4;
                for x in 0..tw {
                    fb.data[o + x * 4..o + x * 4 + 4].copy_from_slice(&pixels[y * tw + x]);
                }
            }
            tx += 64;
        }
        ty += 64;
    }
    Ok(())
}

fn read_run(data: &[u8], p: &mut usize) -> Result<usize> {
    let mut run = 1usize;
    loop {
        if *p >= data.len() {
            bail!("ZRLE run truncated");
        }
        let b = data[*p] as usize;
        *p += 1;
        run += b;
        if b != 255 {
            return Ok(run);
        }
    }
}
