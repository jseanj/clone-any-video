use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::script::ScriptDocument;

// ============================================================================
// 1. 核心数据模型 (字幕语义数据结构)
// ============================================================================

/// 词级字幕显示单元 (CaptionWord)
/// 双轨文本规范：separator_before 保存前置空格或分隔符，
/// 严禁通过猜字符或在渲染阶段拼接空格重构词句。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptionWord {
    pub id: String,
    pub unit_id: String,
    pub text: String,
    pub separator_before: String,
    #[serde(default)]
    pub attributes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spoken: Option<String>,
}

/// 语义对应单元 (CaptionUnit)
/// 包含一个或多个 Word（如复合词、英文连词或 <双文本|Dual Text> 单元）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptionUnit {
    pub id: String,
    pub word_ids: Vec<String>,
}

/// 剧本排版条目 (DocCue)
/// 对应 Script 中的一个有声陈述片段，由 || 或自然段边界切分
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DocCue {
    pub id: String,
    pub unit_ids: Vec<String>,
    #[serde(default)]
    pub role: String,
}

/// 完整的字幕文档 (CaptionDocument)
/// 具有不变式（Invariants）：
/// 1. Cues 严格顺序划分 Units，无遗漏无重叠；
/// 2. Units 严格顺序划分 Words；
/// 3. 文档本身不包含任何秒数或帧率，纯粹由语义决定。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptionDocument {
    pub id: String,
    pub words: Vec<CaptionWord>,
    pub units: Vec<CaptionUnit>,
    pub cues: Vec<DocCue>,
}

impl CaptionDocument {
    /// 校验 CaptionDocument 的结构不变式
    pub fn assert_identity(&self) -> Result<()> {
        let mut word_map = HashMap::new();
        for (idx, word) in self.words.iter().enumerate() {
            if word_map.insert(&word.id, idx).is_some() {
                bail!("Duplicate word id in CaptionDocument: {}", word.id);
            }
        }

        let mut unit_map = HashMap::new();
        let mut covered_word_indices = Vec::new();
        for (idx, unit) in self.units.iter().enumerate() {
            if unit_map.insert(&unit.id, idx).is_some() {
                bail!("Duplicate unit id in CaptionDocument: {}", unit.id);
            }
            if unit.word_ids.is_empty() {
                bail!("Unit {} contains no words", unit.id);
            }
            for w_id in &unit.word_ids {
                match word_map.get(w_id) {
                    Some(&w_idx) => covered_word_indices.push(w_idx),
                    None => bail!("Unit {} references unknown word {}", unit.id, w_id),
                }
            }
        }

        // 验证 Units 严格顺序划分 Words
        for (i, &w_idx) in covered_word_indices.iter().enumerate() {
            if w_idx != i {
                bail!("Units must partition words in sequential order without gaps");
            }
        }
        if covered_word_indices.len() != self.words.len() {
            bail!("Not all words are covered by units in CaptionDocument");
        }

        // 验证 Cues 严格顺序划分 Units
        let mut covered_unit_indices = Vec::new();
        for cue in &self.cues {
            if cue.unit_ids.is_empty() {
                bail!("Cue {} contains no units", cue.id);
            }
            for u_id in &cue.unit_ids {
                match unit_map.get(u_id) {
                    Some(&u_idx) => covered_unit_indices.push(u_idx),
                    None => bail!("Cue {} references unknown unit {}", cue.id, u_id),
                }
            }
        }

        for (i, &u_idx) in covered_unit_indices.iter().enumerate() {
            if u_idx != i {
                bail!("Cues must partition units in sequential order without gaps");
            }
        }
        if covered_unit_indices.len() != self.units.len() {
            bail!("Not all units are covered by cues in CaptionDocument");
        }

        Ok(())
    }
}

// ============================================================================
// 2. 时间轴对齐与时间映射 (CaptionTiming)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UnitTiming {
    pub unit_id: String,
    pub start_time: f64,
    pub end_time: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptionTiming {
    pub timeline_id: String,
    pub document_id: String,
    pub units: Vec<UnitTiming>,
}

