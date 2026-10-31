//! Minimal async RFB client: handshake (3.3/3.7/3.8 incl. Apple's "003.889"), security None / VNC / ARD,
//! 32-bit BGRX true-colour framebuffer, Raw + CopyRect + ZRLE + DesktopSize, input and clipboard messages.

use anyhow::{anyhow, bail, Context, Result};
use flate2::{Decompress, FlushDecompress};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};

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
    /// DesktopSize pseudo-encoding: the framebuffer was replaced by an empty one of this size.
    Resized { width: u16, height: u16 },
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
    pub fn new(width: u16, height: u16) -> Self {
        Self { width, height, data: vec![0; width as usize * height as usize * 4] }
    }
}

pub struct RfbInfo {
    pub name: String,
    pub version: (u32, u32),
    /// The server announced Apple's "003.889": the remote machine is a Mac (Command is its shortcut key).
    pub apple: bool,
    pub width: u16,
    pub height: u16,
}

/// The server's messages are made of many small fields (Hextile reads single bytes): reading them straight from the
/// socket costs a system call each, so they go through a buffer.
const READ_BUFFER: usize = 128 * 1024;

pub struct RfbReader<R> {
    stream: BufReader<R>,
    pub fb: Framebuffer,
    zrle: Decompress,
    /// Called as soon as a FramebufferUpdate starts arriving, before its rectangles are read and decoded.
    update_started: Option<Box<dyn FnMut() + Send>>,
}

