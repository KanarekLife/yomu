//! Paragraph → sentence segmentation (ADR-0006).

use serde::{Deserialize, Serialize};

/// Reading annotation that came from the book itself (`<ruby>` markup).
/// `start`/`end` are char indices into the owning text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RubySpan {
    pub start: usize,
    pub end: usize,
    pub reading: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Paragraph {
    pub text: String,
    pub ruby: Vec<RubySpan>,
    /// Came from an `<h1>`–`<h6>` element.
    pub heading: bool,
    /// An illustration (path inside the EPUB) instead of text.
    pub image: Option<String>,
}

/// Whether a paragraph looks like an in-text chapter heading: an HTML
/// heading, or a short line such as `１`, `第三章`, `プロローグ`.
pub fn is_heading(p: &Paragraph) -> bool {
    let t: String = p.text.chars().filter(|c| !(c.is_whitespace() || *c == '\u{3000}')).collect();
    let n = t.chars().count();
    if n == 0 || n > 24 {
        return false;
    }
    if p.heading {
        return true;
    }
    const NAMED: &[&str] = &["プロローグ", "エピローグ", "序章", "終章", "幕間", "間章", "あとがき", "序", "終幕", "Prologue", "Epilogue"];
    if NAMED.iter().any(|w| t == *w || t.starts_with(&format!("{w}　")) || t.starts_with(&format!("{w} "))) {
        return true;
    }
    let numeral = |c: char| c.is_ascii_digit() || ('０'..='９').contains(&c) || "〇一二三四五六七八九十百千".contains(c);
    let core = t.strip_prefix('第').unwrap_or(&t);
    let digits: String = core.chars().take_while(|c| numeral(*c)).collect();
    if digits.is_empty() {
        return false;
    }
    let rest: String = core.chars().skip(digits.chars().count()).collect();
    // `１`, `12`, `第三章`, `第1話 タイトル`, `三`
    rest.is_empty()
        || (rest.starts_with(['章', '話', '部', '節', '幕', '巻']) && n <= 24)
        || (t.starts_with('第') && rest.chars().next().is_some_and(|c| c == '　' || c == ' '))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sentence {
    pub text: String,
    pub ruby: Vec<RubySpan>,
    /// True for the first sentence of a paragraph.
    pub paragraph_start: bool,
}

const OPENERS: &[char] = &['「', '『', '（', '(', '【', '〈', '《', '〔', '［', '“', '‘', '｛'];
const CLOSERS: &[char] = &['」', '』', '）', ')', '】', '〉', '》', '〕', '］', '”', '’', '｝'];
const TERMINALS: &[char] = &['。', '！', '？', '!', '?', '‼', '⁇', '⁈', '⁉', '．'];

fn is_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{3000}'
}

