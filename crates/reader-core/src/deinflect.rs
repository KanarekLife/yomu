//! Rule-based deinflection in the style of Yomichan/Yomitan (ADR-0007).
//!
//! A rule rewrites a suffix (`kana_in` → `kana_out`). `rules_in` is the word
//! class the inflected form must have (0 = only the original text), and
//! `rules_out` is the class of the result. Chaining rules turns
//! `書かなかった` → `書かない` (adj-i) → `書く` (v5).

use std::sync::OnceLock;

pub const V1: u32 = 1 << 0;
pub const V5: u32 = 1 << 1;
pub const VS: u32 = 1 << 2;
pub const VK: u32 = 1 << 3;
pub const ADJ_I: u32 = 1 << 4;
/// Intermediate classes (never present on dictionary entries).
pub const MASU: u32 = 1 << 8;
pub const STEM: u32 = 1 << 9;
pub const TE: u32 = 1 << 10;
pub const TA: u32 = 1 << 11;

/// Maps Yomitan/JMdict rule identifiers (`v5k`, `v1-s`, `vs-i`, `adj-i`, …)
/// onto our flags.
pub fn parse_rules(s: &str) -> u32 {
    let mut f = 0;
    for r in s.split_whitespace() {
        if r.starts_with("v5") || r == "v4r" || r == "v2r-s" {
            f |= V5;
        } else if r.starts_with("v1") {
            f |= V1;
        } else if r.starts_with("vs") || r == "vz" {
            f |= VS;
        } else if r == "vk" {
            f |= VK;
        } else if r == "adj-i" || r == "adj-ix" {
            f |= ADJ_I;
        }
    }
    f
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub reason: &'static str,
    pub kana_in: String,
    pub kana_out: String,
    pub rules_in: u32,
    pub rules_out: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub term: String,
    /// 0 = untouched source text (matches any entry).
    pub rules: u32,
    /// Innermost-first list of transformations applied.
    pub reasons: Vec<&'static str>,
}

/// Godan rows: dictionary ending, a-, i-, e-, o-row, te, ta.
const GODAN: &[(&str, &str, &str, &str, &str, &str, &str)] = &[
    ("う", "わ", "い", "え", "お", "って", "った"),
    ("く", "か", "き", "け", "こ", "いて", "いた"),
    ("ぐ", "が", "ぎ", "げ", "ご", "いで", "いだ"),
    ("す", "さ", "し", "せ", "そ", "して", "した"),
    ("つ", "た", "ち", "て", "と", "って", "った"),
    ("ぬ", "な", "に", "ね", "の", "んで", "んだ"),
    ("ぶ", "ば", "び", "べ", "ぼ", "んで", "んだ"),
    ("む", "ま", "み", "め", "も", "んで", "んだ"),
    ("る", "ら", "り", "れ", "ろ", "って", "った"),
];

fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(build_rules)
}

