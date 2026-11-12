//! Character-class helpers and furigana alignment.
//!
//! All offsets exchanged with the UI are **Unicode scalar (char) indices**,
//! see ADR-0006.

use serde::{Deserialize, Serialize};

pub fn is_kanji(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}'   // CJK Unified Ideographs
        | '\u{3400}'..='\u{4DBF}' // Extension A
        | '\u{20000}'..='\u{2FA1F}' // Extensions B..F + compat supplement
        | '\u{F900}'..='\u{FAFF}' // Compatibility ideographs
        | '々' | '〆' | 'ヵ' | 'ヶ' | '〇')
}

pub fn is_hiragana(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{309F}')
}

pub fn is_katakana(c: char) -> bool {
    matches!(c, '\u{30A1}'..='\u{30FA}' | '\u{30FD}'..='\u{30FF}' | 'ー' | '\u{31F0}'..='\u{31FF}')
}

pub fn is_kana(c: char) -> bool {
    is_hiragana(c) || is_katakana(c)
}

pub fn has_kanji(s: &str) -> bool {
    s.chars().any(is_kanji)
}

/// Katakana → hiragana (leaves everything else untouched, including `ー`).
pub fn kata_to_hira(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{30A1}'..='\u{30F6}' | '\u{30FD}'..='\u{30FE}' => {
                char::from_u32(c as u32 - 0x60).unwrap_or(c)
            }
            _ => c,
        })
        .collect()
}

/// Hiragana → katakana.
pub fn hira_to_kata(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{3041}'..='\u{3096}' | '\u{309D}'..='\u{309E}' => {
                char::from_u32(c as u32 + 0x60).unwrap_or(c)
            }
            _ => c,
        })
        .collect()
}

/// A piece of a word for rendering: `text` with an optional `ruby` reading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seg {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ruby: Option<String>,
}

impl Seg {
    pub fn plain(t: impl Into<String>) -> Self {
        Seg { text: t.into(), ruby: None }
    }
    pub fn ruby(t: impl Into<String>, r: impl Into<String>) -> Self {
        Seg { text: t.into(), ruby: Some(r.into()) }
    }
}

/// Split `surface` into kanji / non-kanji runs and align them against
/// `reading` (hiragana), so that okurigana is not covered by ruby:
/// `食べる` + `たべる` → `[食|た] べる`.
///
/// Falls back to a single ruby over the whole word when alignment fails.
pub fn align_furigana(surface: &str, reading: &str) -> Vec<Seg> {
    if !has_kanji(surface) {
        return vec![Seg::plain(surface)];
    }
    let reading = kata_to_hira(reading);
    // Runs of (is_kanji, text)
    let mut runs: Vec<(bool, String)> = Vec::new();
    for c in surface.chars() {
        let k = is_kanji(c);
        match runs.last_mut() {
            Some((lk, t)) if *lk == k => t.push(c),
            _ => runs.push((k, c.to_string())),
        }
    }
    let rchars: Vec<char> = reading.chars().collect();
    let mut out = Vec::new();
    if align_rec(&runs, 0, &rchars, 0, &mut out) {
        out
    } else if kata_to_hira(surface) == reading {
        vec![Seg::plain(surface)]
    } else {
        vec![Seg::ruby(surface, reading)]
    }
}

fn align_rec(runs: &[(bool, String)], ri: usize, r: &[char], pos: usize, out: &mut Vec<Seg>) -> bool {
    if ri == runs.len() {
        return pos == r.len();
    }
    let (is_k, text) = &runs[ri];
    if !is_k {
        let t: Vec<char> = kata_to_hira(text).chars().collect();
        if pos + t.len() <= r.len() && r[pos..pos + t.len()] == t[..] {
            out.push(Seg::plain(text.clone()));
            if align_rec(runs, ri + 1, r, pos + t.len(), out) {
                return true;
            }
            out.pop();
        }
        return false;
    }
    // Kanji run: try every non-empty reading length (shortest first is the
    // usual heuristic; the next kana run anchors the match).
    let remaining = r.len().saturating_sub(pos);
    for len in 1..=remaining {
        let reading: String = r[pos..pos + len].iter().collect();
        out.push(Seg::ruby(text.clone(), reading));
        if align_rec(runs, ri + 1, r, pos + len, out) {
            return true;
        }
        out.pop();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kana_conversion() {
        assert_eq!(kata_to_hira("カタカナー"), "かたかなー");
        assert_eq!(hira_to_kata("ひらがな"), "ヒラガナ");
    }

    #[test]
    fn furigana_okurigana() {
        assert_eq!(align_furigana("食べる", "タベル"), vec![Seg::ruby("食", "た"), Seg::plain("べる")]);
        assert_eq!(
            align_furigana("お母さん", "おかあさん"),
            vec![Seg::plain("お"), Seg::ruby("母", "かあ"), Seg::plain("さん")]
        );
        assert_eq!(align_furigana("職員室", "しょくいんしつ"), vec![Seg::ruby("職員室", "しょくいんしつ")]);
        assert_eq!(align_furigana("ひらがな", "ひらがな"), vec![Seg::plain("ひらがな")]);
        // Mismatch → whole-word ruby
        assert_eq!(align_furigana("今日は", "きょうわ"), vec![Seg::ruby("今日は", "きょうわ")]);
    }
}
