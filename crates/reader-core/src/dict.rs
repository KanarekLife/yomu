//! Yomitan-format dictionaries: import and Yomitan-style lookup (ADR-0007).

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek};

use rusqlite::{params, params_from_iter};
use serde::Serialize;
use serde_json::Value;

use crate::deinflect::{chain_cost as cost, compatible, deinflect, parse_rules};
use crate::text::{is_kana, kata_to_hira};
use crate::{now, Core, Error, Result};

#[derive(Debug, Clone, Serialize)]
pub struct DictionaryInfo {
    pub id: i64,
    pub title: String,
    pub revision: String,
    pub enabled: bool,
    pub priority: i64,
    pub term_count: i64,
    pub meta_count: i64,
    /// Catalog id this dictionary was installed from ('' for manual imports).
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Tag {
    pub name: String,
    pub category: String,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Definition {
    pub dictionary: String,
    pub tags: Vec<Tag>,
    /// Raw Yomitan glossary array (strings / structured content / objects).
    pub glossary: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct Frequency {
    pub dictionary: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pitch {
    pub dictionary: String,
    pub reading: String,
    pub positions: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TermEntry {
    pub expression: String,
    pub reading: String,
    /// Text that was matched in the sentence.
    pub source: String,
    /// Number of chars matched.
    pub length: usize,
    pub reasons: Vec<String>,
    pub term_tags: Vec<Tag>,
    pub definitions: Vec<Definition>,
    pub frequencies: Vec<Frequency>,
    pub pitches: Vec<Pitch>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LookupResult {
    pub entries: Vec<TermEntry>,
    /// Longest matched length (for highlighting).
    pub length: usize,
    pub has_dictionaries: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportProgress {
    /// Which dictionary (catalog name or file) this progress belongs to.
    pub name: String,
    pub stage: String,
    pub done: u64,
    pub total: u64,
}

fn json_str(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

impl Core {
    pub fn list_dictionaries(&self) -> Result<Vec<DictionaryInfo>> {
        let db = self.db();
        let mut st = db.prepare(
            "SELECT id, title, revision, enabled, priority, term_count, meta_count, source FROM dictionaries
             WHERE complete = 1 ORDER BY priority DESC, id",
        )?;
        let rows = st
            .query_map([], |r| {
                Ok(DictionaryInfo {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    revision: r.get(2)?,
                    enabled: r.get(3)?,
                    priority: r.get(4)?,
                    term_count: r.get(5)?,
                    meta_count: r.get(6)?,
                    source: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn set_dictionary_enabled(&self, id: i64, enabled: bool) -> Result<()> {
        self.db().execute("UPDATE dictionaries SET enabled = ?2 WHERE id = ?1", params![id, enabled])?;
        Ok(())
    }

    /// `ids` ordered from highest to lowest priority.
    pub fn reorder_dictionaries(&self, ids: &[i64]) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        for (i, id) in ids.iter().enumerate() {
            tx.execute("UPDATE dictionaries SET priority = ?2 WHERE id = ?1", params![id, (ids.len() - i) as i64])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_dictionary(&self, id: i64) -> Result<()> {
        let db = self.db();
        // Delete children explicitly in chunks-free statements; cascades are
        // slow for large dictionaries.
        db.execute("DELETE FROM terms WHERE dict_id = ?1", [id])?;
        db.execute("DELETE FROM term_meta WHERE dict_id = ?1", [id])?;
        db.execute("DELETE FROM tags WHERE dict_id = ?1", [id])?;
        db.execute("DELETE FROM dictionaries WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Import a Yomitan dictionary `.zip`. Runs on its own connection so the
    /// UI stays responsive; `progress` is called periodically.
    pub fn import_dictionary<R: Read + Seek>(
        &self,
        reader: R,
        source: &str,
        mut progress: impl FnMut(ImportProgress),
    ) -> Result<DictionaryInfo> {
        let pname = if source.is_empty() { "dictionary".to_string() } else { source.to_string() };
        let mut zip = zip::ZipArchive::new(reader).map_err(|e| Error::Dict(format!("not a zip archive: {e}")))?;
        let index: Value = {
            let f = zip.by_name("index.json").map_err(|_| Error::Dict("index.json missing – is this a Yomitan dictionary?".into()))?;
            serde_json::from_reader(f)?
        };
        let title = json_str(index.get("title"));
        if title.is_empty() {
            return Err(Error::Dict("index.json has no title".into()));
        }
        let revision = json_str(index.get("revision"));
        let format = index.get("format").or_else(|| index.get("version")).and_then(|v| v.as_i64()).unwrap_or(3);

        let names: Vec<String> = zip.file_names().filter_map(|n| n.ok().map(|n| n.into_owned())).collect();
        let bank = |prefix: &str| -> Vec<String> {
            let mut v: Vec<String> = names
                .iter()
                .filter(|n| n.starts_with(prefix) && n.ends_with(".json"))
                .cloned()
                .collect();
            v.sort_by_key(|n| n.trim_start_matches(prefix).trim_end_matches(".json").parse::<u32>().unwrap_or(0));
            v
        };
        let term_banks = bank("term_bank_");
        let meta_banks = bank("term_meta_bank_");
        let tag_banks = bank("tag_bank_");
        let total = term_banks.len() + meta_banks.len() + tag_banks.len();
        if term_banks.is_empty() && meta_banks.is_empty() {
            return Err(Error::Dict("the archive has no term banks (kanji-only dictionaries are not supported yet)".into()));
        }

        let mut conn = self.new_connection()?;
        {
            let exists: i64 = conn.query_row(
                "SELECT COUNT(*) FROM dictionaries WHERE title = ?1 AND complete = 1",
                [&title],
                |r| r.get(0),
            )?;
            if exists > 0 {
                if source.is_empty() {
                    return Err(Error::Dict(format!("“{title}” is already installed")));
                }
                // Installing a catalog entry that was imported manually before:
                // just tag it so the catalog shows it as installed.
                conn.execute("UPDATE dictionaries SET source = ?2 WHERE title = ?1 AND complete = 1", params![title, source])?;
                drop(conn);
                return self
                    .list_dictionaries()?
                    .into_iter()
                    .find(|d| d.title == title)
                    .ok_or_else(|| Error::Dict("dictionary vanished".into()));
            }
        }
        let prio: i64 = conn.query_row("SELECT COALESCE(MAX(priority), 0) FROM dictionaries", [], |r| r.get(0))?;
        conn.execute(
            "INSERT INTO dictionaries(title, revision, priority, imported_at, complete, source) VALUES (?1, ?2, ?3, ?4, 0, ?5)",
            params![title, revision, prio + 1, now(), source],
        )?;
        let dict_id = conn.last_insert_rowid();

        let result = (|| -> Result<(i64, i64)> {
            let tx = conn.transaction()?;
            let mut terms = 0i64;
            let mut metas = 0i64;
            let mut done = 0usize;
            {
                let mut ins_term = tx.prepare(
                    "INSERT INTO terms(dict_id, expression, reading, def_tags, rules, score, glossary, sequence, term_tags)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                )?;
                let mut ins_meta = tx.prepare("INSERT INTO term_meta(dict_id, expression, mode, data) VALUES (?1,?2,?3,?4)")?;
                let mut ins_tag =
                    tx.prepare("INSERT OR REPLACE INTO tags(dict_id, name, category, notes) VALUES (?1,?2,?3,?4)")?;

                let read_bank = |zip: &mut zip::ZipArchive<R>, name: &str| -> Result<Vec<Value>> {
                    let f = zip.by_name(name).map_err(|e| Error::Dict(e.to_string()))?;
                    let v: Vec<Value> = serde_json::from_reader(std::io::BufReader::new(f))?;
                    Ok(v)
                };

                for name in &tag_banks {
                    for row in read_bank(&mut zip, name)? {
                        let Some(a) = row.as_array() else { continue };
                        ins_tag.execute(params![dict_id, json_str(a.first()), json_str(a.get(1)), json_str(a.get(3))])?;
                    }
                    done += 1;
                    progress(ImportProgress { name: pname.clone(), stage: "tags".into(), done: done as u64, total: total as u64 });
                }
                // Legacy format 1/2 dictionaries store tags in index.json.
                if let Some(Value::Object(tm)) = index.get("tagMeta") {
                    for (name, meta) in tm {
                        ins_tag.execute(params![dict_id, name, json_str(meta.get("category")), json_str(meta.get("notes"))])?;
                    }
                }

                for name in &term_banks {
                    for row in read_bank(&mut zip, name)? {
                        let Some(a) = row.as_array() else { continue };
                        let expression = json_str(a.first());
                        if expression.is_empty() {
                            continue;
                        }
                        let mut reading = json_str(a.get(1));
                        if reading.is_empty() {
                            reading = expression.clone();
                        }
                        let glossary = if format == 1 {
                            Value::Array(a.iter().skip(5).cloned().collect())
                        } else {
                            a.get(5).cloned().unwrap_or(Value::Array(vec![]))
                        };
                        ins_term.execute(params![
                            dict_id,
                            expression,
                            reading,
                            json_str(a.get(2)),
                            json_str(a.get(3)),
                            a.get(4).and_then(|v| v.as_i64()).unwrap_or(0),
                            serde_json::to_string(&glossary)?,
                            if format == 1 { None } else { a.get(6).and_then(|v| v.as_i64()) },
                            if format == 1 { String::new() } else { json_str(a.get(7)) },
                        ])?;
                        terms += 1;
                    }
                    done += 1;
                    progress(ImportProgress { name: pname.clone(), stage: "terms".into(), done: done as u64, total: total as u64 });
                }

                for name in &meta_banks {
                    for row in read_bank(&mut zip, name)? {
                        let Some(a) = row.as_array() else { continue };
                        let mode = json_str(a.get(1));
                        if mode != "freq" && mode != "pitch" {
                            continue;
                        }
                        ins_meta.execute(params![
                            dict_id,
                            json_str(a.first()),
                            mode,
                            serde_json::to_string(a.get(2).unwrap_or(&Value::Null))?
                        ])?;
                        metas += 1;
                    }
                    done += 1;
                    progress(ImportProgress { name: pname.clone(), stage: "metadata".into(), done: done as u64, total: total as u64 });
                }
            }
            tx.execute(
                "UPDATE dictionaries SET term_count = ?2, meta_count = ?3, complete = 1 WHERE id = ?1",
                params![dict_id, terms, metas],
            )?;
            tx.commit()?;
            Ok((terms, metas))
        })();

        match result {
            Ok((term_count, meta_count)) => Ok(DictionaryInfo {
                id: dict_id,
                title,
                revision,
                enabled: true,
                priority: prio + 1,
                term_count,
                meta_count,
                source: source.to_string(),
            }),
            Err(e) => {
                let _ = conn.execute("DELETE FROM dictionaries WHERE id = ?1", [dict_id]);
                Err(e)
            }
        }
    }

    /// Yomitan-style scan: starting at char `offset` of `text`, try every
    /// prefix (longest first), deinflect it, and look the candidates up.
    pub fn lookup(&self, text: &str, offset: usize, max_results: usize) -> Result<LookupResult> {
        let db = self.db();
        let has_dictionaries: bool =
            db.query_row("SELECT EXISTS(SELECT 1 FROM dictionaries WHERE enabled = 1 AND complete = 1)", [], |r| r.get(0))?;
        let empty = LookupResult { entries: vec![], length: 0, has_dictionaries };
        if !has_dictionaries {
            return Ok(empty);
        }

        let chars: Vec<char> = text.chars().skip(offset).take(20).collect();
        // Stop scanning at hard punctuation.
        let stop = chars
            .iter()
            .position(|c| "。、！？!?「」『』（）()…・　 \n".contains(*c))
            .unwrap_or(chars.len());
        let chars = &chars[..stop];
        if chars.is_empty() {
            return Ok(empty);
        }

        // (len, candidate)
        let mut cands: Vec<(usize, crate::deinflect::Candidate)> = Vec::new();
        for len in (1..=chars.len()).rev() {
            let src: String = chars[..len].iter().collect();
            let mut variants = vec![src.clone()];
            let hira = kata_to_hira(&src);
            if hira != src {
                variants.push(hira);
            }
            for v in variants {
                for c in deinflect(&v) {
                    cands.push((len, c));
                }
            }
        }
        let terms: Vec<String> = cands.iter().map(|(_, c)| c.term.clone()).collect::<HashSet<_>>().into_iter().collect();

        struct Row {
            id: i64,
            dict: String,
            prio: i64,
            expression: String,
            reading: String,
            def_tags: String,
            rules: u32,
            score: i64,
            glossary: String,
            sequence: Option<i64>,
            term_tags: String,
            dict_id: i64,
        }
        let mut rows: Vec<Row> = Vec::new();
        for chunk in terms.chunks(300) {
            let ph = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT t.id, d.title, d.priority, t.expression, t.reading, t.def_tags, t.rules, t.score, t.glossary, t.sequence, t.term_tags, t.dict_id
                 FROM terms t JOIN dictionaries d ON d.id = t.dict_id
                 WHERE d.enabled = 1 AND d.complete = 1 AND (t.expression IN ({ph}) OR t.reading IN ({ph}))"
            );
            let mut st = db.prepare_cached(&sql)?;
            let it = st.query_map(params_from_iter(chunk.iter().chain(chunk.iter())), |r| {
                Ok(Row {
                    id: r.get(0)?,
                    dict: r.get(1)?,
                    prio: r.get(2)?,
                    expression: r.get(3)?,
                    reading: r.get(4)?,
                    def_tags: r.get(5)?,
                    rules: parse_rules(&r.get::<_, String>(6)?),
                    score: r.get(7)?,
                    glossary: r.get(8)?,
                    sequence: r.get(9)?,
                    term_tags: r.get(10)?,
                    dict_id: r.get(11)?,
                })
            })?;
            for r in it {
                rows.push(r?);
            }
        }

        // Best match per row: (len, reasons, matched-by-expression)
        struct Hit {
            row: usize,
            len: usize,
            reasons: Vec<&'static str>,
            by_expr: bool,
        }
        let mut best: HashMap<i64, Hit> = HashMap::new();
        for (ri, row) in rows.iter().enumerate() {
            for (len, c) in &cands {
                let by_expr = row.expression == c.term;
                let by_read = row.reading == c.term && c.term.chars().all(|ch| is_kana(ch) || ch == 'ー');
                if !(by_expr || by_read) || !compatible(c.rules, row.rules) {
                    continue;
                }
                let better = match best.get(&row.id) {
                    None => true,
                    Some(h) => {
                        (*len, std::cmp::Reverse(cost(&c.reasons)), by_expr) > (h.len, std::cmp::Reverse(cost(&h.reasons)), h.by_expr)
                    }
                };
                if better {
                    best.insert(row.id, Hit { row: ri, len: *len, reasons: c.reasons.clone(), by_expr });
                }
            }
        }
        let mut hits: Vec<Hit> = best.into_values().collect();
        hits.sort_by(|a, b| {
            let ra = &rows[a.row];
            let rb = &rows[b.row];
            b.len
                .cmp(&a.len)
                .then(b.by_expr.cmp(&a.by_expr))
                .then(cost(&a.reasons).cmp(&cost(&b.reasons)))
                .then(rb.score.cmp(&ra.score))
                .then(rb.prio.cmp(&ra.prio))
                .then(ra.sequence.unwrap_or(i64::MAX).cmp(&rb.sequence.unwrap_or(i64::MAX)))
                .then(ra.id.cmp(&rb.id))
        });

        // Tags
        let mut tag_cache: HashMap<(i64, String), Tag> = HashMap::new();
        let mut tags_for = |db: &rusqlite::Connection, dict_id: i64, s: &str| -> Vec<Tag> {
            s.split_whitespace()
                .map(|name| {
                    tag_cache
                        .entry((dict_id, name.to_string()))
                        .or_insert_with(|| {
                            db.query_row(
                                "SELECT category, notes FROM tags WHERE dict_id = ?1 AND name = ?2",
                                params![dict_id, name],
                                |r| Ok(Tag { name: name.to_string(), category: r.get(0)?, notes: r.get(1)? }),
                            )
                            .unwrap_or(Tag { name: name.to_string(), category: String::new(), notes: String::new() })
                        })
                        .clone()
                })
                .collect()
        };

        // Group by (expression, reading) keeping order.
        let mut entries: Vec<TermEntry> = Vec::new();
        let mut index: HashMap<(String, String), usize> = HashMap::new();
        for h in &hits {
            let r = &rows[h.row];
            let key = (r.expression.clone(), r.reading.clone());
            let def = Definition {
                dictionary: r.dict.clone(),
                tags: tags_for(&db, r.dict_id, &r.def_tags),
                glossary: serde_json::from_str(&r.glossary).unwrap_or(Value::Null),
            };
            if let Some(&i) = index.get(&key) {
                let e = &mut entries[i];
                e.definitions.push(def);
                for t in tags_for(&db, r.dict_id, &r.term_tags) {
                    if !e.term_tags.iter().any(|x| x.name == t.name) {
                        e.term_tags.push(t);
                    }
                }
                continue;
            }
            if entries.len() >= max_results {
                continue;
            }
            index.insert(key, entries.len());
            entries.push(TermEntry {
                expression: r.expression.clone(),
                reading: r.reading.clone(),
                source: chars[..h.len].iter().collect(),
                length: h.len,
                reasons: h.reasons.iter().map(|s| s.to_string()).collect(),
                term_tags: tags_for(&db, r.dict_id, &r.term_tags),
                definitions: vec![def],
                frequencies: vec![],
                pitches: vec![],
            });
        }

        // Frequency & pitch metadata
        if !entries.is_empty() {
            let exprs: Vec<String> = entries.iter().map(|e| e.expression.clone()).collect::<HashSet<_>>().into_iter().collect();
            let ph = vec!["?"; exprs.len()].join(",");
            let sql = format!(
                "SELECT m.expression, m.mode, m.data, d.title FROM term_meta m JOIN dictionaries d ON d.id = m.dict_id
                 WHERE d.enabled = 1 AND d.complete = 1 AND m.expression IN ({ph}) ORDER BY d.priority DESC"
            );
            let mut st = db.prepare(&sql)?;
            let metas = st
                .query_map(params_from_iter(exprs.iter()), |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (expr, mode, data, dict) in metas {
                let data: Value = serde_json::from_str(&data).unwrap_or(Value::Null);
                for e in entries.iter_mut().filter(|e| e.expression == expr) {
                    // Entries with a reading only apply to that reading.
                    let reading = data.get("reading").and_then(|r| r.as_str());
                    if reading.is_some_and(|r| r != e.reading) {
                        continue;
                    }
                    match mode.as_str() {
                        "freq" => {
                            let v = data.get("frequency").unwrap_or(&data);
                            let value = freq_display(v);
                            if !value.is_empty() {
                                e.frequencies.push(Frequency { dictionary: dict.clone(), value });
                            }
                        }
                        "pitch" => {
                            let positions: Vec<i64> = data
                                .get("pitches")
                                .and_then(|p| p.as_array())
                                .map(|a| a.iter().filter_map(|p| p.get("position").and_then(|x| x.as_i64())).collect())
                                .unwrap_or_default();
                            if !positions.is_empty() {
                                e.pitches.push(Pitch {
                                    dictionary: dict.clone(),
                                    reading: reading.unwrap_or(&e.reading).to_string(),
                                    positions,
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let length = entries.iter().map(|e| e.length).max().unwrap_or(0);
        Ok(LookupResult { entries, length, has_dictionaries })
    }
}

fn freq_display(v: &Value) -> String {
    match v {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Object(o) => o
            .get("displayValue")
            .and_then(|d| d.as_str().map(String::from))
            .or_else(|| o.get("value").map(|x| freq_display(x)))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use crate::test_core;
    use std::io::{Cursor, Write};

    fn make_dict() -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("index.json", o).unwrap();
            z.write_all(br#"{"title":"Mini","revision":"1","format":3}"#).unwrap();
            z.start_file("term_bank_1.json", o).unwrap();
            z.write_all(
                r#"[
                ["食べる","たべる","v1","v1",10,["to eat"],1,"common"],
                ["食べ物","たべもの","n","",5,["food"],2,""],
                ["食","しょく","n","",1,["meal"],3,""],
                ["書く","かく","v5","v5",5,["to write"],4,""],
                ["難しい","むずかしい","adj-i","adj-i",5,[{"type":"structured-content","content":"difficult"}],5,""]
            ]"#
                .as_bytes(),
            )
            .unwrap();
            z.start_file("term_meta_bank_1.json", o).unwrap();
            z.write_all(r#"[["食べる","freq",{"value":120,"displayValue":"120㋕"}],["食べる","pitch",{"reading":"たべる","pitches":[{"position":2}]}]]"#.as_bytes()).unwrap();
            z.start_file("tag_bank_1.json", o).unwrap();
            z.write_all(br#"[["v1","partOfSpeech",0,"Ichidan verb",0]]"#).unwrap();
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn import_and_lookup() {
        let (core, _d) = test_core();
        let info = core.import_dictionary(Cursor::new(make_dict()), "", |_| {}).unwrap();
        assert_eq!(info.term_count, 5);
        assert!(core.import_dictionary(Cursor::new(make_dict()), "", |_| {}).is_err());

        let r = core.lookup("私は食べました。", 2, 10).unwrap();
        assert_eq!(r.entries[0].expression, "食べる");
        assert_eq!(r.entries[0].source, "食べました");
        assert_eq!(r.entries[0].length, 5);
        assert_eq!(r.entries[0].frequencies[0].value, "120㋕");
        assert_eq!(r.entries[0].pitches[0].positions, vec![2]);
        assert_eq!(r.entries[0].definitions[0].tags[0].notes, "Ichidan verb");
        assert!(r.entries.iter().any(|e| e.expression == "食"));

        let r = core.lookup("食べ物がない", 0, 10).unwrap();
        assert_eq!(r.entries[0].expression, "食べ物");

        let r = core.lookup("見ていた", 0, 10).unwrap();
        assert!(r.entries.is_empty() || r.entries[0].expression != "見ていく");

        let r = core.lookup("難しそうだ", 0, 10).unwrap();
        assert_eq!(r.entries[0].expression, "難しい");

        core.set_dictionary_enabled(info.id, false).unwrap();
        assert!(!core.lookup("食べる", 0, 10).unwrap().has_dictionaries);
        core.delete_dictionary(info.id).unwrap();
        assert!(core.list_dictionaries().unwrap().is_empty());
    }
}
