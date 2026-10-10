//! Ogg Opus decoder (RFC 7845, 09 §1): the `ogg` crate demuxes, libopus (built from source by
//! `opusic-sys`) decodes. Output is always 48 kHz. Pre-skip and the last granule position trim
//! both ends (gapless), the header output gain is applied, and seeks are sample-exact: a
//! bisection over page granule positions, then 80 ms of pre-roll decoded and discarded.

use std::collections::VecDeque;
use std::ffi::CStr;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::ptr::NonNull;

use ogg::reading::PacketReader;
use opusic_sys as ffi;

use crate::symphonia_decoder::local_path;
use crate::{DecodeError, Decoder, DecoderFactory, StreamInfo};

/// Output sample rate (Opus granule positions always count 48 kHz samples).
pub const RATE: u32 = 48_000;
/// Decoder pre-roll before a seek target (RFC 7845 §4.6).
const PREROLL: u64 = 3_840;
/// Longest Opus packet (120 ms).
const MAX_FRAME: usize = 5_760;
/// Bytes searched per bisection step.
const CHUNK: usize = 64 * 1024;
/// Largest possible Ogg page.
const MAX_PAGE: usize = 27 + 255 + 255 * 255;
/// Granule position of a page on which no packet ends.
const NO_GRANULE: u64 = u64::MAX;

/// Vorbis channel order (RFC 7845 §5.1.1.2) to WAV order, for mapping family 1.
const REORDER: [&[usize]; 9] = [
    &[],
    &[0],
    &[0, 1],
    &[0, 2, 1],
    &[0, 1, 2, 3],
    &[0, 2, 1, 3, 4],
    &[0, 2, 1, 5, 3, 4],
    &[0, 2, 1, 6, 5, 3, 4],
    &[0, 2, 1, 7, 5, 6, 3, 4],
];

pub struct OpusFactory;

impl DecoderFactory for OpusFactory {
    fn name(&self) -> &str {
        "Opus"
    }

    fn extensions(&self) -> Vec<(String, String)> {
        vec![("opus".into(), "Opus Files".into())]
    }

    /// Claims `.ogg`/`.oga` files whose first logical stream is Opus.
    fn claims(&self, location: &str) -> bool {
        matches!(
            crate::extension_of(location).as_deref(),
            Some("ogg" | "oga")
        ) && local_path(location).is_some_and(|p| is_ogg_opus(Path::new(&p)))
    }

    fn open(&self, location: &str) -> Result<Box<dyn Decoder>, DecodeError> {
        Ok(Box::new(OpusDecoder::open(location)?))
    }
}

/// Whether a file starts with an Ogg page carrying an Opus identification header.
pub fn is_ogg_opus(path: &Path) -> bool {
    let mut buf = [0u8; 300];
    let Ok(mut f) = File::open(path) else {
        return false;
    };
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => n += k,
        }
    }
    let buf = &buf[..n];
    if buf.len() < 27 || &buf[..4] != b"OggS" {
        return false;
    }
    let start = 27 + buf[26] as usize;
    buf.get(start..start + 8) == Some(b"OpusHead")
}

/// The identification header (RFC 7845 §5.1).
#[derive(Debug, Clone, PartialEq)]
pub struct OpusHead {
    pub channels: u8,
    pub pre_skip: u16,
    /// Sample rate of the original input (informational only).
    pub input_rate: u32,
    /// Output gain in Q7.8 dB.
    pub output_gain: i16,
    pub mapping_family: u8,
    pub streams: u8,
    pub coupled: u8,
    pub mapping: Vec<u8>,
}

