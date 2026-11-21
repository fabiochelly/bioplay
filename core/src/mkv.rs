//! Matroska / WebM : parcourt les clusters en ne lisant que l'en-tête de chaque bloc. Les
//! blocs audio et vidéo sont sautés sans être lus ; seuls les blocs de sous-titres le sont.

use std::collections::HashMap;
use std::io;

use crate::model::{Cue, Track};
use crate::reader::Reader;
use crate::text;

const EBML: u32 = 0x1A45DFA3;
const SEGMENT: u32 = 0x18538067;
const INFO: u32 = 0x1549A966;
const TIMESTAMP_SCALE: u32 = 0x2AD7B1;
const TRACKS: u32 = 0x1654AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const TRACK_TYPE: u32 = 0x83;
const CODEC_ID: u32 = 0x86;
const LANGUAGE: u32 = 0x22B59C;
const LANGUAGE_BCP47: u32 = 0x22B59D;
const NAME: u32 = 0x536E;
const DEFAULT_DURATION: u32 = 0x23E383;
const CONTENT_ENCODINGS: u32 = 0x6D80;
const CONTENT_ENCODING: u32 = 0x6240;
const CONTENT_ENCODING_TYPE: u32 = 0x5033;
const CONTENT_COMPRESSION: u32 = 0x5034;
const CONTENT_COMP_ALGO: u32 = 0x4254;
const CONTENT_COMP_SETTINGS: u32 = 0x4255;
const CLUSTER: u32 = 0x1F43B675;
const CLUSTER_TIMESTAMP: u32 = 0xE7;
const SIMPLE_BLOCK: u32 = 0xA3;
const BLOCK_GROUP: u32 = 0xA0;
const BLOCK: u32 = 0xA1;
const BLOCK_DURATION: u32 = 0x9B;

const LEVEL1: [u32; 9] = [
    0x114D9B74, INFO, TRACKS, CLUSTER, 0x1C53BB6B, 0x1941A469, 0x1043A770, 0x1254C367, 0xEC,
];

struct Header {
    id: u32,
    size: Option<u64>,
}

fn vint(r: &mut Reader, keep_marker: bool) -> io::Result<(u64, u32)> {
    let first = r.u8()?;
    if first == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "vint invalide"));
    }
    let len = first.leading_zeros() + 1;
    let mut v = if keep_marker { first as u64 } else { (first as u64) & (0xFF >> len) };
    for _ in 1..len {
        v = (v << 8) | r.u8()? as u64;
    }
    Ok((v, len))
}

fn header(r: &mut Reader) -> io::Result<Header> {
    let (id, _) = vint(r, true)?;
    let (size, len) = vint(r, false)?;
    let unknown = size == (1u64 << (7 * len)) - 1;
    Ok(Header { id: id as u32, size: if unknown { None } else { Some(size) } })
}

fn uint(r: &mut Reader, size: u64) -> io::Result<u64> {
    let mut v = 0;
    for _ in 0..size.min(8) {
        v = (v << 8) | r.u8()? as u64;
    }
    r.skip(size.saturating_sub(8));
    Ok(v)
}

fn string(r: &mut Reader, size: u64) -> io::Result<String> {
    let b = r.bytes(size.min(4096) as usize)?;
    r.skip(size.saturating_sub(4096));
    Ok(String::from_utf8_lossy(&b).trim_end_matches('\0').to_owned())
}

/// Itère sur les enfants d'un élément de taille connue.
fn children(r: &mut Reader, size: Option<u64>, mut f: impl FnMut(&mut Reader, Header) -> io::Result<()>) -> io::Result<()> {
    let end = r.pos + size.unwrap_or(0);
    while r.pos < end {
        let h = header(r)?;
        let start = r.pos;
        let Some(sz) = h.size else { break };
        f(r, h)?;
        r.pos = start + sz;
    }
    r.pos = end;
    Ok(())
}

enum Compression {
    None,
    Zlib,
    Strip(Vec<u8>),
}

struct Sub {
    track: Track,
    default_duration: i64,
    compression: Compression,
    /// PGS : un affichage est ouvert, en attente de l'ordre d'effacement.
    open: bool,
}

