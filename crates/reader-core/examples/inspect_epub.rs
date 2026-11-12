//! Import an EPUB into a scratch library and print the first sentences.
//! `cargo run -p reader-core --example inspect_epub -- book.epub [count]`
fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: inspect_epub <file.epub> [count]");
    let count: i64 = args.next().and_then(|c| c.parse().ok()).unwrap_or(25);
    let dir = std::env::temp_dir().join(format!("yomu-inspect-{}", std::process::id()));
    let core = reader_core::Core::open(&dir).unwrap();
    let t = std::time::Instant::now();
    let b = core.import_epub(std::fs::read(&path).unwrap()).unwrap();
    println!("{} / {} — {} sentences (import {:?})", b.title, b.author, b.sentence_count, t.elapsed());
    for c in core.book_detail(b.id).unwrap().chapters {
        println!("  chapter {} “{}” @ {}", c.idx, c.title, c.start);
    }
    let t = std::time::Instant::now();
    let s = core.sentences(b.id, 0, count).unwrap();
    println!("analysed {} sentences in {:?}", s.len(), t.elapsed());
    for s in s {
        let furi: String = s.tokens.iter().flat_map(|t| t.segs.iter()).map(|g| match &g.ruby {
            Some(r) => format!("{}[{}]", g.text, r),
            None => g.text.clone(),
        }).collect();
        println!("{:>4} {}", s.idx, furi);
    }
    let _ = std::fs::remove_dir_all(dir);
}