// ============================================================================
// 3. 视觉样式配方 (CaptionRecipe / CaptionStyle)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Placement {
    pub x: f64,           // 归一化中心或锚点横坐标 (0.0..1.0, 默认 0.5)
    pub y: f64,           // 归一化中心或锚点纵坐标 (0.0..1.0, 默认 0.88)
    pub width: f64,       // 归一化最大可用宽度 (0.0..1.0, 默认 0.86)
    pub height: f64,      // 归一化最大高度 (0.0..1.0, 默认 0.20)
    pub anchor_x: String, // "center" | "left" | "right"
    pub anchor_y: String, // "bottom" | "center" | "top"
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            x: 0.5,
            y: 0.88,
            width: 0.86,
            height: 0.20,
            anchor_x: "center".into(),
            anchor_y: "bottom".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Typography {
    pub font_family: String,
    pub font_size_px: f64,
    pub font_weight: u32,
    pub line_height: f64,
    pub fill_color: String,
    pub stroke_color: String,
    pub stroke_width_px: f64,
    pub shadow_color: String,
    pub shadow_blur_px: f64,
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            font_family: "PingFang SC, -apple-system, sans-serif".into(),
            font_size_px: 44.0,
            font_weight: 800,
            line_height: 1.25,
            fill_color: "#FFFFFF".into(),
            stroke_color: "#0B0E14".into(),
            stroke_width_px: 3.5,
            shadow_color: "rgba(0, 0, 0, 0.75)".into(),
            shadow_blur_px: 8.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Layout {
    pub wrap: String,              // "word" | "char"
    pub max_lines: usize,          // 最大折行数 (通常为 2)
    pub max_words_per_line: usize, // 单行最大词数 (0 表示不限制)
    pub align: String,             // "center" | "left"
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            wrap: "word".into(),
            max_lines: 2,
            max_words_per_line: 0,
            align: "center".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum KaraokeMode {
    None,
    Current,    // 仅当前发音词高亮
    Trail,      // 已经讲过的词保持高亮
    AtomReveal, // 打字机/逐词显现
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum KaraokeTransition {
    Step, // 词首帧瞬间激活
    Wipe, // 平滑字形扫光
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KaraokeConfig {
    pub mode: KaraokeMode,
    pub transition: KaraokeTransition,
    pub active_fill: String,
    pub active_box: bool,
    pub active_box_color: String,
    pub active_underline: bool,
    pub active_scale: f64, // 1.08 动感微缩放
}

impl Default for KaraokeConfig {
    fn default() -> Self {
        Self {
            mode: KaraokeMode::Trail,
            transition: KaraokeTransition::Step,
            active_fill: "#F59E0B".into(), // 活力琥珀金
            active_box: true,
            active_box_color: "rgba(245, 158, 11, 0.22)".into(),
            active_underline: false,
            active_scale: 1.06,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimingEnvelope {
    pub lead_seconds: f64, // 视线引导前置时间 (默认 0.10s ~ 3 帧)
    pub tail_seconds: f64, // 视觉驻留后置时间 (默认 0.20s ~ 6 帧)
    pub handoff: String,   // "cut" | "fade"
}

impl Default for TimingEnvelope {
    fn default() -> Self {
        Self {
            lead_seconds: 0.10,
            tail_seconds: 0.20,
            handoff: "cut".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptionRecipe {
    pub id: String,
    pub placement: Placement,
    pub typography: Typography,
    pub layout: Layout,
    pub karaoke: KaraokeConfig,
    pub envelope: TimingEnvelope,
}

impl Default for CaptionRecipe {
    fn default() -> Self {
        Self {
            id: "caption.fine.default".into(),
            placement: Placement::default(),
            typography: Typography::default(),
            layout: Layout::default(),
            karaoke: KaraokeConfig::default(),
            envelope: TimingEnvelope::default(),
        }
    }
}

// ============================================================================
// 4. 动态人头追踪 (Caption Tracking & RegionEvidence)
// ============================================================================

/// 人物在画面中的观察证据 (Region Evidence)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FaceObservation {
    pub timestamp: f64,
    pub role: String,
    pub box_normalized: [f64; 4], // [x, y, w, h] 范围 0.0..1.0
}

/// 动态人头追踪配置
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HeadTrackingConfig {
    pub enabled: bool,
    pub target_role: String,
    pub offset_y: f64,    // 距离头顶上方的偏移量 (归一化高度，例如 0.06)
    pub clamp_y_min: f64, // 避免冲出顶部画布，例如 0.08
    pub clamp_y_max: f64, // 避免沉到底部遮挡，例如 0.90
}

impl Default for HeadTrackingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            target_role: "HOST".into(),
            offset_y: 0.06,
            clamp_y_min: 0.08,
            clamp_y_max: 0.88,
        }
    }
}

// ============================================================================
// 5. 排期结果模型 (CaptionSchedule)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScheduledWord {
    pub id: String,
    pub text: String,
    pub separator_before: String,
    pub start_time: f64,
    pub end_time: f64,
    pub is_keyword: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScheduledCue {
    pub cue_id: String,
    pub role: String,
    pub timed_start: f64,   // 发音开始时间 (语义时间)
    pub timed_end: f64,     // 发音结束时间 (语义时间)
    pub visible_start: f64, // 视觉出现时间 (含 lead 保护)
    pub visible_end: f64,   // 视觉消失时间 (含 tail 保护与切条截断)
    pub words: Vec<ScheduledWord>,
    pub display_text: String,
    pub tracking_anchor: Option<(f64, f64)>, // 动态人头追踪坐标 (norm_x, norm_y)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptionSchedule {
    pub timeline_id: String,
    pub recipe_id: String,
    pub cues: Vec<ScheduledCue>,
}

// ============================================================================
// 6. 字幕引擎排期与构建实现
// ============================================================================

/// 从解析好的 SVML 剧本文档构造规范的 CaptionDocument
pub fn build_caption_document_from_script(script_doc: &ScriptDocument) -> Result<CaptionDocument> {
    let mut words = Vec::new();
    let mut units = Vec::new();
    let mut cues = Vec::new();

    let mut word_counter = 0;
    let mut unit_counter = 0;
    let mut cue_counter = 0;

    for segment in &script_doc.segments {
        for turn in &segment.turns {
            let role = turn.role.clone().unwrap_or_else(|| "HOST".into());

            // 如果 Turn 显式提供了 cues（由 || 切分）
            if !turn.cues.is_empty() {
                for c in &turn.cues {
                    cue_counter += 1;
                    let cue_id = format!("cue-{:03}", cue_counter);
                    let mut cue_unit_ids = Vec::new();

                    for token in &c.tokens {
                        unit_counter += 1;
                        let unit_id = format!("unit-{:03}", unit_counter);
                        let mut unit_word_ids = Vec::new();

                        // token.text 可以拆为词（英文按空格拆分但保留前置 separator）
                        let raw_text = &token.text;
                        let is_cjk = raw_text
                            .chars()
                            .any(|ch| ch as u32 >= 0x4E00 && ch as u32 <= 0x9FFF);

                        if is_cjk {
                            // 中文逐字为最小词元
                            for (idx, ch) in raw_text.chars().enumerate() {
                                word_counter += 1;
                                let word_id = format!("word-{:04}", word_counter);
                                unit_word_ids.push(word_id.clone());
                                words.push(CaptionWord {
                                    id: word_id,
                                    unit_id: unit_id.clone(),
                                    text: ch.to_string(),
                                    separator_before: if idx == 0 {
                                        token.separator_before.clone()
                                    } else {
                                        String::new()
                                    },
                                    attributes: token
                                        .attributes
                                        .iter()
                                        .map(|a| a.key.clone())
                                        .collect(),
                                    spoken: token.spoken.clone(),
                                });
                            }
                        } else {
                            // 英文词元
                            word_counter += 1;
                            let word_id = format!("word-{:04}", word_counter);
                            unit_word_ids.push(word_id.clone());
                            words.push(CaptionWord {
                                id: word_id,
                                unit_id: unit_id.clone(),
                                text: raw_text.clone(),
                                separator_before: token.separator_before.clone(),
                                attributes: token
                                    .attributes
                                    .iter()
                                    .map(|a| a.key.clone())
                                    .collect(),
                                spoken: token.spoken.clone(),
                            });
                        }

                        units.push(CaptionUnit {
                            id: unit_id.clone(),
                            word_ids: unit_word_ids,
                        });
                        cue_unit_ids.push(unit_id);
                    }

                    cues.push(DocCue {
                        id: cue_id,
                        unit_ids: cue_unit_ids,
                        role: role.clone(),
                    });
                }
            } else if !turn.units.is_empty() {
                // 没有 || 时，以整 Turn 作为一个 Cue
                cue_counter += 1;
                let cue_id = format!("cue-{:03}", cue_counter);
                let mut cue_unit_ids = Vec::new();

                for u in &turn.units {
                    unit_counter += 1;
                    let unit_id = format!("unit-{:03}", unit_counter);
                    let mut unit_word_ids = Vec::new();

                    let raw_text = &u.display;
                    let is_cjk = raw_text
                        .chars()
                        .any(|ch| ch as u32 >= 0x4E00 && ch as u32 <= 0x9FFF);

                    if is_cjk {
                        for ch in raw_text.chars() {
                            word_counter += 1;
                            let word_id = format!("word-{:04}", word_counter);
                            unit_word_ids.push(word_id.clone());
                            words.push(CaptionWord {
                                id: word_id,
                                unit_id: unit_id.clone(),
                                text: ch.to_string(),
                                separator_before: "".into(),
                                attributes: u.attributes.iter().map(|a| a.key.clone()).collect(),
                                spoken: if u.is_dual || !u.spoken.is_empty() {
                                    Some(u.spoken.clone())
                                } else {
                                    None
                                },
                            });
                        }
                    } else {
                        word_counter += 1;
                        let word_id = format!("word-{:04}", word_counter);
                        unit_word_ids.push(word_id.clone());
                        words.push(CaptionWord {
                            id: word_id,
                            unit_id: unit_id.clone(),
                            text: raw_text.clone(),
                            separator_before: " ".into(),
                            attributes: u.attributes.iter().map(|a| a.key.clone()).collect(),
                            spoken: if u.is_dual || !u.spoken.is_empty() {
                                Some(u.spoken.clone())
                            } else {
                                None
                            },
                        });
                    }

                    units.push(CaptionUnit {
                        id: unit_id.clone(),
                        word_ids: unit_word_ids,
                    });
                    cue_unit_ids.push(unit_id);
                }

                cues.push(DocCue {
                    id: cue_id,
                    unit_ids: cue_unit_ids,
                    role: role.clone(),
                });
            }
        }
    }

    let doc = CaptionDocument {
        id: format!("{}.caption", script_doc.id),
        words,
        units,
        cues,
    };

    doc.assert_identity()
        .context("Failed asserting identity of constructed CaptionDocument")?;
    Ok(doc)
}

/// 根据已解析的语音时长分布（例如来自 A-roll 配音或 Whisper 对齐），均匀或按权重将时间投影到每个 Unit
pub fn build_caption_timing_from_speech_intervals(
    doc: &CaptionDocument,
    speech_intervals: &[(f64, f64)],
) -> Result<CaptionTiming> {
    if speech_intervals.len() != doc.cues.len() && !doc.cues.is_empty() {
        // 如果区间数与 cue 数不同，退化为在整体首尾时间按字符长度比例分配
        let start = speech_intervals.first().map(|s| s.0).unwrap_or(0.0);
        let end = speech_intervals.last().map(|s| s.1).unwrap_or(1.0);
        return distribute_timing_linearly(doc, start, end);
    }

    let mut unit_timings = Vec::new();
    let unit_map: HashMap<&str, &CaptionUnit> =
        doc.units.iter().map(|u| (u.id.as_str(), u)).collect();
    let word_map: HashMap<&str, &CaptionWord> =
        doc.words.iter().map(|w| (w.id.as_str(), w)).collect();

    for (cue_idx, cue) in doc.cues.iter().enumerate() {
        let (cue_start, cue_end) = speech_intervals[cue_idx];
        let cue_duration = (cue_end - cue_start).max(0.05);

        // 计算此 Cue 下所有 Unit 的总字符/权重
        let mut unit_weights = Vec::new();
        let mut total_weight = 0.0;
        for u_id in &cue.unit_ids {
            let u = unit_map.get(u_id.as_str()).context("Unit not found")?;
            let mut char_count = 0;
            for w_id in &u.word_ids {
                if let Some(w) = word_map.get(w_id.as_str()) {
                    char_count += w.text.chars().count().max(1);
                }
            }
            let w = (char_count as f64).max(1.0);
            unit_weights.push(w);
            total_weight += w;
        }

        let mut current_t = cue_start;
        for (idx, u_id) in cue.unit_ids.iter().enumerate() {
            let frac = unit_weights[idx] / total_weight;
            let dur = cue_duration * frac;
            let u_start = current_t;
            let u_end = if idx == cue.unit_ids.len() - 1 {
                cue_end
            } else {
                current_t + dur
            };
            current_t = u_end;

            unit_timings.push(UnitTiming {
                unit_id: u_id.clone(),
                start_time: u_start,
                end_time: u_end,
            });
        }
    }

    Ok(CaptionTiming {
        timeline_id: "program.timeline".into(),
        document_id: doc.id.clone(),
        units: unit_timings,
    })
}

/// Bind display Units to measured speech; missing evidence is an error.
pub fn build_caption_timing_from_transcript(
    doc: &CaptionDocument,
    transcript: &crate::transcribe::TranscriptDocument,
) -> Result<CaptionTiming> {
    doc.assert_identity()?;
    let word_map: HashMap<&str, &CaptionWord> =
        doc.words.iter().map(|w| (w.id.as_str(), w)).collect();
    let targets: Vec<String> = doc
        .units
        .iter()
        .map(|unit| {
            let words: Vec<&CaptionWord> = unit
                .word_ids
                .iter()
                .map(|id| word_map[id.as_str()])
                .collect();
            if let Some(spoken) = words.first().and_then(|w| w.spoken.as_ref()) {
                if words.iter().all(|w| w.spoken.as_ref() == Some(spoken)) {
                    return spoken.clone();
                }
            }
            words
                .iter()
                .map(|w| w.spoken.as_deref().unwrap_or(&w.text))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    let spans = crate::speech_alignment::align_units(&targets, transcript)?;
    Ok(CaptionTiming {
        timeline_id: "program.timeline".into(),
        document_id: doc.id.clone(),
        units: doc
            .units
            .iter()
            .zip(spans)
            .map(|(u, s)| UnitTiming {
                unit_id: u.id.clone(),
                start_time: s.start,
                end_time: s.end,
            })
            .collect(),
    })
}

fn distribute_timing_linearly(
    doc: &CaptionDocument,
    start: f64,
    end: f64,
) -> Result<CaptionTiming> {
    let total_units = doc.units.len();
    if total_units == 0 {
        return Ok(CaptionTiming {
            timeline_id: "program.timeline".into(),
            document_id: doc.id.clone(),
            units: Vec::new(),
        });
    }

    let total_duration = (end - start).max(0.1);
    let step = total_duration / total_units as f64;
    let mut unit_timings = Vec::new();

    for (idx, u) in doc.units.iter().enumerate() {
        let u_start = start + idx as f64 * step;
        let u_end = if idx == total_units - 1 {
            end
        } else {
            start + (idx + 1) as f64 * step
        };
        unit_timings.push(UnitTiming {
            unit_id: u.id.clone(),
            start_time: u_start,
            end_time: u_end,
        });
    }

    Ok(CaptionTiming {
        timeline_id: "program.timeline".into(),
        document_id: doc.id.clone(),
        units: unit_timings,
    })
}

/// 核心排期调度：执行前置引导 (lead)、视觉驻留 (tail) 与硬切手递手 (cut handoff)
/// 词级高亮与动态追踪字幕排期算法 (scheduleFineCaption)
pub fn schedule_caption(
    doc: &CaptionDocument,
    timing: &CaptionTiming,
    recipe: &CaptionRecipe,
) -> Result<CaptionSchedule> {
    doc.assert_identity()?;

    let timing_map: HashMap<&str, &UnitTiming> = timing
        .units
        .iter()
        .map(|u| (u.unit_id.as_str(), u))
        .collect();
    let word_map: HashMap<&str, &CaptionWord> =
        doc.words.iter().map(|w| (w.id.as_str(), w)).collect();
    let unit_map: HashMap<&str, &CaptionUnit> =
        doc.units.iter().map(|u| (u.id.as_str(), u)).collect();

    // 1. 计算每个 Cue 的语义起始与结束时间 (timed_start, timed_end)
    let mut raw_cues = Vec::new();

    for cue in &doc.cues {
        let mut min_start = f64::MAX;
        let mut max_end = f64::MIN;
        let mut scheduled_words = Vec::new();
        let mut display_text = String::new();

        for u_id in &cue.unit_ids {
            let u = unit_map.get(u_id.as_str()).context("Unit lookup failed")?;
            let u_t = timing_map
                .get(u_id.as_str())
                .context("Unit timing lookup failed")?;

            min_start = min_start.min(u_t.start_time);
            max_end = max_end.max(u_t.end_time);

            // 将 Unit 内的 words 均分该 unit 的时长
            let word_count = u.word_ids.len();
            let unit_dur = (u_t.end_time - u_t.start_time).max(0.01);
            let word_step = unit_dur / word_count as f64;

            for (w_idx, w_id) in u.word_ids.iter().enumerate() {
                let w = word_map.get(w_id.as_str()).context("Word lookup failed")?;
                let w_start = u_t.start_time + w_idx as f64 * word_step;
                let w_end = if w_idx == word_count - 1 {
                    u_t.end_time
                } else {
                    w_start + word_step
                };

                let is_keyword = w.attributes.iter().any(|attr| {
                    attr.eq_ignore_ascii_case("keyword")
                        || attr.eq_ignore_ascii_case("highlight")
                        || attr.eq_ignore_ascii_case("gold")
                });

                display_text.push_str(&w.separator_before);
                display_text.push_str(&w.text);

                scheduled_words.push(ScheduledWord {
                    id: w.id.clone(),
                    text: w.text.clone(),
                    separator_before: w.separator_before.clone(),
                    start_time: w_start,
                    end_time: w_end,
                    is_keyword,
                });
            }
        }

        if min_start > max_end {
            min_start = 0.0;
            max_end = 0.1;
        }

        raw_cues.push((
            cue.id.clone(),
            cue.role.clone(),
            min_start,
            max_end,
            scheduled_words,
            display_text,
        ));
    }

    // 2. 调度可见时间窗口 (visible_start, visible_end)
    // 保护视线引导（lead）与驻留（tail），相邻 Cue 出现交叠且 handoff == "cut" 时进行硬切截断
    let mut scheduled_cues = Vec::new();
    let cue_count = raw_cues.len();

    for i in 0..cue_count {
        let (cue_id, role, timed_start, timed_end, words, display_text) = &raw_cues[i];

        let mut visible_start = (timed_start - recipe.envelope.lead_seconds).max(0.0);
        let mut visible_end = timed_end + recipe.envelope.tail_seconds;

        // 若前一个 cue 的 visible_end 延伸到了当前 cue 的 visible_start，且 cut handoff，则前一个的结束被硬切
        if i > 0 && recipe.envelope.handoff == "cut" {
            let prev_timed_end = raw_cues[i - 1].3;
            // 当前 cue 的 visible_start 不能侵占前一个 cue 的发音区间
            if visible_start < prev_timed_end {
                visible_start = prev_timed_end;
            }
        }

        // 若后一个 cue 存在，当前 cue 的 visible_end 不能超过后一个 cue 的发音开始
        if i + 1 < cue_count && recipe.envelope.handoff == "cut" {
            let next_timed_start = raw_cues[i + 1].2;
            if visible_end > next_timed_start {
                visible_end = next_timed_start;
            }
        }

        // 保证 visible_end >= visible_start
        if visible_end < visible_start {
            visible_end = visible_start + 0.05;
        }

        scheduled_cues.push(ScheduledCue {
            cue_id: cue_id.clone(),
            role: role.clone(),
            timed_start: *timed_start,
            timed_end: *timed_end,
            visible_start,
            visible_end,
            words: words.clone(),
            display_text: display_text.clone(),
            tracking_anchor: None,
        });
    }

    Ok(CaptionSchedule {
        timeline_id: timing.timeline_id.clone(),
        recipe_id: recipe.id.clone(),
        cues: scheduled_cues,
    })
}

/// 应用动态人头追踪，计算每个 Cue 对应发言人头部的 (x, y) 归一化中心点
pub fn apply_head_tracking(
    schedule: &mut CaptionSchedule,
    observations: &[FaceObservation],
    config: &HeadTrackingConfig,
) {
    if !config.enabled || observations.is_empty() {
        return;
    }

    for cue in &mut schedule.cues {
        // 筛选匹配发言角色并在时间窗口内的面部观测
        let relevant: Vec<&FaceObservation> = observations
            .iter()
            .filter(|obs| {
                obs.role.eq_ignore_ascii_case(&config.target_role)
                    && obs.timestamp >= cue.timed_start - 0.2
                    && obs.timestamp <= cue.timed_end + 0.2
            })
            .collect();

        if !relevant.is_empty() {
            // 取时间窗口内观测框的平均中心
            let mut sum_x = 0.0;
            let mut sum_y = 0.0;
            for obs in &relevant {
                let center_x = obs.box_normalized[0] + obs.box_normalized[2] / 2.0;
                let top_y = obs.box_normalized[1];
                let anchor_y =
                    (top_y - config.offset_y).clamp(config.clamp_y_min, config.clamp_y_max);
                sum_x += center_x;
                sum_y += anchor_y;
            }
            let n = relevant.len() as f64;
            cue.tracking_anchor = Some((sum_x / n, sum_y / n));
        }
    }
}

// ============================================================================
// 7. 导出引擎: ASS / WebVTT / HTML Web-as-Video
// ============================================================================

/// 导出为 ASS (Advanced SubStation Alpha) 字幕文件，带完整 Karaoke 标签与精确排版样式
pub fn export_to_ass(
    schedule: &CaptionSchedule,
    recipe: &CaptionRecipe,
    canvas_w: u32,
    canvas_h: u32,
) -> String {
    let mut out = String::new();

    // 1. [Script Info]
    out.push_str("[Script Info]\n");
    out.push_str("Title: clone-any-video Master Captions\n");
    out.push_str("ScriptType: v4.00+\n");
    out.push_str(&format!("PlayResX: {}\n", canvas_w));
    out.push_str(&format!("PlayResY: {}\n", canvas_h));
    out.push_str("WrapStyle: 0\n");
    out.push_str("ScaledBorderAndShadow: yes\n\n");

    // 2. [V4+ Styles]
    out.push_str("[V4+ Styles]\n");
    out.push_str("Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n");

    // ASS 颜色格式是 &HAABBGGRR (Alpha, Blue, Green, Red)
    let ass_primary = parse_hex_color_to_ass(&recipe.typography.fill_color, "00");
    let ass_karaoke = parse_hex_color_to_ass(&recipe.karaoke.active_fill, "00");
    let ass_outline = parse_hex_color_to_ass(&recipe.typography.stroke_color, "00");
    let ass_shadow = parse_hex_color_to_ass("#000000", "50");

    let margin_v = ((1.0 - recipe.placement.y) * canvas_h as f64) as i32;
    let margin_h = ((1.0 - recipe.placement.width) / 2.0 * canvas_w as f64) as i32;

    out.push_str(&format!(
        "Style: Default,{},{:.1},{},{},{},{},-1,0,0,0,100,100,0,0,1,{:.1},{:.1},2,{},{},{},1\n\n",
        recipe
            .typography
            .font_family
            .split(',')
            .next()
            .unwrap_or("Arial")
            .trim(),
        recipe.typography.font_size_px,
        ass_primary,
        ass_karaoke,
        ass_outline,
        ass_shadow,
        recipe.typography.stroke_width_px,
        recipe.typography.shadow_blur_px.min(4.0),
        margin_h.max(20),
        margin_h.max(20),
        margin_v.max(30),
    ));

    // 3. [Events]
    out.push_str("[Events]\n");
    out.push_str(
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
    );

    for cue in &schedule.cues {
        let start_str = format_ass_time(cue.visible_start);
        let end_str = format_ass_time(cue.visible_end);

        let mut dialogue_text = String::new();

        // 如果启用了动态人头追踪，添加 {\pos(x, y)} 覆盖标签
        if let Some((track_x, track_y)) = cue.tracking_anchor {
            let px_x = (track_x * canvas_w as f64) as i32;
            let px_y = (track_y * canvas_h as f64) as i32;
            dialogue_text.push_str(&format!("{{\\pos({},{})}}", px_x, px_y));
        }

        // 组装 Karaoke 标签 {\k<分秒>}
        for w in &cue.words {
            dialogue_text.push_str(&w.separator_before);
            let dur_cs = (((w.end_time - w.start_time).max(0.02)) * 100.0).round() as u32;

            if recipe.karaoke.mode != KaraokeMode::None {
                dialogue_text.push_str(&format!("{{\\k{}}}", dur_cs));
            }
            if w.is_keyword {
                dialogue_text.push_str(&format!(
                    "{{\\c{}}}{}{{\\c{}}}",
                    ass_karaoke, w.text, ass_primary
                ));
            } else {
                dialogue_text.push_str(&w.text);
            }
        }

        out.push_str(&format!(
            "Dialogue: 0,{},{},Default,{},0,0,0,,{}\n",
            start_str, end_str, cue.role, dialogue_text
        ));
    }

    out
}

/// 导出为标准 WebVTT 字幕
pub fn export_to_webvtt(schedule: &CaptionSchedule) -> String {
    let mut out = String::new();
    out.push_str("WEBVTT - clone-any-video Caption Track\n\n");

    for (idx, cue) in schedule.cues.iter().enumerate() {
        let start_str = format_vtt_time(cue.visible_start);
        let end_str = format_vtt_time(cue.visible_end);

        out.push_str(&format!("{}\n", idx + 1));
        out.push_str(&format!("{} --> {}\n", start_str, end_str));

        let mut line_text = String::new();
        for w in &cue.words {
            line_text.push_str(&w.separator_before);
            let w_start = format_vtt_time(w.start_time);
            line_text.push_str(&format!("<{}>{}", w_start, w.text));
        }
        out.push_str(&format!("{}\n\n", line_text.trim()));
    }

    out
}

/// 导出为现代化 Web-as-Video 动态 HTML/CSS 视听组件
/// 内置 JavaScript `window.renderCaptionFrame(timeInSeconds)`，供无头 Chromium 逐帧调用
pub fn export_to_html_overlay(
    schedule: &CaptionSchedule,
    recipe: &CaptionRecipe,
    canvas_w: u32,
    canvas_h: u32,
) -> String {
    let schedule_json = serde_json::to_string(schedule)
        .unwrap_or_else(|_| "{}".into())
        .replace('<', "\\u003c");
    let recipe_json = serde_json::to_string(recipe)
        .unwrap_or_else(|_| "{}".into())
        .replace('<', "\\u003c");

    format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<title>Dynamic Caption Component</title>
<style>
  * {{ box-sizing: border-box; }}
  html, body {{
    margin: 0;
    padding: 0;
    width: {width}px;
    height: {height}px;
    background: transparent;
    overflow: hidden;
    font-family: {font_family};
    -webkit-font-smoothing: antialiased;
  }}
  #caption-canvas {{
    position: relative;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }}
  .cue-box {{
    position: absolute;
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: center;
    width: {max_width_pct}%;
    max-width: {max_width_pct}%;
    text-align: center;
    font-size: {font_size}px;
    font-weight: {font_weight};
    line-height: {line_height};
    color: {fill_color};
    text-shadow: 0 2px 8px {shadow_color}, 0 0 {stroke_width}px {stroke_color};
    padding: 8px 18px;
    border-radius: 12px;
    transition: none;
    opacity: 0;
    transform: translate(-50%, -100%) scale(0.96);
  }}
  .cue-box.visible {{
    opacity: 1;
    transform: translate(-50%, -100%) scale(1.0);
  }}
  .word-atom {{
    display: inline-block;
    position: relative;
    white-space: pre;
    transition: none;
  }}
  .word-atom.active {{
    color: {active_fill};
    transform: scale({active_scale});
    text-shadow: 0 0 16px {active_fill}, 0 2px 10px rgba(0,0,0,0.8);
  }}
  .word-atom.trail {{
    color: {active_fill};
  }}
  .word-atom.keyword {{
    font-weight: 900;
    text-decoration: underline;
    text-underline-offset: 6px;
  }}
  .active-bg-pill {{
    background: {active_box_color};
    border-radius: 6px;
    padding: 2px 6px;
  }}
</style>
</head>
<body>
<div id="caption-canvas">
  <div id="current-cue" class="cue-box"></div>
</div>

<script>
  const SCHEDULE = {schedule_json};
  const RECIPE = {recipe_json};
  const CANVAS_W = {width};
  const CANVAS_H = {height};

  const container = document.getElementById('current-cue');

  window.renderCaptionFrame = function(currentTime) {{
    const cue = SCHEDULE.cues.find(c => currentTime >= c.visible_start && currentTime < c.visible_end);
    if (!cue) {{
      container.className = 'cue-box';
      container.innerHTML = '';
      return;
    }}

    // 确定定位坐标 (人头追踪优先，否则使用 Recipe 默认 placement)
    let normX = RECIPE.placement.x;
    let normY = RECIPE.placement.y;
    if (cue.tracking_anchor) {{
      normX = cue.tracking_anchor[0];
      normY = cue.tracking_anchor[1];
    }}

    const pxX = Math.round(normX * CANVAS_W);
    const pxY = Math.round(normY * CANVAS_H);

    container.style.left = pxX + 'px';
    container.style.top = pxY + 'px';
    container.className = 'cue-box visible';

    // 渲染词级别 Karaoke 状态
    let html = '';
    for (let i = 0; i < cue.words.length; i++) {{
      const w = cue.words[i];
      let cls = 'word-atom';
      if (w.is_keyword) cls += ' keyword';

      if (RECIPE.karaoke.mode !== 'none' && currentTime >= w.start_time && currentTime < w.end_time) {{
        cls += ' active' + (RECIPE.karaoke.active_box ? ' active-bg-pill' : '');
      }} else if (currentTime >= w.end_time && RECIPE.karaoke.mode === 'trail') {{
        cls += ' trail';
      }}

      const escape = s => s.replace(/[&<>"']/g, c => ({{'&':'&amp;','<':'&lt;','>':'&gt;','\"':'&quot;',"'":'&#39;'}}[c]));
      html += `<span class="${{cls}}">${{escape(w.separator_before)}}${{escape(w.text)}}</span>`;
    }}
    container.innerHTML = html;
  }};
</script>
</body>
</html>"#,
        width = canvas_w,
        height = canvas_h,
        font_family = recipe.typography.font_family,
        max_width_pct = (recipe.placement.width * 100.0).round(),
        font_size = recipe.typography.font_size_px,
        font_weight = recipe.typography.font_weight,
        line_height = recipe.typography.line_height,
        fill_color = recipe.typography.fill_color,
        stroke_color = recipe.typography.stroke_color,
        stroke_width = recipe.typography.stroke_width_px,
        shadow_color = recipe.typography.shadow_color,
        active_fill = recipe.karaoke.active_fill,
        active_scale = recipe.karaoke.active_scale,
        active_box_color = recipe.karaoke.active_box_color,
        schedule_json = schedule_json,
        recipe_json = recipe_json,
    )
}

// ============================================================================
// 8. 实时时间戳状态查询快照 (State Query Snapshot)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptionStateSnapshot {
    pub timestamp: f64,
    pub cue_id: String,
    pub role: String,
    pub display_text: String,
    pub active_word_index: Option<usize>,
    pub active_word_text: Option<String>,
    pub position_px: (f64, f64),
    pub is_tracking_head: bool,
}

pub fn query_caption_state_at(
    schedule: &CaptionSchedule,
    recipe: &CaptionRecipe,
    timestamp: f64,
    canvas_w: u32,
    canvas_h: u32,
) -> Option<CaptionStateSnapshot> {
    for cue in &schedule.cues {
        if timestamp >= cue.visible_start && timestamp < cue.visible_end {
            let mut active_word_idx = None;
            let mut active_word_str = None;

            for (idx, w) in cue.words.iter().enumerate() {
                if timestamp >= w.start_time && timestamp < w.end_time {
                    active_word_idx = Some(idx);
                    active_word_str = Some(w.text.clone());
                    break;
                }
            }

            let (norm_x, norm_y, is_tracking) = if let Some((tx, ty)) = cue.tracking_anchor {
                (tx, ty, true)
            } else {
                (recipe.placement.x, recipe.placement.y, false)
            };

            return Some(CaptionStateSnapshot {
                timestamp,
                cue_id: cue.cue_id.clone(),
                role: cue.role.clone(),
                display_text: cue.display_text.clone(),
                active_word_index: active_word_idx,
                active_word_text: active_word_str,
                position_px: (norm_x * canvas_w as f64, norm_y * canvas_h as f64),
                is_tracking_head: is_tracking,
            });
        }
    }
    None
}

// ============================================================================
// 辅助格式化函数
// ============================================================================

fn format_ass_time(sec: f64) -> String {
    let total_cs = (sec * 100.0).max(0.0).round() as u64;
    let cs = total_cs % 100;
    let total_s = total_cs / 100;
    let s = total_s % 60;
    let total_m = total_s / 60;
    let m = total_m % 60;
    let h = total_m / 60;
    format!("{}:{:02}:{:02}.{:02}", h, m, s, cs)
}

fn format_vtt_time(sec: f64) -> String {
    let total_ms = (sec * 1000.0).max(0.0).round() as u64;
    let ms = total_ms % 1000;
    let total_s = total_ms / 1000;
    let s = total_s % 60;
    let total_m = total_s / 60;
    let m = total_m % 60;
    let h = total_m / 60;
    format!("{:02}:{:02}:{:02}.{:03}", h, m, s, ms)
}

fn parse_hex_color_to_ass(hex: &str, alpha_hex: &str) -> String {
    let clean = hex.trim().trim_start_matches('#');
    if clean.len() == 6 {
        let r = &clean[0..2];
        let g = &clean[2..4];
        let b = &clean[4..6];
        // ASS format: &HAABBGGRR
        format!("&H{}{}{}{}", alpha_hex, b, g, r)
    } else {
        format!("&H{}FFFFFF", alpha_hex)
    }
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_caption_document_identity_and_partitioning() {
        let doc = CaptionDocument {
            id: "test.caption".into(),
            words: vec![
                CaptionWord {
                    id: "w1".into(),
                    unit_id: "u1".into(),
                    text: "Hello".into(),
                    separator_before: "".into(),
                    attributes: vec![],
                    spoken: None,
                },
                CaptionWord {
                    id: "w2".into(),
                    unit_id: "u1".into(),
                    text: "World".into(),
                    separator_before: " ".into(),
                    attributes: vec!["keyword".into()],
                    spoken: None,
                },
                CaptionWord {
                    id: "w3".into(),
                    unit_id: "u2".into(),
                    text: "Rust".into(),
                    separator_before: " ".into(),
                    attributes: vec![],
                    spoken: None,
                },
            ],
            units: vec![
                CaptionUnit {
                    id: "u1".into(),
                    word_ids: vec!["w1".into(), "w2".into()],
                },
                CaptionUnit {
                    id: "u2".into(),
                    word_ids: vec!["w3".into()],
                },
            ],
            cues: vec![
                DocCue {
                    id: "c1".into(),
                    unit_ids: vec!["u1".into()],
                    role: "HOST".into(),
                },
                DocCue {
                    id: "c2".into(),
                    unit_ids: vec!["u2".into()],
                    role: "GUEST".into(),
                },
            ],
        };

        assert!(doc.assert_identity().is_ok());

        // 故意错乱顺序测试不变式拦截
        let mut broken_doc = doc.clone();
        broken_doc.cues[0].unit_ids = vec!["u2".into()];
        broken_doc.cues[1].unit_ids = vec!["u1".into()];
        assert!(broken_doc.assert_identity().is_err());
    }

    #[test]
    fn test_schedule_lead_tail_and_cut_handoff() {
        let doc = CaptionDocument {
            id: "test.caption".into(),
            words: vec![
                CaptionWord {
                    id: "w1".into(),
                    unit_id: "u1".into(),
                    text: "One".into(),
                    separator_before: "".into(),
                    attributes: vec![],
                    spoken: None,
                },
                CaptionWord {
                    id: "w2".into(),
                    unit_id: "u2".into(),
                    text: "Two".into(),
                    separator_before: " ".into(),
                    attributes: vec![],
                    spoken: None,
                },
            ],
            units: vec![
                CaptionUnit {
                    id: "u1".into(),
                    word_ids: vec!["w1".into()],
                },
                CaptionUnit {
                    id: "u2".into(),
                    word_ids: vec!["w2".into()],
                },
            ],
            cues: vec![
                DocCue {
                    id: "c1".into(),
                    unit_ids: vec!["u1".into()],
                    role: "HOST".into(),
                },
                DocCue {
                    id: "c2".into(),
                    unit_ids: vec!["u2".into()],
                    role: "HOST".into(),
                },
            ],
        };

        let timing = CaptionTiming {
            timeline_id: "test".into(),
            document_id: doc.id.clone(),
            units: vec![
                UnitTiming {
                    unit_id: "u1".into(),
                    start_time: 1.0,
                    end_time: 2.0,
                },
                UnitTiming {
                    unit_id: "u2".into(),
                    start_time: 2.1,
                    end_time: 3.0,
                },
            ],
        };

        let recipe = CaptionRecipe {
            envelope: TimingEnvelope {
                lead_seconds: 0.1,
                tail_seconds: 0.2,
                handoff: "cut".into(),
            },
            ..Default::default()
        };

        let schedule = schedule_caption(&doc, &timing, &recipe).expect("Schedule caption");
        assert_eq!(schedule.cues.len(), 2);

        // cue 1: timed [1.0, 2.0], visible [0.9, 2.1] (clipped by next cue timed_start 2.1)
        assert!((schedule.cues[0].visible_start - 0.9).abs() < 1e-4);
        assert!((schedule.cues[0].visible_end - 2.1).abs() < 1e-4);

        // cue 2: timed [2.1, 3.0], visible [2.0, 3.2] (lead clipped by prev timed_end 2.0)
        assert!((schedule.cues[1].visible_start - 2.0).abs() < 1e-4);
        assert!((schedule.cues[1].visible_end - 3.2).abs() < 1e-4);
        // Visibility and highlighting use half-open windows, including exact handoffs.
        let state = query_caption_state_at(&schedule, &recipe, 2.1, 640, 360).unwrap();
        assert_eq!(state.cue_id, "c2");
        assert!(query_caption_state_at(&schedule, &recipe, 3.2, 640, 360).is_none());
    }

    #[test]
    fn test_dynamic_head_tracking_anchor_assignment() {
        let doc = CaptionDocument {
            id: "test.caption".into(),
            words: vec![CaptionWord {
                id: "w1".into(),
                unit_id: "u1".into(),
                text: "Speaking".into(),
                separator_before: "".into(),
                attributes: vec![],
                spoken: None,
            }],
            units: vec![CaptionUnit {
                id: "u1".into(),
                word_ids: vec!["w1".into()],
            }],
            cues: vec![DocCue {
                id: "c1".into(),
                unit_ids: vec!["u1".into()],
                role: "HOST".into(),
            }],
        };

        let timing = CaptionTiming {
            timeline_id: "test".into(),
            document_id: doc.id.clone(),
            units: vec![UnitTiming {
                unit_id: "u1".into(),
                start_time: 5.0,
                end_time: 7.0,
            }],
        };

        let mut schedule = schedule_caption(&doc, &timing, &CaptionRecipe::default()).unwrap();

        let observations = vec![
            FaceObservation {
                timestamp: 5.5,
                role: "HOST".into(),
                box_normalized: [0.4, 0.2, 0.2, 0.2], // center x = 0.5, top y = 0.2
            },
            FaceObservation {
                timestamp: 6.5,
                role: "HOST".into(),
                box_normalized: [0.42, 0.22, 0.2, 0.2], // center x = 0.52, top y = 0.22
            },
        ];

        let config = HeadTrackingConfig {
            enabled: true,
            target_role: "HOST".into(),
            offset_y: 0.05,
            clamp_y_min: 0.05,
            clamp_y_max: 0.9,
        };

        apply_head_tracking(&mut schedule, &observations, &config);

        assert!(schedule.cues[0].tracking_anchor.is_some());
        let (ax, ay) = schedule.cues[0].tracking_anchor.unwrap();
        assert!((ax - 0.51).abs() < 1e-3);
        assert!((ay - (0.21 - 0.05)).abs() < 1e-3);
    }

    #[test]
    fn test_ass_and_webvtt_and_html_export() {
        let doc = CaptionDocument {
            id: "test.caption".into(),
            words: vec![
                CaptionWord {
                    id: "w1".into(),
                    unit_id: "u1".into(),
                    text: "Rust".into(),
                    separator_before: "".into(),
                    attributes: vec!["keyword".into()],
                    spoken: None,
                },
                CaptionWord {
                    id: "w2".into(),
                    unit_id: "u1".into(),
                    text: "Engine".into(),
                    separator_before: " ".into(),
                    attributes: vec![],
                    spoken: None,
                },
            ],
            units: vec![CaptionUnit {
                id: "u1".into(),
                word_ids: vec!["w1".into(), "w2".into()],
            }],
            cues: vec![DocCue {
                id: "c1".into(),
                unit_ids: vec!["u1".into()],
                role: "HOST".into(),
            }],
        };

        let timing = CaptionTiming {
            timeline_id: "test".into(),
            document_id: doc.id.clone(),
            units: vec![UnitTiming {
                unit_id: "u1".into(),
                start_time: 0.5,
                end_time: 1.5,
            }],
        };

        let recipe = CaptionRecipe::default();
        let schedule = schedule_caption(&doc, &timing, &recipe).unwrap();

        // 1. ASS Export
        let ass = export_to_ass(&schedule, &recipe, 1920, 1080);
        assert!(ass.contains("[Script Info]"));
        assert!(ass.contains("[V4+ Styles]"));
        assert!(ass.contains("Dialogue: 0,0:00:00.40,0:00:01.70,Default,HOST,0,0,0,,{\\k50}"));

        // 2. WebVTT Export
        let vtt = export_to_webvtt(&schedule);
        assert!(vtt.contains("WEBVTT"));
        assert!(vtt.contains("00:00:00.400 --> 00:00:01.700"));

        // 3. HTML Export
        let html = export_to_html_overlay(&schedule, &recipe, 1920, 1080);
        assert!(html.contains("window.renderCaptionFrame"));
        assert!(html.contains("class=\"cue-box\""));

        // 4. Query state at timestamp
        let state = query_caption_state_at(&schedule, &recipe, 0.8, 1920, 1080);
        assert!(state.is_some());
        let st = state.unwrap();
        assert_eq!(st.cue_id, "c1");
        assert_eq!(st.active_word_text.as_deref(), Some("Rust"));
    }

    #[test]
    fn test_caption_alignment_from_transcript_with_uneven_pause() {
        let doc = CaptionDocument {
            id: "pause_doc".into(),
            words: vec![
                CaptionWord {
                    id: "w1".into(),
                    unit_id: "u1".into(),
                    text: "Hello".into(),
                    separator_before: "".into(),
                    attributes: vec![],
                    spoken: None,
                },
                CaptionWord {
                    id: "w2".into(),
                    unit_id: "u2".into(),
                    text: "World".into(),
                    separator_before: " ".into(),
                    attributes: vec![],
                    spoken: None,
                },
            ],
            units: vec![
                CaptionUnit {
                    id: "u1".into(),
                    word_ids: vec!["w1".into()],
                },
                CaptionUnit {
                    id: "u2".into(),
                    word_ids: vec!["w2".into()],
                },
            ],
            cues: vec![DocCue {
                id: "c1".into(),
                unit_ids: vec!["u1".into(), "u2".into()],
                role: "HOST".into(),
            }],
        };

        // Transcript has a 2-second silence between Hello and World
        let transcript = crate::transcribe::TranscriptDocument {
            evidence: serde_json::Value::Null,
            format: "test".into(),
            source: "test.wav".into(),
            language: "en".into(),
            audio_seconds: 5.0,
            passages: vec![crate::transcribe::TranscriptPassage {
                text: "Hello World".into(),
                start_seconds: Some(0.2),
                end_seconds: Some(3.5),
                words: vec![
                    crate::transcribe::TranscriptWord {
                        text: "Hello".into(),
                        start_seconds: Some(0.2),
                        end_seconds: Some(0.8),
                        score: Some(0.99),
                    },
                    // 2-second silence here!
                    crate::transcribe::TranscriptWord {
                        text: "World".into(),
                        start_seconds: Some(2.8),
                        end_seconds: Some(3.5),
                        score: Some(0.98),
                    },
                ],
            }],
        };

        let timing = build_caption_timing_from_transcript(&doc, &transcript).unwrap();
        assert_eq!(timing.units.len(), 2);
        // u1 anchors to Hello: 0.2 - 0.8
        assert!((timing.units[0].start_time - 0.2).abs() < 1e-4);
        assert!((timing.units[0].end_time - 0.8).abs() < 1e-4);
        // u2 anchors to World: 2.8 - 3.5, NOT linearly split at ~1.7s!
        assert!((timing.units[1].start_time - 2.8).abs() < 1e-4);
        assert!((timing.units[1].end_time - 3.5).abs() < 1e-4);
    }

    #[test]
    fn test_caption_alignment_substring_avoidance() {
        let doc = CaptionDocument {
            id: "substring_doc".into(),
            words: vec![
                CaptionWord {
                    id: "w1".into(),
                    unit_id: "u1".into(),
                    text: "a".into(),
                    separator_before: "".into(),
                    attributes: vec![],
                    spoken: None,
                },
                CaptionWord {
                    id: "w2".into(),
                    unit_id: "u2".into(),
                    text: "cat".into(),
                    separator_before: " ".into(),
                    attributes: vec![],
                    spoken: None,
                },
            ],
            units: vec![
                CaptionUnit {
                    id: "u1".into(),
                    word_ids: vec!["w1".into()],
                },
                CaptionUnit {
                    id: "u2".into(),
                    word_ids: vec!["w2".into()],
                },
            ],
            cues: vec![DocCue {
                id: "cue".into(),
                unit_ids: vec!["u1".into(), "u2".into()],
                role: "HOST".into(),
            }],
        };

        let transcript = crate::transcribe::TranscriptDocument {
            evidence: serde_json::Value::Null,
            format: "clone-any-video.transcript@1".into(),
            source: "audio.wav".into(),
            language: "en".into(),
            audio_seconds: 4.0,
            passages: vec![crate::transcribe::TranscriptPassage {
                text: "that a cat".into(),
                start_seconds: Some(0.0),
                end_seconds: Some(3.3),
                words: vec![
                    crate::transcribe::TranscriptWord {
                        text: "that".into(),
                        start_seconds: Some(0.0),
                        end_seconds: Some(0.4),
                        score: Some(0.99),
                    },
                    crate::transcribe::TranscriptWord {
                        text: "a".into(),
                        start_seconds: Some(2.0),
                        end_seconds: Some(2.3),
                        score: Some(0.99),
                    },
                    crate::transcribe::TranscriptWord {
                        text: "cat".into(),
                        start_seconds: Some(3.0),
                        end_seconds: Some(3.3),
                        score: Some(0.99),
                    },
                ],
            }],
        };

        let timing = build_caption_timing_from_transcript(&doc, &transcript).unwrap();
        assert_eq!(timing.units.len(), 2);
        // 'a' must match 'a' at 2.0s, NOT 'that' at 0.0s!
        assert!((timing.units[0].start_time - 2.0).abs() < 1e-4);
        assert!((timing.units[0].end_time - 2.3).abs() < 1e-4);
        // 'cat' must match 'cat' at 3.0s, NOT 'a' at 2.0s!
        assert!((timing.units[1].start_time - 3.0).abs() < 1e-4);
        assert!((timing.units[1].end_time - 3.3).abs() < 1e-4);
    }

    #[test]
    fn test_caption_alignment_dual_text_spoken() {
        let doc = CaptionDocument {
            id: "dual_doc".into(),
            words: vec![
                CaptionWord {
                    id: "w1".into(),
                    unit_id: "u1".into(),
                    text: "NYC".into(),
                    separator_before: "".into(),
                    attributes: vec![],
                    spoken: Some("New York".into()),
                },
                CaptionWord {
                    id: "w2".into(),
                    unit_id: "u2".into(),
                    text: "wins.".into(),
                    separator_before: " ".into(),
                    attributes: vec![],
                    spoken: None,
                },
            ],
            units: vec![
                CaptionUnit {
                    id: "u1".into(),
                    word_ids: vec!["w1".into()],
                },
                CaptionUnit {
                    id: "u2".into(),
                    word_ids: vec!["w2".into()],
                },
            ],
            cues: vec![DocCue {
                id: "cue".into(),
                unit_ids: vec!["u1".into(), "u2".into()],
                role: "HOST".into(),
            }],
        };

        let transcript = crate::transcribe::TranscriptDocument {
            evidence: serde_json::Value::Null,
            format: "clone-any-video.transcript@1".into(),
            source: "audio.wav".into(),
            language: "en".into(),
            audio_seconds: 4.0,
            passages: vec![crate::transcribe::TranscriptPassage {
                text: "New York wins.".into(),
                start_seconds: Some(0.0),
                end_seconds: Some(3.5),
                words: vec![
                    crate::transcribe::TranscriptWord {
                        text: "New".into(),
                        start_seconds: Some(0.1),
                        end_seconds: Some(0.5),
                        score: Some(0.99),
                    },
                    crate::transcribe::TranscriptWord {
                        text: "York".into(),
                        start_seconds: Some(0.6),
                        end_seconds: Some(1.0),
                        score: Some(0.99),
                    },
                    crate::transcribe::TranscriptWord {
                        text: "wins".into(),
                        start_seconds: Some(3.0),
                        end_seconds: Some(3.5),
                        score: Some(0.99),
                    },
                ],
            }],
        };

        let timing = build_caption_timing_from_transcript(&doc, &transcript).unwrap();
        assert_eq!(timing.units.len(), 2);
        // 'NYC' with spoken 'New York' spans 0.1 to 1.0
        assert!((timing.units[0].start_time - 0.1).abs() < 1e-4);
        assert!((timing.units[0].end_time - 1.0).abs() < 1e-4);
        // 'wins.' matches 'wins' at 3.0 to 3.5
        assert!((timing.units[1].start_time - 3.0).abs() < 1e-4);
        assert!((timing.units[1].end_time - 3.5).abs() < 1e-4);
    }
}
