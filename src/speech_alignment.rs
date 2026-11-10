//! Provider-neutral, monotonic matching of authored speech to timed evidence.
//! ASR passages are evidence containers, never Script segment identities.
use crate::{script::ScriptDocument, transcribe::TranscriptDocument};
use anyhow::{bail, Context, Result};
use std::collections::HashMap;

#[derive(Debug, Clone)]
struct Token {
    text: String,
    start: f64,
    end: f64,
    passage: usize,
}
#[derive(Debug, Clone)]
pub struct Span {
    pub start: f64,
    pub end: f64,
    first: usize,
    last: usize,
}

fn lexical(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut latin = String::new();
    for ch in text.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() || (ch.is_alphabetic() && (ch as u32) < 0x3000) {
            latin.push(ch);
        } else {
            if !latin.is_empty() {
                tokens.push(std::mem::take(&mut latin));
            }
            if ch.is_alphanumeric() {
                tokens.push(ch.to_string());
            }
        }
    }
    if !latin.is_empty() {
        tokens.push(latin);
    }
    tokens
}
fn evidence(transcript: &TranscriptDocument) -> Result<Vec<Token>> {
    let mut out: Vec<Token> = Vec::new();
    for (p, passage) in transcript.passages.iter().enumerate() {
        for word in &passage.words {
            let parts = lexical(&word.text);
            if parts.is_empty() {
                continue;
            }
            let start = word
                .start_seconds
                .context("Evidence word missing start_seconds")?;
            let end = word
                .end_seconds
                .context("Evidence word missing end_seconds")?;
            if !start.is_finite() || !end.is_finite() || start < 0.0 || end <= start {
                bail!("Invalid evidence interval for '{}'", word.text);
            }
            if out.last().is_some_and(|prev| start + 0.001 < prev.start) {
                bail!("Evidence timestamps are not ordered");
            }
            let n = parts.len();
            // Only subdivide an observed word's measured span; never invent time
            // outside evidence. Character precision is limited by provider data.
            for (i, text) in parts.into_iter().enumerate() {
                out.push(Token {
                    text,
                    start: start + (end - start) * i as f64 / n as f64,
                    end: start + (end - start) * (i + 1) as f64 / n as f64,
                    passage: p,
                });
            }
        }
    }
    if out.is_empty() {
        bail!("Transcript contains no timed lexical evidence");
    }
    Ok(out)
}
pub fn align_units(units: &[String], transcript: &TranscriptDocument) -> Result<Vec<Span>> {
    let ev = evidence(transcript)?;
    let mut cursor = 0;
    let mut spans: Vec<Span> = Vec::new();
    for (unit_index, unit) in units.iter().enumerate() {
        let target = lexical(unit).concat();
        if target.is_empty() {
            let prev = spans.last();
            let at = prev.map(|s| s.end).unwrap_or(ev[0].start);
            let p = prev.map(|s| s.last).unwrap_or(ev[0].passage);
            spans.push(Span {
                start: at,
                end: at,
                first: p,
                last: p,
            });
            continue;
        }
        let mut found = None;
        for i in cursor..ev.len() {
            let mut joined = String::new();
            for j in i..ev.len() {
                joined.push_str(&ev[j].text);
                if joined == target {
                    found = Some((i, j));
                    break;
                }
                if !target.starts_with(&joined) {
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        let (i, j) = found.with_context(|| {
            format!(
                "Cannot align spoken unit {} ('{}'); supply complete matching speech evidence",
                unit_index, unit
            )
        })?;
        spans.push(Span {
            start: ev[i].start,
            end: ev[j].end,
            first: ev[i].passage,
            last: ev[j].passage,
        });
        cursor = j + 1;
    }
    Ok(spans)
}
pub type UnitTimings = HashMap<String, Vec<(f64, f64)>>;
pub fn align_script(
    script: &ScriptDocument,
    transcript: &TranscriptDocument,
) -> Result<(HashMap<String, f64>, UnitTimings)> {
    let targets: Vec<String> = script
        .segments
        .iter()
        .flat_map(|s| {
            s.turns
                .iter()
                .flat_map(|t| t.units.iter().map(|u| u.spoken.clone()))
        })
        .collect();
    let spans = align_units(&targets, transcript)?;
    let mut extents = HashMap::new();
    let mut timings = HashMap::new();
    let mut cursor = 0;
    let counts: Vec<usize> = script
        .segments
        .iter()
        .map(|s| s.turns.iter().map(|t| t.units.len()).sum())
        .collect();
    for (seg_idx, seg) in script.segments.iter().enumerate() {
        let n = counts[seg_idx];
        if n == 0 {
            bail!(
                "Segment '{}' has no spoken units; provide an explicit duration",
                seg.id
            );
        }
        let units = &spans[cursor..cursor + n];
        let first = &units[0];
        let last = &units[n - 1];
        let prev_shared = cursor > 0 && spans[cursor - 1].last == first.first;
        let next_shared = cursor + n < spans.len() && spans[cursor + n].first == last.last;
        let base = if prev_shared {
            first.start
        } else {
            transcript.passages[first.first]
                .start_seconds
                .unwrap_or(first.start)
        };
        let end = if next_shared {
            last.end
        } else {
            transcript.passages[last.last]
                .end_seconds
                .unwrap_or(last.end)
        };
        if !base.is_finite()
            || !end.is_finite()
            || base > first.start
            || end < last.end
            || end <= base
        {
            bail!("Invalid media domain for segment '{}'", seg.id);
        }
        extents.insert(seg.id.clone(), end - base);
        timings.insert(
            seg.id.clone(),
            units
                .iter()
                .map(|s| (s.start - base, s.end - base))
                .collect(),
        );
        cursor += n;
    }
    Ok((extents, timings))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn transcript() -> TranscriptDocument {
        serde_json::from_value(serde_json::json!({"format":"clone-any-video.transcript@1","source":"x","language":"zh","audio_seconds":4,"passages":[{"text":"中文","start_seconds":0,"end_seconds":1,"words":[{"text":"中文","start_seconds":0.1,"end_seconds":1.0}]},{"text":"内容","start_seconds":2,"end_seconds":4,"words":[{"text":"内容","start_seconds":2.0,"end_seconds":3.0}]}]})).unwrap()
    }
    #[test]
    fn matches_cjk_across_passages_and_rejects_missing() {
        let t = transcript();
        let s = crate::script::parse_script(
            "<script id=\"t\"><hook><HOST> 中文 @{next!} 内容。</hook></script>",
        )
        .unwrap();
        let (d, u) = align_script(&s, &t).unwrap();
        assert_eq!(d["hook"], 4.0);
        assert_eq!(u["hook"][1], (2.0, 3.0));
        assert!(align_units(&["不存在".into()], &t).is_err());
    }
}

/// Stable lexical identity; punctuation/layout changes do not change performed words.
pub fn spoken_key(text: &str) -> Vec<String> {
    lexical(text)
}