/// Write side: all client-to-server messages. Kept separate from the reader so a read in progress is never
/// interrupted by input (reading a message is not cancel-safe).
pub struct RfbWriter<W> {
    stream: W,
    width: u16,
    height: u16,
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

/// ZRLE first (best on slow links), then Hextile, CopyRect, Raw, plus DesktopSize.
const ENCODINGS: [i32; 5] = [16, 5, 1, 0, -223];

pub async fn connect<S: AsyncRead + AsyncWrite + Unpin>(
    mut s: S,
    opts: &Options,
) -> Result<(RfbInfo, RfbReader<ReadHalf<S>>, RfbWriter<WriteHalf<S>>)> {
    // --- version ---
    let mut ver = [0u8; 12];
    s.read_exact(&mut ver).await.context("握手第 1 步: 读取服务器 RFB 版本时连接被对端关闭(服务器接受了 TCP 连接但没有发出 VNC 欢迎信息; 请检查本机代理/VPN 是否接管了该地址, 以及服务器是否限制了来源 IP)")?;
    let text = std::str::from_utf8(&ver).map_err(|_| anyhow!("握手第 1 步: 对端发来的不是 RFB 版本串 {ver:?}"))?;
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
    let server_version = text.trim_end().to_string();
    s.write_all(format!("RFB {cm:03}.{cn:03}\n").as_bytes()).await?;

    // --- security ---
    let has_pw = opts.password.as_deref().is_some_and(|p| !p.is_empty());
    let sec = if cn == 3 {
        let t = s.read_u32().await.with_context(|| format!("握手第 2 步: 读取安全类型时连接被对端关闭(服务器 {server_version})"))?;
        if t == 0 {
            bail!("server refused: {}", read_reason(&mut s).await?);
        }
        t as u8
    } else {
        let n = read_u8(&mut s).await.with_context(|| format!("握手第 2 步: 读取安全类型时连接被对端关闭(服务器 {server_version})"))? as usize;
        if n == 0 {
            bail!("server refused: {}", read_reason(&mut s).await?);
        }
        let mut types = vec![0u8; n];
        s.read_exact(&mut types).await.with_context(|| format!("握手第 2 步: 读取安全类型列表时连接被对端关闭(服务器 {server_version})"))?;
        let chosen = choose_security(&types, opts.prefer, has_pw)
            .ok_or_else(|| anyhow!("服务器 {server_version} 只提供了不支持的认证方式 {types:?}(支持 1=无、2=VNC 密码、30=Apple ARD)"))?;
        s.write_all(&[chosen]).await?;
        chosen
    };

    let auth_stage = || format!("握手第 3 步: 认证({}, 服务器 {server_version})时连接被对端关闭", match sec { SEC_NONE => "无认证", SEC_VNC => "VNC 密码", SEC_ARD => "Apple ARD", _ => "其他" });
    match sec {
        SEC_NONE => {}
        SEC_VNC => {
            let mut ch = [0u8; 16];
            s.read_exact(&mut ch).await.with_context(auth_stage)?;
            let pw = opts.password.clone().unwrap_or_default();
            s.write_all(&vnc_auth_response(&pw, &ch)).await.with_context(auth_stage)?;
        }
        SEC_ARD => {
            let generator = s.read_u16().await.with_context(auth_stage)?;
            let key_len = s.read_u16().await.with_context(auth_stage)? as usize;
            if key_len == 0 || key_len > 1024 {
                bail!("bad ARD key length {key_len}");
            }
            let mut prime = vec![0; key_len];
            s.read_exact(&mut prime).await.with_context(auth_stage)?;
            let mut server_public = vec![0; key_len];
            s.read_exact(&mut server_public).await.with_context(auth_stage)?;
            let (enc, cpub) = ard_client_response(
                &ArdServerHello { generator, prime, server_public },
                opts.username.as_deref().unwrap_or(""),
                opts.password.as_deref().unwrap_or(""),
            )?;
            s.write_all(&enc).await.with_context(auth_stage)?;
            s.write_all(&cpub).await.with_context(auth_stage)?;
        }
        other => bail!("unsupported security type {other}"),
    }
    // SecurityResult is sent for every type in 3.8, and for non-None types in 3.3/3.7.
    if cn == 8 || sec != SEC_NONE {
        let result = s.read_u32().await.with_context(|| format!("握手第 4 步: 等待认证结果时连接被对端关闭(服务器 {server_version}; 认证方式 {sec}; 常见原因: 密码不对而服务器直接断开, 或失败次数过多被临时封锁)"))?;
        if result != 0 {
            let reason = if cn == 8 { read_reason(&mut s).await.unwrap_or_default() } else { String::new() };
            bail!("authentication failed{}", if reason.is_empty() { String::new() } else { format!(": {reason}") });
        }
    }

    // --- init ---
    let init_stage = || format!("握手第 5 步: 读取桌面信息时连接被对端关闭(服务器 {server_version}; 认证已通过, 可能是服务器拒绝共享连接或当前没有可用桌面)");
    s.write_all(&[opts.shared as u8]).await.with_context(init_stage)?;
    let width = s.read_u16().await.with_context(init_stage)?;
    let height = s.read_u16().await.with_context(init_stage)?;
    let mut _pf = [0u8; 16];
    s.read_exact(&mut _pf).await.with_context(init_stage)?;
    let name = {
        let len = s.read_u32().await.with_context(init_stage)? as usize;
        if len > 1 << 16 {
            bail!("server name too long");
        }
        let mut b = vec![0; len];
        s.read_exact(&mut b).await.with_context(init_stage)?;
        String::from_utf8_lossy(&b).into_owned()
    };

    let mut writer_msg = pixel_format_message();
    writer_msg.extend(set_encodings_message(&ENCODINGS));
    s.write_all(&writer_msg).await?;
    let (r, w) = tokio::io::split(s);
    Ok((
        RfbInfo { name, version: (cm, cn), apple: server_version.starts_with("RFB 003.889"), width, height },
        RfbReader { stream: BufReader::with_capacity(READ_BUFFER, r), fb: Framebuffer::new(width, height), zrle: Decompress::new(true), update_started: None },
        RfbWriter { stream: w, width, height },
    ))
}

fn pixel_format_message() -> Vec<u8> {
    // 32bpp depth 24 little-endian true colour, R<<16 G<<8 B => bytes in memory: B G R X (BGRA).
    let mut m = vec![0u8, 0, 0, 0];
    m.extend_from_slice(&[32, 24, 0, 1]);
    m.extend_from_slice(&255u16.to_be_bytes());
    m.extend_from_slice(&255u16.to_be_bytes());
    m.extend_from_slice(&255u16.to_be_bytes());
    m.extend_from_slice(&[16, 8, 0, 0, 0, 0]);
    m
}

fn set_encodings_message(encs: &[i32]) -> Vec<u8> {
    let mut m = vec![2u8, 0];
    m.extend_from_slice(&(encs.len() as u16).to_be_bytes());
    for e in encs {
        m.extend_from_slice(&e.to_be_bytes());
    }
    m
}

impl<W: AsyncWrite + Unpin> RfbWriter<W> {
    /// The server resized its desktop; update requests must cover the new size.
    pub fn set_size(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
    }