fn parse_track(r: &mut Reader, size: Option<u64>) -> io::Result<Option<Sub>> {
    let (mut number, mut kind, mut codec, mut lang, mut bcp, mut name, mut dur) = (0, 0, String::new(), String::from("und"), String::new(), String::new(), 0);
    let mut compression = Compression::None;
    children(r, size, |r, h| {
        let s = h.size.unwrap_or(0);
        match h.id {
            TRACK_NUMBER => number = uint(r, s)?,
            TRACK_TYPE => kind = uint(r, s)?,
            CODEC_ID => codec = string(r, s)?,
            LANGUAGE => lang = string(r, s)?,
            LANGUAGE_BCP47 => bcp = string(r, s)?,
            NAME => name = string(r, s)?,
            DEFAULT_DURATION => dur = uint(r, s)?,
            CONTENT_ENCODINGS => children(r, h.size, |r, h| {
                if h.id != CONTENT_ENCODING {
                    return Ok(());
                }
                let (mut etype, mut algo, mut settings) = (0, 0, Vec::new());
                children(r, h.size, |r, h| {
                    match h.id {
                        CONTENT_ENCODING_TYPE => etype = uint(r, h.size.unwrap_or(0))?,
                        CONTENT_COMPRESSION => children(r, h.size, |r, h| {
                            let s = h.size.unwrap_or(0);
                            match h.id {
                                CONTENT_COMP_ALGO => algo = uint(r, s)?,
                                CONTENT_COMP_SETTINGS => settings = r.bytes(s.min(1 << 16) as usize)?,
                                _ => {}
                            }
                            Ok(())
                        })?,
                        _ => {}
                    }
                    Ok(())
                })?;
                if etype == 0 {
                    compression = match algo {
                        0 => Compression::Zlib,
                        3 => Compression::Strip(settings),
                        _ => Compression::None,
                    };
                }
                Ok(())
            })?,
            _ => {}
        }
        Ok(())
    })?;
    if kind != 0x11 {
        return Ok(None);
    }
    let image = matches!(codec.as_str(), "S_HDMV/PGS" | "S_VOBSUB" | "S_DVBSUB" | "S_IMAGE/BMP");
    Ok(Some(Sub {
        track: Track { number, codec, lang: if bcp.is_empty() { lang } else { bcp }, name, image, cues: Vec::new() },
        default_duration: dur as i64,
        compression,
        open: false,
    }))
}

struct State {
    subs: HashMap<u64, Sub>,
    scale: i64,
}

impl State {
    fn ms(&self, ticks: i64) -> i64 {
        ticks * self.scale / 1_000_000
    }
}

/// Lit l'en-tête d'un bloc ; s'il appartient à un sous-titre, lit sa charge.
/// Renvoie (piste, temps en ticks, données).
fn block(r: &mut Reader, size: u64, st: &State) -> io::Result<Option<(u64, i64, Vec<u8>)>> {
    let start = r.pos;
    let end = start + size;
    let (track, _) = vint(r, false)?;
    if !st.subs.contains_key(&track) {
        r.pos = end;
        return Ok(None);
    }
    let hi = r.u8()?;
    let lo = r.u8()?;
    let rel = i16::from_be_bytes([hi, lo]) as i64;
    let flags = r.u8()?;
    if (flags >> 1) & 3 != 0 {
        r.pos = end; // laçage : jamais utilisé pour des sous-titres
        return Ok(None);
    }
    let n = end.saturating_sub(r.pos).min(4 << 20) as usize;
    let data = r.bytes(n)?;
    r.pos = end;
    Ok(Some((track, rel, data)))
}

fn decompress(c: &Compression, data: Vec<u8>) -> Vec<u8> {
    match c {
        Compression::None => data,
        Compression::Zlib => miniz_oxide::inflate::decompress_to_vec_zlib(&data).unwrap_or_default(),
        Compression::Strip(prefix) => [prefix.as_slice(), &data].concat(),
    }
}

/// Composition PGS : nombre d'objets affichés (0 = l'écran s'efface).
fn pgs_objects(data: &[u8]) -> Option<u8> {
    let mut i = 0;
    while i + 3 <= data.len() {
        let kind = data[i];
        let len = u16::from_be_bytes([data[i + 1], data[i + 2]]) as usize;
        if kind == 0x16 && i + 3 + 11 <= data.len() {
            return Some(data[i + 3 + 10]);
        }
        i += 3 + len;
    }
    None
}