impl OpusHead {
    pub fn parse(p: &[u8]) -> Result<Self, DecodeError> {
        let bad = |m: &str| DecodeError::Open(format!("Opus header: {m}"));
        if p.len() < 19 || &p[..8] != b"OpusHead" {
            return Err(bad("missing OpusHead"));
        }
        if p[8] & 0xF0 != 0 {
            return Err(DecodeError::Unsupported(format!("Opus version {}", p[8])));
        }
        let channels = p[9];
        if channels == 0 {
            return Err(bad("zero channels"));
        }
        let family = p[18];
        let (streams, coupled, mapping) = match family {
            0 if channels <= 2 => (1, channels - 1, (0..channels).collect()),
            0 => return Err(bad("family 0 with more than 2 channels")),
            1 => {
                if channels > 8 {
                    return Err(bad("family 1 with more than 8 channels"));
                }
                let table = p
                    .get(19..21 + channels as usize)
                    .ok_or_else(|| bad("truncated channel mapping"))?;
                let (streams, coupled) = (table[0], table[1]);
                let mapping = table[2..].to_vec();
                let total = streams as u16 + coupled as u16;
                if streams == 0 || coupled > streams || total > 255 {
                    return Err(bad("invalid stream counts"));
                }
                if mapping.iter().any(|&m| m != 255 && m as u16 >= total) {
                    return Err(bad("invalid channel mapping"));
                }
                (streams, coupled, mapping)
            }
            f => {
                return Err(DecodeError::Unsupported(format!(
                    "Opus channel mapping family {f}"
                )));
            }
        };
        Ok(Self {
            channels,
            pre_skip: u16::from_le_bytes([p[10], p[11]]),
            input_rate: u32::from_le_bytes([p[12], p[13], p[14], p[15]]),
            output_gain: i16::from_le_bytes([p[16], p[17]]),
            mapping_family: family,
            streams,
            coupled,
            mapping,
        })
    }

    /// Output gain in dB.
    pub fn output_gain_db(&self) -> f32 {
        self.output_gain as f32 / 256.0
    }
}

/// `R128_TRACK_GAIN` and `R128_ALBUM_GAIN` from the comment header, in Q7.8 dB.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct R128Gains {
    pub track: Option<i16>,
    pub album: Option<i16>,
}

impl R128Gains {
    fn parse(p: &[u8]) -> Self {
        let mut gains = Self::default();
        if p.len() < 8 || &p[..8] != b"OpusTags" {
            return gains;
        }
        let mut pos = 8;
        let mut take = |n: usize| {
            let s = p.get(pos..pos + n);
            pos += n;
            s
        };
        let u32_at = |b: Option<&[u8]>| b.map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let Some(vendor) = u32_at(take(4)) else {
            return gains;
        };
        take(vendor as usize);
        let count = u32_at(take(4)).unwrap_or(0);
        for _ in 0..count {
            let Some(len) = u32_at(take(4)) else { break };
            let Some(c) = take(len as usize) else { break };
            let c = String::from_utf8_lossy(c);
            let Some((k, v)) = c.split_once('=') else {
                continue;
            };
            let v = v.trim().parse::<i16>().ok();
            if k.eq_ignore_ascii_case("R128_TRACK_GAIN") {
                gains.track = gains.track.or(v);
            } else if k.eq_ignore_ascii_case("R128_ALBUM_GAIN") {
                gains.album = gains.album.or(v);
            }
        }
        gains
    }
}

/// Owned libopus multistream decoder (also used for mapping family 0).
struct MsDecoder {
    st: NonNull<ffi::OpusMSDecoder>,
    channels: usize,
}

// SAFETY: the decoder state is plain memory owned by this value; libopus keeps no thread-local
// or global references to it.
unsafe impl Send for MsDecoder {}

fn opus_error(code: i32) -> String {
    // SAFETY: opus_strerror returns a pointer to a static NUL-terminated string.
    unsafe { CStr::from_ptr(ffi::opus_strerror(code)) }
        .to_string_lossy()
        .into_owned()
}

