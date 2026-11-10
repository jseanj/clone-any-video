use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

// ============================================================================
// 数据结构定义 (SVML 剧本语义 AST 规范)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordAttribute {
    pub key: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DisplayToken {
    #[serde(default)]
    pub separator_before: String,
    pub text: String,
    pub attributes: Vec<WordAttribute>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spoken: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlignmentUnit {
    pub display: String,
    pub spoken: String,
    pub is_dual: bool,
    pub attributes: Vec<WordAttribute>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptionCue {
    pub index: usize,
    pub text: String,
    pub tokens: Vec<DisplayToken>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Turn {
    pub role: Option<String>,
    pub spoken_text: String,
    pub units: Vec<AlignmentUnit>,
    pub cues: Vec<CaptionCue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MomentMarker {
    pub name: String,
    pub at_unit_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SelectionMarker {
    pub name: String,
    pub is_close: bool,
    pub at_unit_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Segment {
    pub id: String,
    pub turns: Vec<Turn>,
    pub moments: Vec<MomentMarker>,
    pub selections: Vec<SelectionMarker>,
    pub spoken_text: String,
    pub estimated_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScriptDocument {
    pub id: String,
    pub segments: Vec<Segment>,
    pub total_words: usize,
    pub total_estimated_seconds: f64,
}

// ============================================================================
// SVML 剧本解析器核心实现
// ============================================================================

fn extract_attr(tag: &str, attr_name: &str) -> Option<String> {
    let pattern = format!("{}=\"", attr_name);
    if let Some(start) = tag.find(&pattern) {
        let val_start = start + pattern.len();
        if let Some(end) = tag[val_start..].find('"') {
            return Some(tag[val_start..val_start + end].to_string());
        }
    }
    None
}

/// 解析 SVML `<script id="...">` 文本为 AST 剧本文档
pub fn parse_script(svml_content: &str) -> Result<ScriptDocument> {
    let content = svml_content.trim();
    if content.is_empty() {
        bail!("Empty script content");
    }

    // 查找 <script id="..."> 标签
    let script_start_idx = content.find("<script")
        .context("Missing <script> tag in SVML")?;
    let script_tag_end = content[script_start_idx..].find('>')
        .context("Malformed <script> tag")? + script_start_idx;

    let tag_header = &content[script_start_idx..script_tag_end];
    let script_id = extract_attr(tag_header, "id").unwrap_or_else(|| "default".to_string());

    let script_close = content.rfind("</script>")
        .context("Missing </script> closing tag in SVML")?;
    let body = &content[script_tag_end + 1..script_close];

    // 解析各个 Segment
    let mut segments = Vec::new();
    let mut cursor = 0;

    while cursor < body.len() {
        if let Some(open_rel) = body[cursor..].find('<') {
            let open_pos = cursor + open_rel;
            if open_pos + 1 >= body.len() {
                break;
            }

            // 检查是否是注释或无效标记
            if body[open_pos..].starts_with("<!--") {
                if let Some(comm_end) = body[open_pos..].find("-->") {
                    cursor = open_pos + comm_end + 3;
                    continue;
                }
            }

            if let Some(close_tag_rel) = body[open_pos..].find('>') {
                let tag_end = open_pos + close_tag_rel;
                let tag_content = &body[open_pos + 1..tag_end].trim();

                // 排除闭合标签与自闭合
                if !tag_content.starts_with('/') && !tag_content.ends_with('/') {
                    let seg_name = tag_content.split_whitespace().next().unwrap_or("");
                    let closing_tag = format!("</{}>", seg_name);

                    if let Some(seg_close_rel) = body[tag_end + 1..].find(&closing_tag) {
                        let seg_body_start = tag_end + 1;
                        let seg_body_end = tag_end + 1 + seg_close_rel;
                        let seg_body = &body[seg_body_start..seg_body_end];

                        let segment = parse_segment(seg_name, seg_body)?;
                        segments.push(segment);

                        cursor = seg_body_end + closing_tag.len();
                        continue;
                    }
                }
            }
            cursor = open_pos + 1;
        } else {
            break;
        }
    }

    if segments.is_empty() {
        // 如果没有找到显式 Segment 标签，整体作为一个 default segment
        let segment = parse_segment("main", body)?;
        segments.push(segment);
    }

    let mut total_words = 0;
    let mut total_estimated_seconds = 0.0;
    for s in &segments {
        for t in &s.turns {
            total_words += t.units.len();
        }
        total_estimated_seconds += s.estimated_seconds;
    }

    Ok(ScriptDocument {
        id: script_id,
        segments,
        total_words,
        total_estimated_seconds: (total_estimated_seconds * 100.0).round() / 100.0,
    })
}

/// 解析单个 Segment 内部的角色轮次、台词、双重文本与标记
fn parse_segment(id: &str, content: &str) -> Result<Segment> {
    let mut turns = Vec::new();
    let mut moments = Vec::new();
    let mut selections = Vec::new();

    let mut current_role: Option<String> = None;
    let mut current_units: Vec<AlignmentUnit> = Vec::new();
    let mut current_cues: Vec<CaptionCue> = Vec::new();
    let mut current_cue_tokens: Vec<DisplayToken> = Vec::new();
    let mut separator = String::new();

    let mut chars = content.chars().peekable();

    while let Some(&ch) = chars.peek() {
        // 0. 检查注释 <!-- ... -->
        if ch == '<' {
            let peek_str: String = chars.clone().take(4).collect();
            if peek_str == "<!--" {
                let mut comm_buf = String::new();
                while let Some(c) = chars.next() {
                    comm_buf.push(c);
                    if comm_buf.ends_with("-->") {
                        break;
                    }
                }
                continue;
            }
        }

        // 1 & 4. 检查 <...> 标签：双重文本 <Display | Spoken> 或角色切换 <ROLE>
        if ch == '<' {
            let mut clone_chars = chars.clone();
            clone_chars.next(); // skip '<'
            let mut inner = String::new();
            let mut found_gt = false;
            while let Some(c) = clone_chars.next() {
                if c == '>' {
                    found_gt = true;
                    break;
                }
                inner.push(c);
            }

            if found_gt {
                if let Some(bar_pos) = inner.find('|') {
                    // 双重文本 <Display | Spoken>
                    let disp = inner[..bar_pos].trim();
                    let spok = inner[bar_pos + 1..].trim();
                    let spoken_resolved = if spok.is_empty() { disp } else { spok };

                    // 准确消耗 '<' + inner (字符数) + '>'
                    chars.next(); // consume '<'
                    for _ in 0..inner.chars().count() {
                        chars.next();
                    }
                    chars.next(); // consume '>'

                    let attrs = parse_postfix_attributes(&mut chars);
                    let unit = AlignmentUnit {
                        display: disp.to_string(),
                        spoken: spoken_resolved.to_string(),
                        is_dual: true,
                        attributes: attrs.clone(),
                    };
                    current_units.push(unit);
                    current_cue_tokens.push(DisplayToken {
                        separator_before: if current_cue_tokens.is_empty() {separator.clear();String::new()} else {std::mem::take(&mut separator)},
                        text: disp.to_string(),
                        attributes: attrs,
                        spoken: Some(spoken_resolved.to_string()),
                    });
                    continue;
                } else if is_valid_role_tag(inner.trim()) {
                    let role_tag = inner.trim().to_string();
                    chars.next(); // consume '<'
                    for _ in 0..inner.chars().count() {
                        chars.next();
                    }
                    chars.next(); // consume '>'

                    if !current_units.is_empty() || !current_cue_tokens.is_empty() {
                        finish_cue(&mut current_cues, &mut current_cue_tokens);
                        let spoken = make_spoken_text(&current_units);
                        turns.push(Turn {
                            role: current_role.clone(),
                            spoken_text: spoken,
                            units: current_units.clone(),
                            cues: current_cues.clone(),
                        });
                        current_units.clear();
                        current_cues.clear();
                    }
                    current_role = Some(role_tag);
                    continue;
                }
            }
        }

        // 2. 检查字幕分句断点：||
        if ch == '|' {
            let mut clone_chars = chars.clone();
            clone_chars.next();
            if clone_chars.peek() == Some(&'|') {
                chars.next();
                chars.next();
                finish_cue(&mut current_cues, &mut current_cue_tokens);
                continue;
            }
        }

        // 3. 检查语义标记 @{...}
        if ch == '@' {
            let mut clone_chars = chars.clone();
            clone_chars.next(); // skip '@'
            if clone_chars.peek() == Some(&'{') {
                clone_chars.next(); // skip '{'
                let mut marker_buf = String::new();
                let mut found_brace = false;
                while let Some(c) = clone_chars.next() {
                    if c == '}' {
                        found_brace = true;
                        break;
                    }
                    marker_buf.push(c);
                }

                if found_brace {
                    chars.next(); // consume '@'
                    chars.next(); // consume '{'
                    for _ in 0..marker_buf.chars().count() {
                        chars.next();
                    }
                    chars.next(); // consume '}'

                    let marker_content = marker_buf.trim();
                    let cumulative_unit_idx: usize = turns.iter().map(|t| t.units.len()).sum::<usize>() + current_units.len();

                    if marker_content.ends_with('!') {
                        let name = marker_content.trim_end_matches('!').trim_start_matches('~');
                        moments.push(MomentMarker {
                            name: name.to_string(),
                            at_unit_index: cumulative_unit_idx,
                        });
                    } else if marker_content.starts_with('/') {
                        let name = marker_content.trim_start_matches('/').trim_end_matches('~');
                        selections.push(SelectionMarker {
                            name: name.to_string(),
                            is_close: true,
                            at_unit_index: cumulative_unit_idx,
                        });
                    } else {
                        let name = marker_content.trim_start_matches('~');
                        selections.push(SelectionMarker {
                            name: name.to_string(),
                            is_close: false,
                            at_unit_index: cumulative_unit_idx,
                        });
                    }
                    continue;
                }
            }
        }

        // 5. 空白符处理
        if ch.is_whitespace() {
            separator.push(ch);
            chars.next();
            continue;
        }

        // 6. 普通文本词汇提取
        let mut word = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() || c == '<' || c == '|' || c == '@' || c == '{' {
                break;
            }
            word.push(c);
            chars.next();
        }

        if !word.is_empty() {
            let attrs = parse_postfix_attributes(&mut chars);
            let unit = AlignmentUnit {
                display: word.clone(),
                spoken: word.clone(),
                is_dual: false,
                attributes: attrs.clone(),
            };
            current_units.push(unit);
            current_cue_tokens.push(DisplayToken {
                        separator_before: if current_cue_tokens.is_empty() {separator.clear();String::new()} else {std::mem::take(&mut separator)},
                text: word,
                attributes: attrs,
                spoken: None,
            });
        } else {
            chars.next();
        }
    }

    // 结算最后一个 Turn
    if !current_units.is_empty() || !current_cue_tokens.is_empty() {
        finish_cue(&mut current_cues, &mut current_cue_tokens);
        let spoken = make_spoken_text(&current_units);
        turns.push(Turn {
            role: current_role,
            spoken_text: spoken,
            units: current_units,
            cues: current_cues,
        });
    }

    let seg_spoken = turns
        .iter()
        .map(|t| t.spoken_text.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    let estimated = estimate_spoken_duration(&seg_spoken, turns.len());

    Ok(Segment {
        id: id.to_string(),
        turns,
        moments,
        selections,
        spoken_text: seg_spoken,
        estimated_seconds: estimated,
    })
}

fn is_valid_role_tag(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn parse_postfix_attributes<I>(chars: &mut std::iter::Peekable<I>) -> Vec<WordAttribute>
where
    I: Iterator<Item = char>,
{
    let mut attrs = Vec::new();
    if chars.peek() == Some(&'{') {
        chars.next(); // 消耗 '{'
        let mut attr_str = String::new();
        while let Some(&c) = chars.peek() {
            chars.next();
            if c == '}' {
                break;
            }
            attr_str.push(c);
        }
        for item in attr_str.split(',') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            if let Some((k, v)) = item.split_once('=') {
                attrs.push(WordAttribute {
                    key: k.trim().to_string(),
                    value: Some(v.trim().to_string()),
                });
            } else {
                attrs.push(WordAttribute {
                    key: item.to_string(),
                    value: None,
                });
            }
        }
    }
    attrs
}

fn finish_cue(cues: &mut Vec<CaptionCue>, tokens: &mut Vec<DisplayToken>) {
    if !tokens.is_empty() {
        let full_text = tokens
            .iter()
            .map(|t| format!("{}{}",t.separator_before,t.text))
            .collect::<String>();
        cues.push(CaptionCue {
            index: cues.len() + 1,
            text: full_text,
            tokens: std::mem::take(tokens),
        });
    }
}

fn make_spoken_text(units: &[AlignmentUnit]) -> String {
    let mut text=String::new();
    for unit in units {
        let closes=unit.spoken.chars().next().is_some_and(|c| ",.!?:;，。！？：；)]}”’".contains(c));
        if !text.is_empty() && !closes {text.push(' ');}
        text.push_str(&unit.spoken);
    }
    text
}

/// 语义时长预估 (Speech Duration Estimation)
/// 语速模型: 汉字按 3.8 字/秒，英文按 2.6 词/秒，标点停顿 0.25 秒，角色切分气口 0.35 秒
pub fn estimate_spoken_duration(text: &str, turn_count: usize) -> f64 {
    let mut cjk_count = 0usize;
    let mut latin_words = 0usize;
    let mut pause_count = 0usize;

    for word in text.split_whitespace() {
        let has_cjk = word.chars().any(|c| c >= '\u{4e00}' && c <= '\u{9fa5}');
        if has_cjk {
            for c in word.chars() {
                if c >= '\u{4e00}' && c <= '\u{9fa5}' {
                    cjk_count += 1;
                } else if c == '，' || c == '。' || c == '！' || c == '？' || c == ',' || c == '.' {
                    pause_count += 1;
                }
            }
        } else {
            latin_words += 1;
            if word.ends_with(',') || word.ends_with('.') || word.ends_with('!') || word.ends_with('?') {
                pause_count += 1;
            }
        }
    }

    let cjk_sec = (cjk_count as f64) / 3.8;
    let latin_sec = (latin_words as f64) / 2.6;
    let pause_sec = (pause_count as f64) * 0.25;
    let turn_overhead = (turn_count as f64) * 0.35;

    let total = cjk_sec + latin_sec + pause_sec + turn_overhead;
    (total * 10.0).round() / 10.0
}

// ============================================================================
// 导出视图辅助方法
// ============================================================================

impl ScriptDocument {
    /// 导出纯发音文本流 (给 TTS 合成器或 Whisper 对齐器使用)
    pub fn to_spoken_stream(&self) -> String {
        self.segments
            .iter()
            .map(|s| s.spoken_text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// 导出带角色对话流 (给大模型或语音演员试音查看)
    pub fn to_dialogue_stream(&self) -> String {
        let mut out = String::new();
        for seg in &self.segments {
            out.push_str(&format!("=== [{}] ===\n", seg.id));
            for turn in &seg.turns {
                let role = turn.role.as_deref().unwrap_or("VOICE");
                out.push_str(&format!("{}: {}\n", role, turn.spoken_text));
            }
            out.push('\n');
        }
        out.trim().to_string()
    }

    /// 导出字幕 Cues 列表 (给字幕渲染引擎使用)
    pub fn to_caption_cues(&self) -> Vec<CaptionCue> {
        let mut all_cues = Vec::new();
        for seg in &self.segments {
            for turn in &seg.turns {
                for cue in &turn.cues {
                    let mut c = cue.clone();
                    c.index = all_cues.len() + 1;
                    all_cues.push(c);
                }
            }
        }
        all_cues
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dual_text_and_roles() {
        let svml = r#"
        <script id="promo">
          <hook>
            <HOST> @{claim!} I made the <API | A P I>{emphasis} || work overnight. @{/claim}
            <GUEST> Really? <CSS | see ess ess> too?
          </hook>
        </script>
        "#;

        let doc = parse_script(svml).expect("script should parse successfully");
        assert_eq!(doc.id, "promo");
        assert_eq!(doc.segments.len(), 1);

        let seg = &doc.segments[0];
        assert_eq!(seg.id, "hook");
        assert_eq!(seg.turns.len(), 2);

        // Turn 1: HOST
        let turn1 = &seg.turns[0];
        assert_eq!(turn1.role.as_deref(), Some("HOST"));
        assert!(turn1.spoken_text.contains("A P I"));
        assert_eq!(turn1.cues.len(), 2);
        assert_eq!(turn1.cues[0].text, "I made the API");
        assert_eq!(turn1.cues[1].text, "work overnight.");

        // Check dual text unit
        let api_unit = turn1.units.iter().find(|u| u.display == "API").unwrap();
        assert_eq!(api_unit.spoken, "A P I");
        assert_eq!(api_unit.is_dual, true);
        assert_eq!(api_unit.attributes[0].key, "emphasis");

        // Turn 2: GUEST
        let turn2 = &seg.turns[1];
        assert_eq!(turn2.role.as_deref(), Some("GUEST"));
        assert!(turn2.spoken_text.contains("see ess ess"));
    }

    #[test]
    fn test_spoken_duration_estimation() {
        let en_text = "Hello world welcome to clone any video";
        let dur = estimate_spoken_duration(en_text, 1);
        assert!(dur >= 2.0 && dur <= 4.5);

        let zh_text = "你好世界，欢迎来到视频克隆系统。";
        let dur_zh = estimate_spoken_duration(zh_text, 1);
        assert!(dur_zh >= 3.0 && dur_zh <= 6.0);
    }

    #[test]
    fn test_chinese_dual_text_and_markers_do_not_swallow_subsequent_text() {
        let svml = r#"
        <script id="chinese_test">
          <seg1>
            <中文 | 中 文> 后续文本必须保留。
          </seg1>
          <seg2>
            Hello @{重点!} 后续文本必须保留。
          </seg2>
        </script>
        "#;
        let doc = parse_script(svml).expect("script should parse successfully");
        assert_eq!(doc.segments.len(), 2);

        let seg1 = &doc.segments[0];
        assert_eq!(seg1.turns[0].spoken_text, "中 文 后续文本必须保留。");

        let seg2 = &doc.segments[1];
        assert_eq!(seg2.turns[0].spoken_text, "Hello 后续文本必须保留。");
        assert_eq!(seg2.moments.len(), 1);
        assert_eq!(seg2.moments[0].name, "重点");
        assert_eq!(seg2.moments[0].at_unit_index, 1);
    }
}
