//! Generates JSON fixtures for the README screenshots (scripts/screenshots).
//! Uses only public-domain text (夏目漱石『吾輩は猫である』, 1905) and the real
//! core: EPUB import, tokenizer/furigana and the recommended dictionaries.
//!
//! `cargo run -p reader-core --release --example screenshot_fixtures -- out.json`
use std::io::Write;

use serde_json::{json, Value};

const PARAGRAPHS: &[&str] = &[
    "　吾輩は猫である。名前はまだ無い。",
    "　どこで生れたかとんと見当がつかぬ。何でも薄暗いじめじめした所でニャーニャー泣いていた事だけは記憶している。吾輩はここで始めて人間というものを見た。しかもあとで聞くとそれは書生という人間中で一番<ruby>獰悪<rt>どうあく</rt></ruby>な種族であったそうだ。",
    "　この書生というのは時々我々を捕えて煮て食うという話である。しかしその当時は何という考もなかったから別段恐しいとも思わなかった。",
];

fn epub() -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let o = zip::write::SimpleFileOptions::default();
        let mut put = |name: &str, body: &str| {
            z.start_file(name, o).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        };
        put("mimetype", "application/epub+zip");
        put("META-INF/container.xml", r#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#);
        put("content.opf", r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>吾輩は猫である</dc:title><dc:creator>夏目漱石</dc:creator><dc:language>ja</dc:language></metadata><manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/></manifest><spine toc="ncx"><itemref idref="c1"/></spine></package>"#);
        put("toc.ncx", r#"<?xml version="1.0"?><ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap><navPoint id="a"><navLabel><text>一</text></navLabel><content src="c1.xhtml"/></navPoint></navMap></ncx>"#);
        let body: String = PARAGRAPHS.iter().map(|p| format!("<p>{p}</p>")).collect();
        put("c1.xhtml", &format!("<html><body>{body}</body></html>"));
        z.finish().unwrap();
    }
    buf.into_inner()
}

/// Hand-written example of the LLM output
/// shape, for the sentence 「何でも薄暗い…記憶している。」.
fn translation() -> Value {
    json!({
        "translation": "All I remember is crying \"meow, meow\" somewhere dim and damp.",
        "literal": "As far as I know, only the fact that I was crying 'nyaa nyaa' in a dim, damp place is in my memory.",
        "breakdown": [
            {"text": "何でも", "reading": "なんでも", "meaning": "as far as I can tell; apparently", "role": "adverb (hearsay / vague recollection)"},
            {"text": "薄暗い", "reading": "うすぐらい", "meaning": "dim, gloomy", "role": "i-adjective, modifies 所"},
            {"text": "じめじめした", "reading": "", "meaning": "damp, clammy", "role": "onomatopoeic adverb + した (past of する), modifies 所"},
            {"text": "所", "reading": "ところ", "meaning": "place", "role": "noun"},
            {"text": "で", "reading": "", "meaning": "at, in", "role": "particle marking where an action happens"},
            {"text": "ニャーニャー", "reading": "", "meaning": "meow meow", "role": "onomatopoeia (cat's cry)"},
            {"text": "泣いていた", "reading": "ないていた", "meaning": "was crying", "role": "verb 泣く, 〜ていた (past progressive)"},
            {"text": "事", "reading": "こと", "meaning": "the fact (that…)", "role": "nominalizer turning the clause into a noun"},
            {"text": "だけは", "reading": "", "meaning": "only (that much)", "role": "particle だけ + contrastive は"},
            {"text": "記憶している", "reading": "きおくしている", "meaning": "remember", "role": "noun 記憶 + している (ongoing state)"}
        ],
        "grammar": [
            {"pattern": "〜ていた", "explanation": "Past progressive: an action that was going on at that time (\"was crying\")."},
            {"pattern": "〜事だけは記憶している", "explanation": "\"The only thing I remember is that…\"; だけは stresses that everything else is forgotten."},
            {"pattern": "何でも", "explanation": "At the start of a sentence it signals a vague or second-hand recollection: \"as far as I know\"."}
        ],
        "notes": "The narrator is a cat — this is the famous opening of Natsume Sōseki's 'I Am a Cat' (1905).",
        "model": "deepseek-flash",
        "cached": true
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let out = std::env::args().nth(1).expect("usage: screenshot_fixtures <out.json>");
    let dir = std::env::temp_dir().join(format!("yomu-shots-{}", std::process::id()));
    let core = reader_core::Core::open(&dir).unwrap();
    let book = core.import_epub(epub()).unwrap();
    let detail = core.book_detail(book.id).unwrap();
    let sentences = core.sentences(book.id, 0, 64).unwrap();

    for e in reader_core::download::CATALOG.iter().filter(|e| e.recommended) {
        let tmp = dir.join("dl.zip");
        reader_core::download::download(e.url, &tmp, |_, _| {}).await.unwrap();
        core.import_dictionary(std::io::BufReader::new(std::fs::File::open(&tmp).unwrap()), e.id, |_| {}).unwrap();
    }

    // Lookups for every word start in every sentence, so the capture script
    // can click anything.
    let mut lookups = serde_json::Map::new();
    for s in &sentences {
        for t in s.tokens.iter().filter(|t| !t.punct) {
            for off in t.start..t.end {
                let r = core.lookup(&s.text, off, 12).unwrap();
                lookups.insert(format!("{}|{}", s.text, off), serde_json::to_value(r).unwrap());
            }
        }
    }
    let tr_idx = sentences.iter().find(|s| s.text.starts_with("何でも")).map(|s| s.idx).unwrap();

    let fixtures = json!({
        "book": book,
        "detail": detail,
        "sentences": sentences,
        "lookups": lookups,
        "translations": { tr_idx.to_string(): translation() },
        "dictionaries": core.list_dictionaries().unwrap(),
        "catalog": reader_core::download::CATALOG,
    });
    std::fs::write(&out, serde_json::to_string(&fixtures).unwrap()).unwrap();
    println!("wrote {out}: {} sentences, {} lookups, translation at #{tr_idx}", fixtures["sentences"].as_array().unwrap().len(), fixtures["lookups"].as_object().unwrap().len());
    let _ = std::fs::remove_dir_all(dir);
}
