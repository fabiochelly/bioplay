//! Fichiers de sous-titres séparés (SRT, WebVTT, ASS/SSA) et nettoyage du texte des lignes.

use crate::model::{Cue, Track};

/// UTF-8 ou UTF-16 avec BOM, UTF-8 sans BOM, sinon Windows-1252 (fréquent pour les
/// vieux .srt français).
pub fn decode(bytes: &[u8]) -> String {
    if let Some(b) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(b).into_owned();
    }
    if bytes.len() >= 2 && (bytes[..2] == [0xFF, 0xFE] || bytes[..2] == [0xFE, 0xFF]) {
        let le = bytes[0] == 0xFF;
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|p| if le { u16::from_le_bytes([p[0], p[1]]) } else { u16::from_be_bytes([p[0], p[1]]) })
            .collect();
        return String::from_utf16_lossy(&units);
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => bytes.iter().map(|&b| cp1252(b)).collect(),
    }
}

fn cp1252(b: u8) -> char {
    const HIGH: [u16; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160,
        0x2039, 0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013,
        0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
    ];
    match b {
        0x80..=0x9F => char::from_u32(HIGH[(b - 0x80) as usize] as u32).unwrap_or('?'),
        _ => b as char,
    }
}

/// `01:02:03,456`, `02:03.456`, `1:02:03.45` (ASS, centièmes) → millisecondes.
pub fn time(s: &str) -> Option<i64> {
    let s = s.trim();
    let mut parts: Vec<&str> = s.split(':').collect();
    let last = parts.pop()?;
    let (sec, frac) = match last.find(['.', ',']) {
        Some(i) => (&last[..i], &last[i + 1..]),
        None => (last, ""),
    };
    let mut ms: i64 = sec.trim().parse::<i64>().ok()? * 1000;
    if !frac.is_empty() {
        let digits: String = frac.chars().take_while(|c| c.is_ascii_digit()).take(3).collect();
        let n: i64 = digits.parse().ok()?;
        ms += n * 10i64.pow(3 - digits.len() as u32);
    }
    let mut mult = 60_000;
    for p in parts.iter().rev() {
        ms += p.trim().parse::<i64>().ok()? * mult;
        mult *= 60;
    }
    Some(ms)
}

/// Retire les balises HTML (`<i>`, `<font …>`, `<c.yellow>`) et les entités courantes.
pub fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ")
        .trim().to_owned()
}

/// Texte d'une ligne ASS : sans `{\balises}`, sans dessins vectoriels (`{\p1}…{\p0}`).
pub fn strip_ass(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut drawing = false;
    let mut rest = s;
    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix('{') {
            let end = stripped.find('}').map(|i| i + 1).unwrap_or(stripped.len());
            let tag = &stripped[..end.min(stripped.len())];
            let mut t = tag;
            while let Some(i) = t.find("\\p") {
                let d = t[i + 2..].chars().next();
                if let Some(d) = d.filter(|d| d.is_ascii_digit()) {
                    drawing = d != '0';
                }
                t = &t[i + 2..];
            }
            rest = &stripped[end.min(stripped.len())..];
            continue;
        }
        let c = rest.chars().next().unwrap();
        rest = &rest[c.len_utf8()..];
        if drawing {
            continue;
        }
        if c == '\\' {
            match rest.chars().next() {
                Some('N') | Some('n') => {
                    out.push('\n');
                    rest = &rest[1..];
                    continue;
                }
                Some('h') => {
                    out.push(' ');
                    rest = &rest[1..];
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
    }
    out.trim().to_owned()
}

/// Champ `Text` d'un bloc ASS dans un MKV : après les 8 premières virgules
/// (ReadOrder, Layer, Style, Name, MarginL, MarginR, MarginV, Effect).
pub fn mkv_ass_text(s: &str) -> String {
    let mut rest = s;
    for _ in 0..8 {
        match rest.find(',') {
            Some(i) => rest = &rest[i + 1..],
            None => return strip_ass(s),
        }
    }
    strip_ass(rest)
}

pub fn index(bytes: &[u8]) -> Vec<Track> {
    let text = decode(bytes);
    let (codec, cues) = if text.contains("[Script Info]") || text.contains("[Events]") {
        ("ass", ass(&text))
    } else if text.trim_start().starts_with("WEBVTT") {
        ("webvtt", timed_blocks(&text))
    } else {
        ("subrip", timed_blocks(&text))
    };
    vec![Track { number: 0, codec: codec.into(), lang: String::new(), name: String::new(), image: false, cues }]
}

/// SRT et WebVTT : une ligne `début --> fin`, puis le texte jusqu'à la ligne vide.
fn timed_blocks(text: &str) -> Vec<Cue> {
    let mut cues = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(i) = line.find("-->") else { continue };
        let start = time(&line[..i]);
        let end = line[i + 3..].split_whitespace().next().and_then(time);
        let (Some(start), Some(end)) = (start, end) else { continue };
        let mut body = Vec::new();
        while let Some(l) = lines.peek() {
            if l.trim().is_empty() {
                break;
            }
            body.push(strip_html(lines.next().unwrap()));
        }
        cues.push(Cue { start, end, text: body.join("\n") });
    }
    cues
}

fn ass(text: &str) -> Vec<Cue> {
    let mut cues = Vec::new();
    let mut fields: Vec<String> = ["layer", "start", "end", "style", "name", "marginl", "marginr", "marginv", "effect", "text"]
        .iter().map(|s| s.to_string()).collect();
    let mut events = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            events = l.eq_ignore_ascii_case("[events]");
            continue;
        }
        if !events {
            continue;
        }
        if let Some(f) = l.strip_prefix("Format:") {
            fields = f.split(',').map(|s| s.trim().to_ascii_lowercase()).collect();
        } else if let Some(d) = l.strip_prefix("Dialogue:") {
            let parts: Vec<&str> = d.splitn(fields.len(), ',').collect();
            let get = |name: &str| fields.iter().position(|f| f == name).and_then(|i| parts.get(i).copied());
            let (Some(s), Some(e), Some(t)) = (get("start").and_then(time), get("end").and_then(time), get("text")) else { continue };
            cues.push(Cue { start: s, end: e, text: strip_ass(t) });
        }
    }
    cues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(time("01:02:03,456"), Some(3_723_456));
        assert_eq!(time("02:03.4"), Some(123_400));
        assert_eq!(time("0:00:01.50"), Some(1_500));
    }

    #[test]
    fn ass_tags() {
        assert_eq!(strip_ass(r"{\an8\i1}Bonjour\Nle monde"), "Bonjour\nle monde");
        assert_eq!(strip_ass(r"{\p1}m 0 0 l 10 10{\p0}Texte"), "Texte");
        assert_eq!(mkv_ass_text(r"3,0,Default,,0,0,0,,Salut, toi"), "Salut, toi");
    }

    #[test]
    fn srt() {
        let t = index(b"1\n00:00:01,000 --> 00:00:02,500\n<i>Un</i>\ndeux\n\n2\n00:00:03,000 --> 00:00:04,000\nTrois\n");
        assert_eq!(t[0].cues.len(), 2);
        assert_eq!(t[0].cues[0].text, "Un\ndeux");
        assert_eq!(t[0].cues[1].start, 3000);
    }

    #[test]
    fn latin1() {
        assert_eq!(decode(b"\xe9t\xe9 \x9c"), "été œ");
    }
}