fn add(st: &mut State, track: u64, at: i64, duration: Option<i64>, data: Vec<u8>) {
    let start = st.ms(at);
    let dur_ms = duration.map(|d| st.ms(d));
    let Some(sub) = st.subs.get_mut(&track) else { return };
    let data = decompress(&sub.compression, data);
    let codec = sub.track.codec.as_str();
    if codec == "S_HDMV/PGS" {
        let Some(objects) = pgs_objects(&data) else { return };
        if sub.open {
            if let Some(last) = sub.track.cues.last_mut() {
                last.end = start;
            }
            sub.open = false;
        }
        if objects > 0 {
            sub.track.cues.push(Cue { start, end: 0, text: "▣".into() });
            sub.open = true;
        }
        return;
    }
    let end = match dur_ms {
        Some(d) if d > 0 => start + d,
        _ if sub.default_duration > 0 => start + sub.default_duration / 1_000_000,
        _ => 0,
    };
    let text = if sub.track.image {
        "▣".to_owned()
    } else {
        let s = String::from_utf8_lossy(&data);
        match codec {
            "S_TEXT/ASS" | "S_TEXT/SSA" | "S_ASS" | "S_SSA" => text::mkv_ass_text(&s),
            _ => text::strip_html(&s),
        }
    };
    sub.track.cues.push(Cue { start, end, text });
}

fn cluster(r: &mut Reader, size: Option<u64>, seg_end: u64, st: &mut State) -> io::Result<()> {
    let end = size.map(|s| r.pos + s);
    let mut ts: i64 = 0;
    loop {
        let limit = end.unwrap_or(seg_end);
        if r.pos >= limit || r.pos >= r.size {
            break;
        }
        let at = r.pos;
        let h = header(r)?;
        if end.is_none() && LEVEL1.contains(&h.id) {
            r.pos = at; // fin d'un cluster de taille inconnue
            break;
        }
        let Some(sz) = h.size else { break };
        let next = r.pos + sz;
        match h.id {
            CLUSTER_TIMESTAMP => ts = uint(r, sz)? as i64,
            SIMPLE_BLOCK => {
                if let Some((t, rel, data)) = block(r, sz, st)? {
                    add(st, t, ts + rel, None, data);
                }
            }
            BLOCK_GROUP => {
                let mut found = None;
                let mut duration = None;
                children(r, Some(sz), |r, h| {
                    match h.id {
                        BLOCK => found = block(r, h.size.unwrap_or(0), st)?,
                        BLOCK_DURATION => duration = Some(uint(r, h.size.unwrap_or(0))? as i64),
                        _ => {}
                    }
                    Ok(())
                })?;
                if let Some((t, rel, data)) = found {
                    add(st, t, ts + rel, duration, data);
                }
            }
            _ => {}
        }
        r.pos = next;
    }
    Ok(())
}

pub fn index(r: &mut Reader) -> io::Result<Vec<Track>> {
    let h = header(r)?;
    if h.id != EBML {
        return Ok(Vec::new());
    }
    r.skip(h.size.unwrap_or(0));
    let seg = loop {
        let h = header(r)?;
        if h.id == SEGMENT {
            break h;
        }
        r.skip(h.size.unwrap_or(0));
    };
    let seg_end = seg.size.map(|s| r.pos + s).unwrap_or(r.size).min(r.size);
    let mut st = State { subs: HashMap::new(), scale: 1_000_000 };
    let mut tracks_seen = false;
    while r.pos < seg_end {
        let Ok(h) = header(r) else { break };
        let start = r.pos;
        match h.id {
            INFO => children(r, h.size, |r, h| {
                if h.id == TIMESTAMP_SCALE {
                    st.scale = uint(r, h.size.unwrap_or(0))? as i64;
                }
                Ok(())
            })?,
            TRACKS => {
                children(r, h.size, |r, h| {
                    if h.id == TRACK_ENTRY {
                        if let Some(s) = parse_track(r, h.size)? {
                            st.subs.insert(s.track.number, s);
                        }
                    }
                    Ok(())
                })?;
                tracks_seen = true;
                if st.subs.is_empty() {
                    break; // aucun sous-titre : inutile de parcourir le film
                }
            }
            CLUSTER if tracks_seen => {
                // Une erreur de lecture au milieu (fichier tronqué) garde ce qui a été lu.
                if cluster(r, h.size, seg_end, &mut st).is_err() {
                    break;
                }
                continue;
            }
            _ => {}
        }
        match h.size {
            Some(s) => r.pos = start + s,
            None if h.id == CLUSTER => {}
            None => break,
        }
    }
    let mut tracks: Vec<Track> = st.subs.into_values().map(|s| s.track).collect();
    tracks.sort_by_key(|t| t.number);
    Ok(tracks)
}
