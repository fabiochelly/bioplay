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

impl Track {
    /// Dernière ligne commencée à `ms`.
    pub fn at(&self, ms: i64) -> Option<usize> {
        let n = self.cues.partition_point(|c| c.start <= ms);
        n.checked_sub(1)
    }

    /// Ligne visée par `steps` crans depuis `ms` : +1 la suivante, -1 la précédente. Une ligne
    /// commencée depuis plus de 0,8 s est d'abord reprise à son début (comme un lecteur audio).
    pub fn step(&self, ms: i64, steps: i32) -> Option<usize> {
        if self.cues.is_empty() || steps == 0 {
            return None;
        }
        let base = self.at(ms + 40).map(|i| i as i64).unwrap_or(-1);
        let target = if steps > 0 {
            base + steps as i64
        } else {
            let current = if base >= 0 && ms - self.cues[base as usize].start < 800 { base - 1 } else { base };
            current + steps as i64 + 1
        };
        Some(target.clamp(0, self.cues.len() as i64 - 1) as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> Track {
        let cues = (0..5).map(|i| Cue { start: i * 1000 + 1000, end: i * 1000 + 1800, text: format!("{i}") }).collect();
        Track { number: 1, codec: String::new(), lang: String::new(), name: String::new(), image: false, cues }
    }

    #[test]
    fn steps() {
        let t = track(); // lignes à 1, 2, 3, 4, 5 s
        assert_eq!(t.step(2500, 1), Some(2)); // pendant la ligne 1 → ligne 2
        assert_eq!(t.step(2900, -1), Some(1)); // reprend la ligne en cours
        assert_eq!(t.step(2500, -1), Some(0)); // juste commencée → la précédente
        assert_eq!(t.step(500, 1), Some(0));
        assert_eq!(t.step(4500, 9), Some(4));
    }
}
