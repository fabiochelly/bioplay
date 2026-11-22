//! Part des sauts qui tomberaient près d'une image clé (tolérances avant / après).
fn main() {
    let path = std::env::args().nth(1).expect("fichier");
    let k = bioplay_core::keyframes(std::fs::File::open(&path).unwrap());
    let tracks = bioplay_core::index(std::fs::File::open(&path).unwrap()).unwrap_or_default();
    let near = |t: f64, before: f64, after: f64| k.iter().any(|&x| x >= t - before && x <= t + after);
    if let Some(tr) = tracks.iter().find(|t| !t.image && !t.cues.is_empty()) {
        let starts: Vec<f64> = tr.cues.iter().map(|c| c.start as f64 / 1000.0).collect();
        // Dans le silence avant la réplique (après la fin de la précédente), au plus `b` s avant.
        for (b, a) in [(5.0, 0.15), (8.0, 0.15), (5.0, 0.3)] {
            let mut n = 0;
            for (i, c) in tr.cues.iter().enumerate() {
                let t = c.start as f64 / 1000.0;
                let prev_end = if i > 0 { tr.cues[i - 1].end as f64 / 1000.0 } else { 0.0 };
                let lo = (t - b).max(prev_end.min(t));
                if near(t, t - lo, a) { n += 1; }
            }
            println!("sous-titres : image clé dans le silence avant (≤ {b} s) ou +{a} s → {:.0} %", 100.0 * n as f64 / tr.cues.len() as f64);
        }
        for (b, a) in [(2.0, 0.15), (2.0, 0.5), (3.0, 0.15), (3.0, 0.5), (3.0, 1.0), (4.0, 0.5), (5.0, 0.5), (5.0, 1.0)] {
            let n = starts.iter().filter(|&&t| near(t, b, a)).count();
            println!("sous-titres ({} lignes) : image clé entre −{b} s et +{a} s → {:.0} %", starts.len(), 100.0 * n as f64 / starts.len() as f64);
        }
    }
    let end = *k.last().unwrap_or(&0.0);
    for tol in [1.0, 1.5, 2.0] {
        let (mut n, mut tot) = (0, 0);
        let mut p = 30.0;
        while p + 10.0 < end {
            tot += 1;
            if near(p + 10.0, tol, tol) { n += 1; }
            p += 0.73;
        }
        println!("sauts de 10 s : image clé à ±{tol} s → {:.0} %", 100.0 * n as f64 / tot as f64);
    }
}