impl MsDecoder {
    fn new(h: &OpusHead) -> Result<Self, DecodeError> {
        let mut err = 0;
        // SAFETY: `mapping` holds `channels` entries, checked by `OpusHead::parse`.
        let st = unsafe {
            ffi::opus_multistream_decoder_create(
                RATE as i32,
                h.channels as i32,
                h.streams as i32,
                h.coupled as i32,
                h.mapping.as_ptr(),
                &mut err,
            )
        };
        match NonNull::new(st) {
            Some(st) if err == ffi::OPUS_OK => Ok(Self {
                st,
                channels: h.channels as usize,
            }),
            Some(st) => {
                // SAFETY: created above and not used afterwards.
                unsafe { ffi::opus_multistream_decoder_destroy(st.as_ptr()) };
                Err(DecodeError::Open(opus_error(err)))
            }
            None => Err(DecodeError::Open(opus_error(err))),
        }
    }

    /// Decodes one packet into `out` (at least `MAX_FRAME * channels` long); returns frames.
    fn decode(&mut self, packet: &[u8], out: &mut [f32]) -> Result<usize, String> {
        let frames = out.len() / self.channels;
        // SAFETY: `out` has room for `frames * channels` samples and `packet` is a valid slice.
        let n = unsafe {
            ffi::opus_multistream_decode_float(
                self.st.as_ptr(),
                packet.as_ptr(),
                packet.len() as i32,
                out.as_mut_ptr(),
                frames as i32,
                0,
            )
        };
        if n < 0 {
            Err(opus_error(n))
        } else {
            Ok(n as usize)
        }
    }
}

impl Drop for MsDecoder {
    fn drop(&mut self) {
        // SAFETY: `st` came from opus_multistream_decoder_create and is destroyed once.
        unsafe { ffi::opus_multistream_decoder_destroy(self.st.as_ptr()) };
    }
}

/// Samples (at 48 kHz) in a packet, 0 when it cannot be parsed.
fn packet_samples(p: &[u8]) -> u64 {
    if p.is_empty() {
        return 0;
    }
    // SAFETY: reads at most `p.len()` bytes of `p`.
    let n = unsafe { ffi::opus_packet_get_nb_samples(p.as_ptr(), p.len() as i32, RATE as i32) };
    n.max(0) as u64
}

/// Location and granule position of an Ogg page.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PageHdr {
    offset: u64,
    len: u64,
    granule: u64,
    serial: u32,
}

fn crc_table() -> &'static [u32; 256] {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut r = (i as u32) << 24;
            for _ in 0..8 {
                r = if r & 0x8000_0000 != 0 {
                    (r << 1) ^ 0x04C1_1DB7
                } else {
                    r << 1
                };
            }
            *e = r;
        }
        t
    })
}

/// Ogg page checksum, with the checksum field (bytes 22..26) counted as zero.
fn page_crc(page: &[u8]) -> u32 {
    let t = crc_table();
    page.iter().enumerate().fold(0u32, |crc, (i, &b)| {
        let b = if (22..26).contains(&i) { 0 } else { b };
        (crc << 8) ^ t[((crc >> 24) as u8 ^ b) as usize]
    })
}

/// Parses a complete, checksum-valid page at the start of `buf`.
fn parse_page(buf: &[u8], offset: u64) -> Option<PageHdr> {
    if buf.len() < 27 || &buf[..4] != b"OggS" || buf[4] != 0 {
        return None;
    }
    let header = 27 + buf[26] as usize;
    let body: usize = buf.get(27..header)?.iter().map(|&s| s as usize).sum();
    let page = buf.get(..header + body)?;
    if page_crc(page) != u32::from_le_bytes([buf[22], buf[23], buf[24], buf[25]]) {
        return None;
    }
    Some(PageHdr {
        offset,
        len: page.len() as u64,
        granule: u64::from_le_bytes(buf[6..14].try_into().ok()?),
        serial: u32::from_le_bytes([buf[14], buf[15], buf[16], buf[17]]),
    })
}

/// Valid pages in `buf` (read from file offset `base`) that start before `limit`.
fn scan_pages(buf: &[u8], base: u64, limit: usize) -> Vec<PageHdr> {
    let mut pages = Vec::new();
    let mut i = 0;
    while i < limit.min(buf.len()) {
        if buf[i] == b'O'
            && let Some(p) = parse_page(&buf[i..], base + i as u64)
        {
            i += p.len as usize;
            pages.push(p);
        } else {
            i += 1;
        }
    }
    pages
}