    pub async fn request_update(&mut self, incremental: bool) -> Result<()> {
        self.send_raw(&self.update_request(incremental)).await
    }

    /// FramebufferUpdateRequest for the whole desktop, for batching with [`Self::send_raw`].
    pub fn update_request(&self, incremental: bool) -> [u8; 10] {
        let (w, h) = (self.width.to_be_bytes(), self.height.to_be_bytes());
        [3, incremental as u8, 0, 0, 0, 0, w[0], w[1], h[0], h[1]]
    }

    pub async fn send_key(&mut self, keysym: u32, down: bool) -> Result<()> {
        self.send_raw(&key_message(keysym, down)).await
    }

    pub async fn send_pointer(&mut self, x: u16, y: u16, buttons: u8) -> Result<()> {
        self.send_raw(&pointer_message(x, y, buttons)).await
    }

    pub async fn send_cut_text(&mut self, text: &str) -> Result<()> {
        self.send_raw(&cut_text_message(text)).await
    }

    /// Several messages built with the `*_message` helpers go out in one write.
    pub async fn send_raw(&mut self, bytes: &[u8]) -> Result<()> {
        self.stream.write_all(bytes).await?;
        Ok(())
    }
}

pub fn key_message(keysym: u32, down: bool) -> [u8; 8] {
    let k = keysym.to_be_bytes();
    [4, down as u8, 0, 0, k[0], k[1], k[2], k[3]]
}

pub fn pointer_message(x: u16, y: u16, buttons: u8) -> [u8; 6] {
    let (x, y) = (x.to_be_bytes(), y.to_be_bytes());
    [5, buttons, x[0], x[1], y[0], y[1]]
}

pub fn cut_text_message(text: &str) -> Vec<u8> {
    // Latin-1 only per RFC 6143; characters outside it become '?'.
    let bytes: Vec<u8> = text.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect();
    let mut m = vec![6u8, 0, 0, 0];
    m.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    m.extend_from_slice(&bytes);
    m
}

impl<R: AsyncRead + Unpin> RfbReader<R> {
    /// `hook` runs when a FramebufferUpdate starts arriving. Asking for the next update right then (instead of after
    /// the whole one is read and decoded) keeps the server sending while this side is still busy.
    pub fn set_update_hook(&mut self, hook: impl FnMut() + Send + 'static) {
        self.update_started = Some(Box::new(hook));
    }