fn build_rules() -> Vec<Rule> {
    let mut v: Vec<Rule> = Vec::new();
    let mut add = |reason: &'static str, i: &str, o: &str, ri: u32, ro: u32| {
        v.push(Rule { reason, kana_in: i.to_string(), kana_out: o.to_string(), rules_in: ri, rules_out: ro });
    };

    // ---- Godan verbs -------------------------------------------------------
    for &(u, a, i, e, o, te, ta) in GODAN {
        add("negative", &format!("{a}ない"), u, ADJ_I, V5);
        add("negative", &format!("{a}ず"), u, 0, V5);
        add("negative", &format!("{a}ずに"), u, 0, V5);
        add("negative", &format!("{a}ぬ"), u, 0, V5);
        add("negative", &format!("{a}ん"), u, 0, V5);
        add("passive", &format!("{a}れる"), u, V1, V5);
        add("causative", &format!("{a}せる"), u, V1, V5);
        add("causative", &format!("{a}す"), u, V5, V5);
        add("causative passive", &format!("{a}される"), u, V1, V5);
        add("masu stem", i, u, STEM, V5);
        add("potential", &format!("{e}る"), u, V1, V5);
        add("imperative", e, u, 0, V5);
        add("conditional", &format!("{e}ば"), u, 0, V5);
        add("volitional", &format!("{o}う"), u, 0, V5);
        add("te", te, u, TE, V5);
        add("past", ta, u, TA, V5);
    }
    // 行く is irregular in te/ta.
    for (stem, dict) in [("行", "行く"), ("い", "いく"), ("逝", "逝く")] {
        add("te", &format!("{stem}って"), dict, TE, V5);
        add("past", &format!("{stem}った"), dict, TA, V5);
    }

    // ---- Ichidan verbs -----------------------------------------------------
    add("negative", "ない", "る", ADJ_I, V1);
    add("negative", "ず", "る", 0, V1);
    add("negative", "ずに", "る", 0, V1);
    add("negative", "ん", "る", 0, V1);
    add("passive/potential", "られる", "る", V1, V1);
    add("potential", "れる", "る", V1, V1);
    add("causative", "させる", "る", V1, V1);
    add("causative", "さす", "る", V5, V1);
    add("masu stem", "", "る", STEM, V1);
    add("imperative", "ろ", "る", 0, V1);
    add("imperative", "よ", "る", 0, V1);
    add("conditional", "れば", "る", 0, V1);
    add("volitional", "よう", "る", 0, V1);
    add("te", "て", "る", TE, V1);
    add("past", "た", "る", TA, V1);

    // ---- する --------------------------------------------------------------
    {
        let dict = "する";
        add("negative", "しない", dict, ADJ_I, VS);
        add("negative", "せず", dict, 0, VS);
        add("negative", "せずに", dict, 0, VS);
        add("negative", "せぬ", dict, 0, VS);
        add("negative", "せん", dict, 0, VS);
        add("passive", "される", dict, V1, VS);
        add("causative", "させる", dict, V1, VS);
        add("potential", "できる", dict, V1, VS);
        add("masu stem", "し", dict, STEM, VS);
        add("imperative", "しろ", dict, 0, VS);
        add("imperative", "せよ", dict, 0, VS);
        add("conditional", "すれば", dict, 0, VS);
        add("volitional", "しよう", dict, 0, VS);
        add("te", "して", dict, TE, VS);
        add("past", "した", dict, TA, VS);
    }

    // ---- 来る --------------------------------------------------------------
    for (k, ko, ki, ku) in [("くる", "こ", "き", "く"), ("来る", "来", "来", "来")] {
        add("negative", &format!("{ko}ない"), k, ADJ_I, VK);
        add("negative", &format!("{ko}ず"), k, 0, VK);
        add("passive/potential", &format!("{ko}られる"), k, V1, VK);
        add("potential", &format!("{ko}れる"), k, V1, VK);
        add("causative", &format!("{ko}させる"), k, V1, VK);
        add("masu stem", ki, k, STEM, VK);
        add("imperative", &format!("{ko}い"), k, 0, VK);
        add("conditional", &format!("{ku}れば"), k, 0, VK);
        add("volitional", &format!("{ko}よう"), k, 0, VK);
        add("te", &format!("{ki}て"), k, TE, VK);
        add("past", &format!("{ki}た"), k, TA, VK);
    }

    // ---- i-adjectives ------------------------------------------------------
    add("adverbial", "く", "い", 0, ADJ_I);
    add("negative", "くない", "い", ADJ_I, ADJ_I);
    add("te", "くて", "い", TE, ADJ_I);
    add("past", "かった", "い", TA, ADJ_I);
    add("conditional", "ければ", "い", 0, ADJ_I);
    add("conditional", "けりゃ", "い", 0, ADJ_I);
    add("volitional", "かろう", "い", 0, ADJ_I);
    add("noun", "さ", "い", 0, ADJ_I);
    add("-sou", "そう", "い", 0, ADJ_I);
    add("-sugiru", "すぎる", "い", V1, ADJ_I);
    add("-sugiru", "過ぎる", "い", V1, ADJ_I);
    add("archaic", "き", "い", 0, ADJ_I);
    add("-ge", "げ", "い", 0, ADJ_I);
    add("negative", "なきゃ", "ない", 0, ADJ_I);
    add("negative", "なくちゃ", "ない", 0, ADJ_I);

    // ---- Polite ------------------------------------------------------------
    add("polite", "ます", "", MASU, STEM);
    add("polite past", "ました", "ます", 0, MASU);
    add("polite negative", "ません", "ます", 0, MASU);
    add("polite past negative", "ませんでした", "ます", 0, MASU);
    add("polite volitional", "ましょう", "ます", 0, MASU);
    add("polite te", "まして", "ます", 0, MASU);
    add("polite imperative", "ませ", "ます", 0, MASU);

    // ---- Things attached to the masu stem ---------------------------------
    add("-tai", "たい", "", ADJ_I, STEM);
    add("-tagaru", "たがる", "", V5, STEM);
    add("-sou", "そう", "", 0, STEM);
    add("-nagara", "ながら", "", 0, STEM);
    add("-nasai", "なさい", "", 0, STEM);
    add("-sugiru", "すぎる", "", V1, STEM);
    add("-sugiru", "過ぎる", "", V1, STEM);
    add("-yasui", "やすい", "", ADJ_I, STEM);
    add("-nikui", "にくい", "", ADJ_I, STEM);
    add("-kata", "方", "", 0, STEM);
    add("-kata", "かた", "", 0, STEM);

    // ---- Things attached to the te / ta forms ------------------------------
    add("-tara", "たら", "た", 0, TA);
    add("-tara", "だら", "だ", 0, TA);
    add("-tari", "たり", "た", 0, TA);
    add("-tari", "だり", "だ", 0, TA);
    add("-temo", "ても", "て", 0, TE);
    add("-temo", "でも", "で", 0, TE);
    add("-tewa", "ては", "て", 0, TE);
    add("-tewa", "では", "で", 0, TE);
    add("-cha", "ちゃ", "ては", 0, TE);
    add("-cha", "じゃ", "では", 0, TE);
    for (aux, class, reason) in [
        ("いる", V1, "-te iru"),
        ("る", V1, "-te iru"),
        ("おる", V5, "-te oru"),
        ("おく", V5, "-te oku"),
        ("しまう", V5, "-te shimau"),
        ("いく", V5, "-te iku"),
        ("く", V5, "-te iku"),
        ("くる", VK, "-te kuru"),
        ("ある", V5, "-te aru"),
        ("みる", V1, "-te miru"),
        ("くれる", V1, "-te kureru"),
        ("あげる", V1, "-te ageru"),
        ("もらう", V5, "-te morau"),
        ("ほしい", ADJ_I, "-te hoshii"),
        ("ください", 0, "-te kudasai"),
    ] {
        add(reason, &format!("て{aux}"), "て", class, TE);
        add(reason, &format!("で{aux}"), "で", class, TE);
    }
    add("-te shimau", "ちゃう", "て", V5, TE);
    add("-te shimau", "じゃう", "で", V5, TE);
    add("-te shimau", "ちまう", "て", V5, TE);
    add("-te oku", "とく", "て", V5, TE);
    add("-te oku", "どく", "で", V5, TE);
    v
}

