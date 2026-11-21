//! `cargo run --release --example dump -- film.mkv` : affiche l'index et sa durée de calcul.
fn main() {
    let path = std::env::args().nth(1).expect("chemin");
    let t = std::time::Instant::now();
    let mut tracks = bioplay_core::index(std::fs::File::open(&path).unwrap()).unwrap();
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    for tr in &tracks {
        println!("piste {} {} [{}] «{}» : {} lignes", tr.number, tr.codec, tr.lang, tr.name, tr.cues.len());
        for c in tr.cues.iter().take(4) {
            println!("   {:>8} → {:>8}  {}", c.start, c.end, c.text.replace('\n', " / "));
        }
    }
    let enc = bioplay_core::model::encode(&mut tracks);
    println!("{} octets encodés, {:.1} ms", enc.len(), ms);
}