fn read_at<R: Read + Seek>(r: &mut R, pos: u64, len: usize) -> std::io::Result<Vec<u8>> {
    r.seek(SeekFrom::Start(pos))?;
    let mut buf = Vec::with_capacity(len);
    r.take(len as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

/// First page of `serial` with a granule position that starts in `from..to`.
fn find_page<R: Read + Seek>(
    r: &mut R,
    from: u64,
    to: u64,
    serial: u32,
) -> std::io::Result<Option<PageHdr>> {
    let mut pos = from;
    while pos < to {
        let buf = read_at(r, pos, CHUNK + MAX_PAGE)?;
        let limit = CHUNK.min((to - pos) as usize);
        if let Some(p) = scan_pages(&buf, pos, limit)
            .into_iter()
            .find(|p| p.serial == serial && p.granule != NO_GRANULE)
        {
            return Ok(Some(p));
        }
        if buf.len() <= CHUNK {
            break;
        }
        pos += CHUNK as u64;
    }
    Ok(None)
}

/// Granule position of the last page of `serial`, searching backwards from the end.
fn last_granule<R: Read + Seek>(r: &mut R, len: u64, serial: u32) -> std::io::Result<Option<u64>> {
    let mut end = len;
    while end > 0 {
        let start = end.saturating_sub(CHUNK as u64);
        let buf = read_at(r, start, (end - start) as usize + MAX_PAGE)?;
        let last = scan_pages(&buf, start, (end - start) as usize)
            .into_iter()
            .filter(|p| p.serial == serial && p.granule != NO_GRANULE)
            .map(|p| p.granule)
            .next_back();
        if last.is_some() {
            return Ok(last);
        }
        end = start;
    }
    Ok(None)
}

/// Decoder for one Ogg Opus logical stream (the first one in the file).
pub struct OpusDecoder {
    reader: PacketReader<BufReader<File>>,
    file_len: u64,
    serial: u32,
    head: OpusHead,
    r128: R128Gains,
    dec: MsDecoder,
    info: StreamInfo,
    gain: f32,
    reorder: Option<&'static [usize]>,
    /// First page after the headers.
    first_page: PageHdr,
    /// Granule position of the first decoded sample (before pre-skip).
    base_gp: u64,
    /// Granule position of the end of the stream.
    end_gp: u64,
    /// Packets with their starting granule position.
    queue: VecDeque<(Vec<u8>, u64)>,
    next_gp: Option<u64>,
    eos: bool,
    /// Samples before this granule position are decoded and dropped.
    skip_to: u64,
    pcm: Vec<f32>,
    /// (packet bytes, packet samples) for the bit-rate window.
    recent: VecDeque<(usize, u64)>,
}

impl OpusDecoder {
    pub fn open(location: &str) -> Result<Self, DecodeError> {
        let path = local_path(location).ok_or_else(|| DecodeError::Unsupported(location.into()))?;
        let file = File::open(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => DecodeError::NotFound(location.into()),
            _ => DecodeError::Open(e.to_string()),
        })?;
        let file_len = file
            .metadata()
            .map_err(|e| DecodeError::Open(e.to_string()))?
            .len();
        let mut reader = PacketReader::new(BufReader::new(file));
        let oerr = |e: ogg::OggReadError| DecodeError::Open(e.to_string());

        let first = reader
            .read_packet()
            .map_err(oerr)?
            .ok_or_else(|| DecodeError::Open("empty Ogg file".into()))?;
        let serial = first.stream_serial();
        let head = OpusHead::parse(&first.data)?;
        let tags = loop {
            let p = reader
                .read_packet()
                .map_err(oerr)?
                .ok_or_else(|| DecodeError::Open("missing OpusTags".into()))?;
            if p.stream_serial() == serial {
                break p;
            }
        };
        let r128 = R128Gains::parse(&tags.data);

        let data_start = reader
            .get_mut()
            .stream_position()
            .map_err(|e| DecodeError::Open(e.to_string()))?;
        let mut first_page = PageHdr {
            offset: data_start,
            len: 0,
            granule: 0,
            serial,
        };
        {
            let r = reader.get_mut();
            let buf = read_at(r, data_start, CHUNK + MAX_PAGE)
                .map_err(|e| DecodeError::Open(e.to_string()))?;
            if let Some(p) = scan_pages(&buf, data_start, buf.len())
                .into_iter()
                .find(|p| p.serial == serial)
            {
                first_page = p;
            }
        }
        let end_gp = last_granule(reader.get_mut(), file_len, serial)
            .map_err(|e| DecodeError::Open(e.to_string()))?
            .unwrap_or(NO_GRANULE);
        reader
            .seek_bytes(SeekFrom::Start(first_page.offset))
            .map_err(|e| DecodeError::Open(e.to_string()))?;

        let dec = MsDecoder::new(&head)?;
        let channels = head.channels as usize;
        let mut d = Self {
            reader,
            file_len,
            serial,
            gain: 10f32.powf(head.output_gain_db() / 20.0),
            reorder: (head.mapping_family == 1 && channels > 2).then(|| REORDER[channels]),
            info: StreamInfo {
                sample_rate: RATE,
                channels: channels as u16,
                bits: 0,
                length_ms: None,
                seekable: true,
                codec: "Opus".into(),
                lossless: false,
            },
            head,
            r128,
            dec,
            first_page,
            base_gp: 0,
            end_gp,
            queue: VecDeque::new(),
            next_gp: None,
            eos: false,
            skip_to: 0,
            pcm: vec![0.0; MAX_FRAME * channels],
            recent: VecDeque::new(),
        };
        d.fill_batch()?;
        d.base_gp = d.queue.front().map_or(0, |(_, gp)| *gp);
        d.skip_to = d.base_gp + d.head.pre_skip as u64;
        if d.end_gp != NO_GRANULE {
            d.info.length_ms = Some(d.total_samples() * 1000 / RATE as u64);
        }
        Ok(d)
    }

    pub fn head(&self) -> &OpusHead {
        &self.head
    }

    /// Header output gain in dB (already applied to the output).
    pub fn output_gain_db(&self) -> f32 {
        self.head.output_gain_db()
    }

    /// `R128_TRACK_GAIN` / `R128_ALBUM_GAIN` in Q7.8 dB (relative to −23 LUFS, 07 §5.3).
    pub fn r128_gains(&self) -> R128Gains {
        self.r128
    }

    /// Decoded length in samples per channel, after trimming.
    pub fn total_samples(&self) -> u64 {
        self.end_gp
            .saturating_sub(self.base_gp + self.head.pre_skip as u64)
    }

    /// Reads the packets that end on the next page and gives each its start granule position.
    /// Returns false at the end of the stream.
    fn fill_batch(&mut self) -> Result<bool, DecodeError> {
        let mut batch = Vec::new();
        let mut page_gp = None;
        let mut last = false;
        loop {
            let p = match self.reader.read_packet() {
                Ok(Some(p)) => p,
                Ok(None) => break,
                Err(e) => {
                    log::warn!("Opus: {e}");
                    break;
                }
            };
            if p.stream_serial() != self.serial {
                continue;
            }
            let ends_page = p.last_in_page() || p.last_in_stream();
            last = p.last_in_stream();
            let gp = p.absgp_page();
            batch.push(p.data);
            if ends_page {
                page_gp = Some(gp);
                break;
            }
        }
        if page_gp.is_none() || last {
            self.eos = true;
        }
        if batch.is_empty() {
            return Ok(false);
        }
        let durations: Vec<u64> = batch.iter().map(|p| packet_samples(p)).collect();
        let sum: u64 = durations.iter().sum();
        let start = match (page_gp, self.next_gp) {
            (Some(_), Some(next)) if last => next,
            (Some(gp), _) => gp.saturating_sub(sum),
            (None, Some(next)) => next,
            (None, None) => return Ok(false),
        };
        let mut gp = start;
        for (p, d) in batch.into_iter().zip(durations) {
            self.queue.push_back((p, gp));
            gp += d;
        }
        self.next_gp = Some(gp);
        Ok(true)
    }

    /// Last page of the stream whose granule position is at most `goal` (bisection).
    fn page_before(&mut self, goal: u64) -> std::io::Result<PageHdr> {
        let serial = self.serial;
        let r = self.reader.get_mut();
        let mut best = self.first_page;
        let mut lo = best.offset + best.len;
        let mut hi = self.file_len;
        while lo < hi {
            if hi - lo <= CHUNK as u64 {
                let buf = read_at(r, lo, (hi - lo) as usize + MAX_PAGE)?;
                for p in scan_pages(&buf, lo, buf.len()) {
                    if p.serial != serial || p.granule == NO_GRANULE {
                        continue;
                    }
                    if p.granule > goal {
                        break;
                    }
                    best = p;
                }
                break;
            }
            let mid = lo + (hi - lo) / 2;
            match find_page(r, mid, hi, serial)? {
                Some(p) if p.granule <= goal => {
                    best = p;
                    lo = p.offset + p.len;
                }
                _ => hi = mid,
            }
        }
        Ok(best)
    }

    fn write_frames(&self, frames: std::ops::Range<usize>, out: &mut Vec<f32>) {
        let ch = self.dec.channels;
        let pcm = &self.pcm[frames.start * ch..frames.end * ch];
        out.reserve(pcm.len());
        match self.reorder {
            Some(map) => {
                for f in pcm.chunks_exact(ch) {
                    out.extend(map.iter().map(|&c| f[c] * self.gain));
                }
            }
            None if self.gain != 1.0 => out.extend(pcm.iter().map(|s| s * self.gain)),
            None => out.extend_from_slice(pcm),
        }
    }
}

impl Decoder for OpusDecoder {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn read(&mut self, out: &mut Vec<f32>) -> Result<usize, DecodeError> {
        loop {
            let Some((packet, start)) = self.queue.pop_front() else {
                if self.eos || !self.fill_batch()? {
                    return Ok(0);
                }
                continue;
            };
            if start >= self.end_gp {
                self.queue.clear();
                self.eos = true;
                return Ok(0);
            }
            let n = match self.dec.decode(&packet, &mut self.pcm) {
                Ok(n) => n,
                Err(e) => {
                    log::warn!("Opus: {e}");
                    let n = (packet_samples(&packet) as usize).min(MAX_FRAME);
                    self.pcm[..n * self.dec.channels].fill(0.0);
                    n
                }
            };
            self.recent.push_back((packet.len(), n as u64));
            while self.recent.len() > 64 {
                self.recent.pop_front();
            }
            let s0 = start.max(self.skip_to);
            let s1 = (start + n as u64).min(self.end_gp);
            if s1 <= s0 {
                continue;
            }
            let range = (s0 - start) as usize..(s1 - start) as usize;
            let frames = range.len();
            self.write_frames(range, out);
            return Ok(frames);
        }
    }

    fn seek(&mut self, ms: u64) -> Result<u64, DecodeError> {
        let t = (ms * RATE as u64 / 1000).min(self.total_samples());
        let target = self.base_gp + self.head.pre_skip as u64 + t;
        let serr = |e: std::io::Error| DecodeError::Seek(e.to_string());
        let page = self
            .page_before(target.saturating_sub(PREROLL))
            .map_err(serr)?;
        self.reader
            .seek_bytes(SeekFrom::Start(page.offset))
            .map_err(serr)?;
        self.dec = MsDecoder::new(&self.head).map_err(|e| DecodeError::Seek(e.to_string()))?;
        self.queue.clear();
        self.recent.clear();
        self.next_gp = None;
        self.eos = false;
        self.skip_to = target;
        Ok(t * 1000 / RATE as u64)
    }

    fn kbps(&self) -> Option<u32> {
        let (bytes, samples) = self
            .recent
            .iter()
            .fold((0, 0), |(b, s), (pb, ps)| (b + pb, s + ps));
        (samples > 0).then(|| (bytes as u64 * 8 * RATE as u64 / samples / 1000) as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    const FFMPEG: &str = "/usr/bin/ffmpeg";

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nulamp-opus-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Encodes a 440 Hz sine of `secs` seconds with libopus; `None` when ffmpeg is missing.
    fn encode(dir: &Path, name: &str, secs: &str, extra: &[&str]) -> Option<PathBuf> {
        if !Path::new(FFMPEG).exists() {
            eprintln!("skipping: {FFMPEG} not found");
            return None;
        }
        let out = dir.join(name);
        let status = Command::new(FFMPEG)
            .args(["-nostdin", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg(format!(
                "sine=frequency=440:sample_rate=48000:duration={secs}"
            ))
            .args(extra)
            .args(["-c:a", "libopus", "-b:a", "96k"])
            .arg(&out)
            .status()
            .ok()?;
        if !status.success() {
            eprintln!("skipping: ffmpeg could not encode with libopus");
            return None;
        }
        Some(out)
    }

    fn decode_all(d: &mut dyn Decoder) -> Vec<f32> {
        let mut out = Vec::new();
        while d.read(&mut out).unwrap() > 0 {}
        out
    }

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|v| v * v).sum::<f32>() / s.len().max(1) as f32).sqrt()
    }

    #[test]
    fn exact_length_after_trimming() {
        let dir = tmp_dir("len");
        let Some(path) = encode(&dir, "a.opus", "2.5", &[]) else {
            return;
        };
        let mut d = OpusDecoder::open(path.to_str().unwrap()).unwrap();
        assert_eq!(d.info().sample_rate, 48_000);
        assert_eq!(d.info().channels, 1);
        assert_eq!(d.total_samples(), 120_000);
        assert_eq!(d.info().length_ms, Some(2500));
        assert!(d.head().pre_skip > 0);
        let pcm = decode_all(&mut d);
        assert_eq!(pcm.len(), 120_000);
        assert!(rms(&pcm[1000..]) > 0.05);
        assert!(d.kbps().is_some_and(|k| k > 20));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn seeks_to_the_exact_sample() {
        let dir = tmp_dir("seek");
        let Some(path) = encode(&dir, "s.opus", "6", &["-ac", "2"]) else {
            return;
        };
        let mut d = OpusDecoder::open(path.to_str().unwrap()).unwrap();
        assert_eq!(d.info().channels, 2);
        let full = decode_all(&mut d);
        assert_eq!(full.len(), 288_000 * 2);
        for ms in [4321u64, 1234, 10, 0, 5990] {
            assert_eq!(d.seek(ms).unwrap(), ms);
            let rest = decode_all(&mut d);
            let start = (ms * 48 * 2) as usize;
            assert_eq!(rest.len(), full.len() - start, "seek {ms}");
            let diff: Vec<f32> = rest
                .iter()
                .zip(&full[start..])
                .map(|(a, b)| a - b)
                .collect();
            assert!(rms(&diff) < 0.01, "seek {ms}: rms diff {}", rms(&diff));
        }
        assert_eq!(d.seek(60_000).unwrap(), 6000);
        assert!(decode_all(&mut d).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Rewrites the header gain of an Ogg Opus file and fixes the page checksum.
    fn set_output_gain(path: &Path, gain: i16) {
        let mut b = std::fs::read(path).unwrap();
        let start = 27 + b[26] as usize;
        assert_eq!(&b[start..start + 8], b"OpusHead");
        b[start + 16..start + 18].copy_from_slice(&gain.to_le_bytes());
        let page = parse_page_len(&b);
        let crc = page_crc(&b[..page]);
        b[22..26].copy_from_slice(&crc.to_le_bytes());
        std::fs::write(path, b).unwrap();
    }

    fn parse_page_len(b: &[u8]) -> usize {
        let header = 27 + b[26] as usize;
        header + b[27..header].iter().map(|&s| s as usize).sum::<usize>()
    }

    #[test]
    fn applies_output_gain() {
        let dir = tmp_dir("gain");
        let Some(path) = encode(&dir, "g.opus", "1", &[]) else {
            return;
        };
        let plain = decode_all(&mut OpusDecoder::open(path.to_str().unwrap()).unwrap());
        set_output_gain(&path, -1541);
        let mut d = OpusDecoder::open(path.to_str().unwrap()).unwrap();
        assert!((d.output_gain_db() + 6.02).abs() < 0.01);
        let quiet = decode_all(&mut d);
        assert_eq!(plain.len(), quiet.len());
        let ratio = rms(&plain) / rms(&quiet);
        assert!((ratio - 2.0).abs() < 0.01, "{ratio}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn multichannel_family_1() {
        let dir = tmp_dir("mc");
        let Some(path) = encode(
            &dir,
            "m.opus",
            "1",
            &[
                "-af",
                "pan=5.1|FC=c0|FL=0*c0|FR=0*c0|LFE=0*c0|BL=0*c0|BR=0*c0",
            ],
        ) else {
            return;
        };
        let mut d = OpusDecoder::open(path.to_str().unwrap()).unwrap();
        assert_eq!(d.head().mapping_family, 1);
        assert_eq!(d.info().channels, 6);
        let pcm = decode_all(&mut d);
        assert_eq!(pcm.len(), 48_000 * 6);
        let ch = |c: usize| rms(&pcm.iter().skip(c).step_by(6).copied().collect::<Vec<_>>());
        // WAV order: FL FR FC LFE BL BR.
        assert!(ch(2) > 0.05, "centre carries the tone");
        assert!([0, 1, 3, 4, 5].iter().all(|&c| ch(c) < 0.01));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn claims_ogg_with_opus() {
        let dir = tmp_dir("claim");
        let Some(path) = encode(&dir, "c.opus", "0.2", &[]) else {
            return;
        };
        let ogg = dir.join("c.ogg");
        std::fs::copy(&path, &ogg).unwrap();
        assert!(OpusFactory.claims(ogg.to_str().unwrap()));
        assert!(!OpusFactory.claims(path.to_str().unwrap()));
        let r = crate::Registry::with_defaults();
        assert_eq!(r.select(ogg.to_str().unwrap()).unwrap().name(), "Opus");
        assert_eq!(r.select(path.to_str().unwrap()).unwrap().name(), "Opus");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parses_headers() {
        let mut h = b"OpusHead\x01\x02\x38\x01\x80\xbb\x00\x00\x00\x01\x00".to_vec();
        let head = OpusHead::parse(&h).unwrap();
        assert_eq!(
            (head.channels, head.pre_skip, head.output_gain),
            (2, 312, 256)
        );
        assert_eq!(head.mapping, vec![0, 1]);
        h[9] = 3;
        assert!(OpusHead::parse(&h).is_err());
        let mut tags = b"OpusTags\x03\x00\x00\x00abc\x02\x00\x00\x00".to_vec();
        for c in ["R128_TRACK_GAIN=-512", "r128_album_gain=128"] {
            tags.extend_from_slice(&(c.len() as u32).to_le_bytes());
            tags.extend_from_slice(c.as_bytes());
        }
        let g = R128Gains::parse(&tags);
        assert_eq!((g.track, g.album), (Some(-512), Some(128)));
    }

    #[test]
    fn page_checksum() {
        // "OggS" header of an empty page; checksum computed independently of the field.
        let mut p = vec![0u8; 27];
        p[..4].copy_from_slice(b"OggS");
        let crc = page_crc(&p);
        p[22..26].copy_from_slice(&crc.to_le_bytes());
        assert_eq!(page_crc(&p), crc);
        assert!(parse_page(&p, 0).is_some());
        p[5] = 1;
        assert!(parse_page(&p, 0).is_none());
    }
}