    /// Read one server message and apply it to the framebuffer.
    pub async fn next_event(&mut self) -> Result<ServerEvent> {
        match read_u8(&mut self.stream).await? {
            0 => {
                let _pad = read_u8(&mut self.stream).await?;
                let n = self.stream.read_u16().await?;
                if let Some(hook) = self.update_started.as_mut() {
                    hook();
                }
                let mut rects = Vec::new();
                let mut resized = None;
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
                        5 => self.read_hextile(r).await?,
                        16 => self.read_zrle(r).await?,
                        -223 => {
                            self.fb = Framebuffer::new(r.w, r.h);
                            resized = Some((r.w, r.h));
                        }
                        other => bail!("unsupported encoding {other}"),
                    }
                    rects.push(r);
                }
                if let Some((width, height)) = resized {
                    return Ok(ServerEvent::Resized { width, height });
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
        let row_len = r.w as usize * 4;
        for y in 0..r.h as usize {
            let o = ((r.y as usize + y) * self.fb.width as usize + r.x as usize) * 4;
            let row = &mut self.fb.data[o..o + row_len];
            self.stream.read_exact(row).await?;
            // the fourth byte of a 32-bit pixel is padding
            row.chunks_exact_mut(4).for_each(|px| px[3] = 255);
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
        let row_len = r.w as usize * 4;
        // Rows move in place (memmove per row); the order keeps overlapping source rows intact.
        let rows = 0..r.h as usize;
        let mut copy = |y: usize| {
            let from = (sy + y) * stride + sx * 4;
            let to = (r.y as usize + y) * stride + r.x as usize * 4;
            self.fb.data.copy_within(from..from + row_len, to);
        };
        if r.y as usize > sy {
            rows.rev().for_each(&mut copy);
        } else {
            rows.for_each(&mut copy);
        }
        Ok(())
    }

    async fn read_hextile(&mut self, r: Rect) -> Result<()> {
        self.clip(r)?;
        let stride = self.fb.width as usize * 4;
        let mut background = [0u8, 0, 0, 255];
        let mut foreground = [0u8, 0, 0, 255];
        let mut tile = [0u8; 16 * 16 * 4];
        let mut ty = 0u16;
        while ty < r.h {
            let th = (r.h - ty).min(16) as usize;
            let mut tx = 0u16;
            while tx < r.w {
                let tw = (r.w - tx).min(16) as usize;
                let sub = self.stream.read_u8().await?;
                let origin = (r.y as usize + ty as usize) * stride + (r.x as usize + tx as usize) * 4;
                // fills the part of a tile's sub-rectangle that lies inside the tile
                let put = |fb: &mut Framebuffer, x: usize, y: usize, w: usize, h: usize, c: [u8; 4]| {
                    if x >= tw {
                        return;
                    }
                    for yy in y..(y + h).min(th) {
                        let o = origin + yy * stride + x * 4;
                        fb.data[o..o + ((x + w).min(tw) - x) * 4].chunks_exact_mut(4).for_each(|px| px.copy_from_slice(&c));
                    }
                };
                if sub & 1 != 0 {
                    // raw tile: one read, then row copies
                    let bytes = &mut tile[..tw * th * 4];
                    self.stream.read_exact(bytes).await?;
                    for (y, src) in bytes.chunks_exact(tw * 4).enumerate() {
                        let o = origin + y * stride;
                        let row = &mut self.fb.data[o..o + tw * 4];
                        row.copy_from_slice(src);
                        row.chunks_exact_mut(4).for_each(|px| px[3] = 255);
                    }
                } else {
                    if sub & 2 != 0 {
                        let mut px = [0u8; 4];
                        self.stream.read_exact(&mut px).await?;
                        background = [px[0], px[1], px[2], 255];
                    }
                    if sub & 4 != 0 {
                        let mut px = [0u8; 4];
                        self.stream.read_exact(&mut px).await?;
                        foreground = [px[0], px[1], px[2], 255];
                    }
                    put(&mut self.fb, 0, 0, tw, th, background);
                    if sub & 8 != 0 {
                        let count = self.stream.read_u8().await?;
                        for _ in 0..count {
                            let colour = if sub & 16 != 0 {
                                let mut px = [0u8; 4];
                                self.stream.read_exact(&mut px).await?;
                                [px[0], px[1], px[2], 255]
                            } else {
                                foreground
                            };
                            let xy = self.stream.read_u8().await?;
                            let wh = self.stream.read_u8().await?;
                            let (x, y) = ((xy >> 4) as usize, (xy & 15) as usize);
                            let (w, h) = (((wh >> 4) + 1) as usize, ((wh & 15) + 1) as usize);
                            put(&mut self.fb, x, y, w, h, colour);
                        }
                    }
                }
                tx += 16;
            }
            ty += 16;
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
            let out_before = out.len();
            self.zrle
                .decompress_vec(&comp[consumed..], &mut out, FlushDecompress::Sync)
                .map_err(|e| anyhow!("zlib: {e}"))?;
            if (self.zrle.total_in() - before_in) as usize == consumed && out.len() == out_before {
                bail!("ZRLE stream made no progress");
            }
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
    let mut scratch = vec![[0u8; 4]; 64 * 64];
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
            let pixels = &mut scratch[..tw * th];
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
                fb.data[o..o + tw * 4].copy_from_slice(pixels[y * tw..(y + 1) * tw].as_flattened());
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
