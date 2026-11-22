//! `cargo run --release --example keyframes -- film.mkv` : images clés lues dans l'index.
fn main() {
    let path = std::env::args().nth(1).expect("fichier");
    let t = std::time::Instant::now();
    let k = bioplay_core::keyframes(std::fs::File::open(&path).unwrap());
    let el = t.elapsed();
    let gaps: Vec<f64> = k.windows(2).map(|w| w[1] - w[0]).collect();
    let mean = gaps.iter().sum::<f64>() / gaps.len().max(1) as f64;
    println!("{} images clés en {:.1} ms, écart moyen {:.2} s, max {:.2} s", k.len(), el.as_secs_f64() * 1e3, mean, gaps.iter().cloned().fold(0.0, f64::max));
    println!("premières : {:?}", &k[..k.len().min(6)]);
}