/// Generate all deinflection candidates of `source` (including itself).
pub fn deinflect(source: &str) -> Vec<Candidate> {
    let mut out = vec![Candidate { term: source.to_string(), rules: 0, reasons: vec![] }];
    let mut i = 0;
    while i < out.len() && out.len() < 512 {
        let cur = out[i].clone();
        i += 1;
        if cur.reasons.len() >= 6 {
            continue;
        }
        for r in rules() {
            if cur.rules == 0 {
                // From the raw text, intermediate-only rules (empty suffix)
                // must not fire.
                if r.kana_in.is_empty() {
                    continue;
                }
            } else if cur.rules & r.rules_in == 0 {
                continue;
            }
            if !cur.term.ends_with(r.kana_in.as_str()) {
                continue;
            }
            let base = &cur.term[..cur.term.len() - r.kana_in.len()];
            if base.is_empty() && r.kana_out.is_empty() {
                continue;
            }
            let term = format!("{base}{}", r.kana_out);
            if term.chars().count() == 0 || term == cur.term && r.rules_out == cur.rules {
                continue;
            }
            let mut reasons = vec![r.reason];
            reasons.extend(cur.reasons.iter().copied());
            if let Some(existing) = out.iter_mut().find(|c| c.term == term && c.rules == r.rules_out) {
                // Same form reached another way: keep the more plausible chain.
                if chain_cost(&reasons) < chain_cost(&existing.reasons) {
                    existing.reasons = reasons;
                }
                continue;
            }
            out.push(Candidate { term, rules: r.rules_out, reasons });
        }
    }
    out
}

