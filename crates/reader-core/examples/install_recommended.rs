//! Smoke test for the one-click setup: download + import the recommended
//! catalog into a scratch library, then run a few lookups.
//! `cargo run -p reader-core --release --example install_recommended`
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let dir = std::env::temp_dir().join(format!("yomu-dicts-{}", std::process::id()));
    let core = reader_core::Core::open(&dir).unwrap();
    for e in reader_core::download::CATALOG.iter().filter(|e| e.recommended) {
        let t = std::time::Instant::now();
        let tmp = dir.join("dl.zip");
        reader_core::download::download(e.url, &tmp, |_, _| {}).await.unwrap();
        let size = std::fs::metadata(&tmp).unwrap().len();
        let info = core
            .import_dictionary(std::io::BufReader::new(std::fs::File::open(&tmp).unwrap()), e.id, |_| {})
            .unwrap();
        println!("{:<24} {:>6.1} MB  {:>7} terms {:>7} meta  {:?}", info.title, size as f64 / 1e6, info.term_count, info.meta_count, t.elapsed());
    }
    for (text, off) in [("先生は本当に嬉しそうに笑って", 6), ("走らされてしまった", 0), ("私は首を縦に振ります。", 5)] {
        let t = std::time::Instant::now();
        let r = core.lookup(text, off, 24).unwrap();
        let e = &r.entries[0];
        println!("{text}@{off} → {}【{}】 {:?} freq={:?} pitch={:?} ({} entries, {:?})", e.expression, e.reading, e.reasons,
            e.frequencies.iter().map(|f| f.value.clone()).collect::<Vec<_>>(), e.pitches.iter().map(|p| p.positions.clone()).collect::<Vec<_>>(), r.entries.len(), t.elapsed());
    }
    let _ = std::fs::remove_dir_all(dir);
}
