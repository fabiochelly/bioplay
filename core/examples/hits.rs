//! Part des sauts qui tomberaient près d'une image clé (tolérances avant / après).
fn main() {
    let path = std::env::args().nth(1).expect("fichier");
    let k = bioplay_core::keyframes(std::fs::File::open(&path).unwrap());
    let tracks = bioplay_core::index(std::fs::File::open(&path).unwrap()).unwrap_or_default();
    let near = |t: f64, before: f64, after: f64| k.iter().any(|&x| x >= t - before && x <= t + after);
    if let Some(tr) = tracks.iter().find(|t| !t.image && !t.cues.is_empty()) {
        let starts: Vec<f64> = tr.cues.iter().map(|c| c.start as f64 / 1000.0).collect();
        for (b, a) in [(0.5, 0.1), (1.0, 0.1), (1.5, 0.15), (2.0, 0.15)] {
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