/// Splits a paragraph into sentences.
///
/// Rules:
/// * Terminal punctuation (`。！？…`) ends a sentence when we are not inside
///   brackets, *or* when the current sentence itself starts with an opening
///   bracket (a dialogue line such as `「A。B。」` → `「A。` / `B。」`).
///   Quotes embedded in narration (`彼は「何？」と言った。`) are kept whole.
/// * Trailing closers and repeated terminals stick to the sentence they end.
/// * Leading/trailing whitespace (incl. the full-width indent) is trimmed and
///   ruby spans are re-based onto the resulting sentence.
pub fn split_sentences(p: &Paragraph) -> Vec<Sentence> {
    let chars: Vec<char> = p.text.chars().collect();
    let mut bounds: Vec<(usize, usize)> = Vec::new();
    let mut start = 0usize;
    let mut depth: i32 = 0;
    // Bracket level of a dialogue quote that opened a sentence; terminals at
    // that level split the quote into several sentences.
    let mut dialogue: Option<i32> = None;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if OPENERS.contains(&c) {
            let at_start = (start..i).all(|k| is_space(chars[k]));
            depth += 1;
            if at_start && dialogue.is_none() {
                dialogue = Some(depth);
            }
        } else if CLOSERS.contains(&c) {
            depth = (depth - 1).max(0);
            if dialogue.is_some_and(|l| depth < l) {
                dialogue = None;
            }
        }
        if TERMINALS.contains(&c) && (depth == 0 || dialogue == Some(depth)) {
            let mut j = i + 1;
            while j < chars.len() && (TERMINALS.contains(&chars[j]) || CLOSERS.contains(&chars[j])) {
                if CLOSERS.contains(&chars[j]) {
                    depth = (depth - 1).max(0);
                    if dialogue.is_some_and(|l| depth < l) {
                        dialogue = None;
                    }
                }
                j += 1;
            }
            bounds.push((start, j));
            start = j;
            i = j;
            continue;
        }
        i += 1;
    }
    if start < chars.len() {
        bounds.push((start, chars.len()));
    }

    let mut out = Vec::new();
    for (s, e) in bounds {
        // trim
        let mut s2 = s;
        let mut e2 = e;
        while s2 < e2 && is_space(chars[s2]) {
            s2 += 1;
        }
        while e2 > s2 && is_space(chars[e2 - 1]) {
            e2 -= 1;
        }
        if s2 >= e2 {
            continue;
        }
        let text: String = chars[s2..e2].iter().collect();
        let ruby = p
            .ruby
            .iter()
            .filter(|r| r.start >= s2 && r.end <= e2)
            .map(|r| RubySpan { start: r.start - s2, end: r.end - s2, reading: r.reading.clone() })
            .collect();
        let paragraph_start = out.is_empty();
        out.push(Sentence { text, ruby, paragraph_start });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(s: &str) -> Vec<String> {
        split_sentences(&Paragraph { text: s.into(), ruby: vec![], ..Default::default() }).into_iter().map(|s| s.text).collect()
    }

    #[test]
    fn headings() {
        let h = |t: &str| is_heading(&Paragraph { text: t.into(), ..Default::default() });
        for t in ["１", "　２", "12", "第三章", "第1話 はじまり", "プロローグ", "三"] {
            assert!(h(t), "{t}");
        }
        for t in ["一人で帰った。", "「うん」", "三つ目", "１０ページの５行目を見なさい。", ""] {
            assert!(!h(t), "{t}");
        }
    }

    #[test]
    fn narration() {
        assert_eq!(texts("　私は猫だ。名前はまだない。"), vec!["私は猫だ。", "名前はまだない。"]);
    }

    #[test]
    fn embedded_quote_kept() {
        assert_eq!(
            texts("　先生は、「なんなの？　その勝算っていうのは」と、優しい顔で言いました。"),
            vec!["先生は、「なんなの？　その勝算っていうのは」と、優しい顔で言いました。"]
        );
    }

    #[test]
    fn dialogue_split() {
        assert_eq!(
            texts("「分かったわ。考えておく」"),
            vec!["「分かったわ。", "考えておく」"]
        );
        assert_eq!(texts("「え？！」　そうなの。"), vec!["「え？！」", "そうなの。"]);
        assert_eq!(
            texts("「見ていたの。そこで逃げられるって。それで、先生だって。でしょう？　だものね」"),
            vec!["「見ていたの。", "そこで逃げられるって。", "それで、先生だって。", "でしょう？", "だものね」"]
        );
        // nested quote inside dialogue stays whole
        assert_eq!(texts("「彼は『何？』と言った。」"), vec!["「彼は『何？』と言った。」"]);
    }

    #[test]
    fn ruby_rebased() {
        let p = Paragraph {
            text: "あ。私、小柳です。".into(),
            ruby: vec![RubySpan { start: 4, end: 6, reading: "こやなぎ".into() }],
            ..Default::default()
        };
        let s = split_sentences(&p);
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].ruby, vec![RubySpan { start: 2, end: 4, reading: "こやなぎ".into() }]);
        assert!(s[0].paragraph_start && !s[1].paragraph_start);
    }
}
