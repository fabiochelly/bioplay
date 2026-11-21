//! Ce que l'index rend à l'app, et son format d'échange : des lignes à tabulations, lues
//! en une passe côté Kotlin, sans dépendance de sérialisation.
//!
//! ```text
//! T <numéro> <codec> <langue> <nom> <image 0|1>
//! C <début ms> <fin ms> <texte, retours à la ligne en \x1f>
//! ```

pub struct Cue {
    pub start: i64,
    pub end: i64,
    pub text: String,
}

pub struct Track {
    pub number: u64,
    pub codec: String,
    pub lang: String,
    pub name: String,
    pub image: bool,
    pub cues: Vec<Cue>,
}

fn clean(s: &str) -> String {
    s.chars()
        .filter(|&c| c != '\r')
        .map(|c| match c {
            '\t' => ' ',
            '\n' => '\u{1f}',
            c => c,
        })
        .collect()
}

pub fn encode(tracks: &mut [Track]) -> String {
    let mut out = String::new();
    for t in tracks.iter_mut() {
        finish(&mut t.cues);
        out += &format!(
            "T\t{}\t{}\t{}\t{}\t{}\n",
            t.number,
            clean(&t.codec),
            clean(&t.lang),
            clean(&t.name),
            t.image as u8
        );
        for c in &t.cues {
            out += &format!("C\t{}\t{}\t{}\n", c.start, c.end, clean(&c.text));
        }
    }
    out
}

/// Trie, retire les doublons (les ASS empilent souvent plusieurs couches identiques)
/// et donne une fin aux sous-titres qui n'en ont pas.
fn finish(cues: &mut Vec<Cue>) {
    cues.retain(|c| !c.text.trim().is_empty());
    cues.sort_by_key(|c| (c.start, c.end));
    cues.dedup_by(|b, a| a.start == b.start && a.text == b.text);
    for i in 0..cues.len() {
        if cues[i].end <= cues[i].start {
            let next = cues.get(i + 1).map(|n| n.start).unwrap_or(i64::MAX);
            cues[i].end = (cues[i].start + 4000).min(next);
        }
    }
}