/// Ranking cost of a deinflection chain: number of steps, with rare readings
/// of ambiguous suffixes penalised (見ていた is 〜ている + past, not 〜ていく).
pub fn chain_cost(reasons: &[&'static str]) -> usize {
    reasons.len() * 2 + reasons.iter().filter(|r| matches!(**r, "-te iku" | "-te oku" | "-te oru" | "potential")).count()
}

/// Whether a dictionary entry with `entry_rules` can match a candidate.
pub fn compatible(candidate_rules: u32, entry_rules: u32) -> bool {
    candidate_rules == 0 || candidate_rules & entry_rules != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(src: &str, term: &str, class: u32) -> bool {
        deinflect(src).iter().any(|c| c.term == term && compatible(c.rules, class))
    }

    #[test]
    fn verbs() {
        assert!(has("書かなかった", "書く", V5));
        assert!(has("食べました", "食べる", V1));
        assert!(has("食べています", "食べる", V1));
        assert!(has("読んでいた", "読む", V5));
        assert!(has("行った", "行く", V5));
        assert!(has("走らされてしまった", "走る", V5));
        assert!(has("言いました", "言う", V5));
        assert!(has("勉強した", "勉強する", VS));
        assert!(has("来なかった", "来る", VK));
        assert!(has("しまった", "しまう", V5));
        assert!(has("なっちゃった", "なる", V5));
        assert!(has("呼びだされた", "呼びだす", V5));
        assert!(has("教えてあげます", "教える", V1));
        assert!(has("食べたかった", "食べる", V1));
    }

    #[test]
    fn adjectives() {
        assert!(has("難しそう", "難しい", ADJ_I));
        assert!(has("嬉しかった", "嬉しい", ADJ_I));
        assert!(has("優しくない", "優しい", ADJ_I));
    }

    #[test]
    fn class_mismatch_rejected() {
        // 書いた is not an ichidan form of 書いる
        let c = deinflect("書いた");
        assert!(c.iter().any(|c| c.term == "書く" && c.rules == V5));
        assert!(!c.iter().any(|c| c.term == "書く" && compatible(c.rules, V1)));
    }

    #[test]
    fn rule_parsing() {
        assert_eq!(parse_rules("v5k vt"), V5);
        assert_eq!(parse_rules("v1 vi"), V1);
        assert_eq!(parse_rules("adj-i"), ADJ_I);
        assert_eq!(parse_rules("vs"), VS);
    }
}

#[cfg(test)]
mod chain_tests {
    use super::*;
    #[test]
    fn te_iru_past_chain_exists() {
        let c = deinflect("見ていた");
        assert!(c.iter().any(|c| c.term == "見る" && c.rules & V1 != 0 && c.reasons.contains(&"-te iru")));
    }
}
