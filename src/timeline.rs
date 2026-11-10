use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::script::ScriptDocument;

/// Time duration representation supporting frames, milliseconds, and seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum TimeDuration {
    Frames(u64),
    Milliseconds(u64),
    Seconds(f64),
}

impl TimeDuration {
    /// Parse standard duration strings like "12f", "250ms", "1.5s", "3s".
    #[allow(dead_code)]
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        if let Some(num) = s.strip_suffix('f') {
            let frames: u64 = num.parse().context("Invalid frame count")?;
            Ok(TimeDuration::Frames(frames))
        } else if let Some(num) = s.strip_suffix("ms") {
            let ms: u64 = num.parse().context("Invalid milliseconds")?;
            Ok(TimeDuration::Milliseconds(ms))
        } else if let Some(num) = s.strip_suffix('s') {
            let sec: f64 = num.parse().context("Invalid seconds")?;
            Ok(TimeDuration::Seconds(sec))
        } else {
            // Default to seconds if plain float/int
            let sec: f64 = s.parse().context("Invalid duration format (expected e.g. '12f', '250ms', '1.5s')")?;
            Ok(TimeDuration::Seconds(sec))
        }
    }

    pub fn to_seconds(&self, fps: f64) -> f64 {
        match self {
            TimeDuration::Frames(f) => (*f as f64) / fps,
            TimeDuration::Milliseconds(ms) => (*ms as f64) / 1000.0,
            TimeDuration::Seconds(s) => *s,
        }
    }

    #[allow(dead_code)]
    pub fn to_frames(&self, fps: f64) -> u64 {
        match self {
            TimeDuration::Frames(f) => *f,
            TimeDuration::Milliseconds(ms) => ((*ms as f64 / 1000.0) * fps).round() as u64,
            TimeDuration::Seconds(s) => (*s * fps).round() as u64,
        }
    }
}

/// Point expression in temporal domain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PointExpr {
    Start,
    End,
    NamedPoint(String),
    Offset(Box<PointExpr>, TimeDuration, bool /* is_add */),
    Earliest(Vec<PointExpr>),
    Latest(Vec<PointExpr>),
}

impl PointExpr {
    #[allow(dead_code)]
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("start") {
            return Ok(PointExpr::Start);
        }
        if s.eq_ignore_ascii_case("end") {
            return Ok(PointExpr::End);
        }

        // Functions: latest(...) or earliest(...)
        if let Some(inner) = s.strip_prefix("latest(").and_then(|x| x.strip_suffix(')')) {
            let mut list = Vec::new();
            for part in inner.split(',') {
                list.push(PointExpr::parse(part)?);
            }
            return Ok(PointExpr::Latest(list));
        }
        if let Some(inner) = s.strip_prefix("earliest(").and_then(|x| x.strip_suffix(')')) {
            let mut list = Vec::new();
            for part in inner.split(',') {
                list.push(PointExpr::parse(part)?);
            }
            return Ok(PointExpr::Earliest(list));
        }

        // Offset: base + duration or base - duration
        if let Some(idx) = s.rfind('+') {
            let (left, right) = s.split_at(idx);
            let base = PointExpr::parse(left)?;
            let dur = TimeDuration::parse(&right[1..])?;
            return Ok(PointExpr::Offset(Box::new(base), dur, true));
        }
        if let Some(idx) = s.rfind('-') {
            // Ensure not a negative number
            let (left, right) = s.split_at(idx);
            if !left.trim().is_empty() {
                let base = PointExpr::parse(left)?;
                let dur = TimeDuration::parse(&right[1..])?;
                return Ok(PointExpr::Offset(Box::new(base), dur, false));
            }
        }

        // Named point or literal identifier
        if s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' || c == ':') {
            Ok(PointExpr::NamedPoint(s.to_string()))
        } else {
            bail!("Cannot parse point expression: '{}'", s);
        }
    }
}

/// Declaration of a Window (temporal span).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowDecl {
    pub id: String,
    pub from: Option<PointExpr>,
    pub until: Option<PointExpr>,
    pub for_duration: Option<TimeDuration>,
    pub label: Option<String>,
}

/// Declaration of an Instant (zero-width point in time).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstantDecl {
    pub id: String,
    pub at: PointExpr,
    pub label: Option<String>,
}

/// Complete declarative timeline manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineManifest {
    pub id: String,
    pub frame_rate: f64,
    pub windows: Vec<WindowDecl>,
    pub instants: Vec<InstantDecl>,
    pub end_expr: Option<PointExpr>,
}

/// Fully resolved temporal window with exact seconds and frames.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedWindow {
    pub id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub start_frame: u64,
    pub end_frame: u64,
    pub duration_seconds: f64,
    pub duration_frames: u64,
    pub label: Option<String>,
}

/// Fully resolved instant with exact second and frame.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedInstant {
    pub id: String,
    pub at_seconds: f64,
    pub at_frame: u64,
    pub label: Option<String>,
}

/// Complete resolved timeline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedTimeline {
    pub id: String,
    pub frame_rate: f64,
    pub total_seconds: f64,
    pub total_frames: u64,
    pub windows: Vec<ResolvedWindow>,
    pub instants: Vec<ResolvedInstant>,
}

/// Dynamic Timeline Graph Solver
pub struct TimelineSolver {
    manifest: TimelineManifest,
    resolved_points: HashMap<String, f64>,
    eval_stack: HashSet<String>,
}

impl TimelineSolver {
    pub fn new(manifest: TimelineManifest) -> Self {
        let mut resolved = HashMap::new();
        resolved.insert("start".to_string(), 0.0);
        Self {
            manifest,
            resolved_points: resolved,
            eval_stack: HashSet::new(),
        }
    }

    /// Resolve all declarations into a deterministic timeline.
    pub fn solve(mut self) -> Result<ResolvedTimeline> {
        let fps = self.manifest.frame_rate;
        if fps <= 0.0 {
            bail!("Frame rate must be positive, got {}", fps);
        }

        // First pass: resolve window boundaries
        let windows_len = self.manifest.windows.len();
        for i in 0..windows_len {
            let win = self.manifest.windows[i].clone();
            self.resolve_window_points(&win)?;
        }

        // Second pass: resolve instants
        let instants_len = self.manifest.instants.len();
        for i in 0..instants_len {
            let inst = self.manifest.instants[i].clone();
            let key = inst.id.clone();
            if !self.resolved_points.contains_key(&key) {
                let t = self.resolve_point_expr(&inst.at)?;
                self.resolved_points.insert(key, t);
            }
        }

        // Third pass: determine program end
        let end_opt = self.manifest.end_expr.clone();
        let end_seconds = if let Some(ref end_expr) = end_opt {
            self.resolve_point_expr(end_expr)?
        } else {
            // Default: latest of all window ends and instants
            let mut max_t = 0.0;
            for win in &self.manifest.windows {
                let end_key = format!("{}.end", win.id);
                if let Some(&t) = self.resolved_points.get(&end_key) {
                    if t > max_t {
                        max_t = t;
                    }
                }
            }
            for inst in &self.manifest.instants {
                if let Some(&t) = self.resolved_points.get(&inst.id) {
                    if t > max_t {
                        max_t = t;
                    }
                }
            }
            max_t
        };
        self.resolved_points.insert("end".to_string(), end_seconds);

        // Build final resolved items
        let mut resolved_windows = Vec::new();
        for win in &self.manifest.windows {
            let start_t = *self.resolved_points.get(&format!("{}.start", win.id))
                .ok_or_else(|| anyhow!("Unresolved start for window '{}'", win.id))?;
            let end_t = *self.resolved_points.get(&format!("{}.end", win.id))
                .ok_or_else(|| anyhow!("Unresolved end for window '{}'", win.id))?;
            let dur_t = (end_t - start_t).max(0.0);

            let start_frame = (start_t * fps).round() as u64;
            let end_frame = (end_t * fps).round() as u64;
            let dur_frames = end_frame.saturating_sub(start_frame);

            resolved_windows.push(ResolvedWindow {
                id: win.id.clone(),
                start_seconds: (start_t * 1000.0).round() / 1000.0,
                end_seconds: (end_t * 1000.0).round() / 1000.0,
                start_frame,
                end_frame,
                duration_seconds: (dur_t * 1000.0).round() / 1000.0,
                duration_frames: dur_frames,
                label: win.label.clone(),
            });
        }

        let mut resolved_instants = Vec::new();
        for inst in &self.manifest.instants {
            let t = *self.resolved_points.get(&inst.id)
                .ok_or_else(|| anyhow!("Unresolved instant '{}'", inst.id))?;
            let frame = (t * fps).round() as u64;
            resolved_instants.push(ResolvedInstant {
                id: inst.id.clone(),
                at_seconds: (t * 1000.0).round() / 1000.0,
                at_frame: frame,
                label: inst.label.clone(),
            });
        }

        // Sort by start time
        resolved_windows.sort_by(|a, b| a.start_seconds.partial_cmp(&b.start_seconds).unwrap());
        resolved_instants.sort_by(|a, b| a.at_seconds.partial_cmp(&b.at_seconds).unwrap());

        let total_frames = (end_seconds * fps).round() as u64;

        Ok(ResolvedTimeline {
            id: self.manifest.id.clone(),
            frame_rate: fps,
            total_seconds: (end_seconds * 1000.0).round() / 1000.0,
            total_frames,
            windows: resolved_windows,
            instants: resolved_instants,
        })
    }

    fn resolve_window_points(&mut self, win: &WindowDecl) -> Result<()> {
        let start_key = format!("{}.start", win.id);
        let end_key = format!("{}.end", win.id);
        let fps = self.manifest.frame_rate;

        // Case 1: from + for_duration
        if let (Some(from), Some(for_dur)) = (&win.from, &win.for_duration) {
            let start_t = self.resolve_point_expr(from)?;
            let dur_t = for_dur.to_seconds(fps);
            let end_t = start_t + dur_t;
            self.resolved_points.insert(start_key, start_t);
            self.resolved_points.insert(end_key, end_t);
            return Ok(());
        }

        // Case 2: from + until
        if let (Some(from), Some(until)) = (&win.from, &win.until) {
            let start_t = self.resolve_point_expr(from)?;
            let end_t = self.resolve_point_expr(until)?;
            self.resolved_points.insert(start_key, start_t);
            self.resolved_points.insert(end_key, end_t);
            return Ok(());
        }

        // Case 3: until + for_duration
        if let (Some(until), Some(for_dur)) = (&win.until, &win.for_duration) {
            let end_t = self.resolve_point_expr(until)?;
            let dur_t = for_dur.to_seconds(fps);
            let start_t = end_t - dur_t;
            self.resolved_points.insert(start_key, start_t);
            self.resolved_points.insert(end_key, end_t);
            return Ok(());
        }

        bail!("Window '{}' must specify exactly two of: from, until, for_duration", win.id);
    }

    fn resolve_point_expr(&mut self, expr: &PointExpr) -> Result<f64> {
        let fps = self.manifest.frame_rate;
        match expr {
            PointExpr::Start => Ok(0.0),
            PointExpr::End => {
                if let Some(&t) = self.resolved_points.get("end") {
                    Ok(t)
                } else {
                    bail!("Cannot resolve 'end' before program end is determined");
                }
            }
            PointExpr::NamedPoint(ref name) => {
                if let Some(&t) = self.resolved_points.get(name) {
                    return Ok(t);
                }

                // Cycle detection
                if self.eval_stack.contains(name) {
                    bail!("Cyclic dependency detected involving point '{}'", name);
                }
                self.eval_stack.insert(name.clone());

                // Try to resolve based on window
                let res = if let Some(win_id) = name.strip_suffix(".start") {
                    let win = self.manifest.windows.iter().find(|w| w.id == win_id)
                        .cloned().ok_or_else(|| anyhow!("Window '{}' not found", win_id))?;
                    self.resolve_window_points(&win)?;
                    *self.resolved_points.get(name).unwrap()
                } else if let Some(win_id) = name.strip_suffix(".end") {
                    let win = self.manifest.windows.iter().find(|w| w.id == win_id)
                        .cloned().ok_or_else(|| anyhow!("Window '{}' not found", win_id))?;
                    self.resolve_window_points(&win)?;
                    *self.resolved_points.get(name).unwrap()
                } else if let Some(inst) = self.manifest.instants.iter().find(|i| i.id == *name).cloned() {
                    let t = self.resolve_point_expr(&inst.at)?;
                    self.resolved_points.insert(name.clone(), t);
                    t
                } else {
                    bail!("Unknown point or window reference: '{}'", name);
                };

                self.eval_stack.remove(name);
                Ok(res)
            }
            PointExpr::Offset(base, dur, is_add) => {
                let base_t = self.resolve_point_expr(base)?;
                let dur_t = dur.to_seconds(fps);
                if *is_add {
                    Ok(base_t + dur_t)
                } else {
                    Ok(base_t - dur_t)
                }
            }
            PointExpr::Earliest(list) => {
                if list.is_empty() {
                    bail!("earliest() requires at least one point");
                }
                let mut min_t = f64::MAX;
                for item in list {
                    let t = self.resolve_point_expr(item)?;
                    if t < min_t {
                        min_t = t;
                    }
                }
                Ok(min_t)
            }
            PointExpr::Latest(list) => {
                if list.is_empty() {
                    bail!("latest() requires at least one point");
                }
                let mut max_t = f64::MIN;
                for item in list {
                    let t = self.resolve_point_expr(item)?;
                    if t > max_t {
                        max_t = t;
                    }
                }
                Ok(max_t)
            }
        }
    }
}

/// Project a semantic ScriptDocument into a resolved Timeline.
pub fn project_script_to_timeline(
    script: &ScriptDocument,
    fps: f64,
    default_gap_s: f64,
    speech_extents: Option<&HashMap<String, f64>>,
) -> Result<ResolvedTimeline> {
    project_script_to_timeline_with_alignments(script, fps, default_gap_s, speech_extents, None)
}

/// Project a semantic ScriptDocument into a resolved Timeline with word/unit acoustic alignments.
/// When word-level timestamps are provided via segment_unit_timings, moments and selections
/// anchor directly to real acoustic timestamps rather than proportional linear division.
pub fn project_script_to_timeline_with_alignments(
    script: &ScriptDocument,
    fps: f64,
    default_gap_s: f64,
    speech_extents: Option<&HashMap<String, f64>>,
    segment_unit_timings: Option<&HashMap<String, Vec<(f64, f64)>>>,
) -> Result<ResolvedTimeline> {
    let mut windows = Vec::new();
    let mut instants = Vec::new();

    let mut prev_segment_id: Option<String> = None;

    for seg in &script.segments {
        let from_expr = if let Some(ref prev_id) = prev_segment_id {
            if default_gap_s > 0.0 {
                PointExpr::Offset(
                    Box::new(PointExpr::NamedPoint(format!("{}.end", prev_id))),
                    TimeDuration::Seconds(default_gap_s),
                    true,
                )
            } else {
                PointExpr::NamedPoint(format!("{}.end", prev_id))
            }
        } else {
            PointExpr::Start
        };

        // Determine duration: measured speech extent or estimated
        let dur_s = if let Some(extents) = speech_extents {
            extents.get(&seg.id).copied().unwrap_or(seg.estimated_seconds)
        } else {
            seg.estimated_seconds
        };

        windows.push(WindowDecl {
            id: seg.id.clone(),
            from: Some(from_expr),
            until: None,
            for_duration: Some(TimeDuration::Seconds(dur_s)),
            label: Some(format!("Segment: {}", seg.id)),
        });

        // Map Moments inside this segment (e.g. @{claim!})
        let total_units: usize = seg.turns.iter().map(|t| t.units.len()).sum::<usize>().max(1);
        let unit_timings = segment_unit_timings.and_then(|m| m.get(&seg.id));

        for moment in &seg.moments {
            let offset_s = if let Some(timings) = unit_timings {
                if let Some(&(u_start, _)) = timings.get(moment.at_unit_index) {
                    u_start
                } else {
                    let ratio = (moment.at_unit_index as f64 / total_units as f64).clamp(0.0, 1.0);
                    ratio * dur_s
                }
            } else {
                let ratio = (moment.at_unit_index as f64 / total_units as f64).clamp(0.0, 1.0);
                ratio * dur_s
            };

            instants.push(InstantDecl {
                id: moment.name.clone(),
                at: PointExpr::Offset(
                    Box::new(PointExpr::NamedPoint(format!("{}.start", seg.id))),
                    TimeDuration::Seconds(offset_s),
                    true,
                ),
                label: Some(format!("Moment: @{}!", moment.name)),
            });
        }

        // Pair up Selections inside this segment (e.g. @{proof} ... @{/proof})
        let mut open_selections: HashMap<String, usize> = HashMap::new();
        for sel in &seg.selections {
            if !sel.is_close {
                open_selections.insert(sel.name.clone(), sel.at_unit_index);
            } else if let Some(start_idx) = open_selections.remove(&sel.name) {
                let (sel_start, sel_dur) = if let Some(timings) = unit_timings {
                    let s_time = timings.get(start_idx).map(|t| t.0).unwrap_or_else(|| {
                        let start_ratio = (start_idx as f64 / total_units as f64).clamp(0.0, 1.0);
                        start_ratio * dur_s
                    });
                    let e_time = timings.get(sel.at_unit_index.saturating_sub(1)).map(|t| t.1)
                        .or_else(|| timings.get(sel.at_unit_index).map(|t| t.0))
                        .unwrap_or_else(|| {
                            let end_ratio = (sel.at_unit_index as f64 / total_units as f64).clamp(0.0, 1.0);
                            end_ratio * dur_s
                        });
                    (s_time, (e_time - s_time).max(0.05))
                } else {
                    let start_ratio = (start_idx as f64 / total_units as f64).clamp(0.0, 1.0);
                    let end_ratio = (sel.at_unit_index as f64 / total_units as f64).clamp(start_ratio, 1.0);
                    let sel_start = start_ratio * dur_s;
                    let sel_dur = ((end_ratio - start_ratio) * dur_s).max(0.05);
                    (sel_start, sel_dur)
                };

                windows.push(WindowDecl {
                    id: format!("{}:{}", seg.id, sel.name),
                    from: Some(PointExpr::Offset(
                        Box::new(PointExpr::NamedPoint(format!("{}.start", seg.id))),
                        TimeDuration::Seconds(sel_start),
                        true,
                    )),
                    until: None,
                    for_duration: Some(TimeDuration::Seconds(sel_dur)),
                    label: Some(format!("Selection: @{{{}}}", sel.name)),
                });
            }
        }

        prev_segment_id = Some(seg.id.clone());
    }

    let manifest = TimelineManifest {
        id: script.id.clone(),
        frame_rate: fps,
        windows,
        instants,
        end_expr: None,
    };

    let solver = TimelineSolver::new(manifest);
    solver.solve()
}

/// Render a clean ASCII visualization of the resolved timeline.
pub fn render_ascii_timeline(timeline: &ResolvedTimeline) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "=== Resolved Timeline: [{}] (FPS: {}, Duration: {:.3}s, Frames: {}) ===\n\n",
        timeline.id, timeline.frame_rate, timeline.total_seconds, timeline.total_frames
    ));

    out.push_str("┌───────┬────────────────────────────┬─────────────────────┬───────────────────┬────────────────┐\n");
    out.push_str("│ Type  │ ID                         │ Time Range (s)      │ Frame Range (f)   │ Duration (s/f) │\n");
    out.push_str("├───────┼────────────────────────────┼─────────────────────┼───────────────────┼────────────────┤\n");

    for win in &timeline.windows {
        let is_sub = win.id.contains(':');
        let prefix = if is_sub { "  └─ [Sel] " } else { "[Window] " };
        let id_col = format!("{}{}", prefix, win.id);
        out.push_str(&format!(
            "│ Win   │ {:<26} │ {:>7.3}s ~ {:>7.3}s │ {:>7} ~ {:>7} │ {:>6.3}s ({:>4}f) │\n",
            if id_col.len() > 26 { &id_col[..26] } else { &id_col },
            win.start_seconds,
            win.end_seconds,
            win.start_frame,
            win.end_frame,
            win.duration_seconds,
            win.duration_frames
        ));
    }

    if !timeline.instants.is_empty() {
        out.push_str("├───────┼────────────────────────────┼─────────────────────┼───────────────────┼────────────────┤\n");
        for inst in &timeline.instants {
            let label = inst.label.as_deref().unwrap_or("");
            out.push_str(&format!(
                "│ Inst  │ [Point] {:<18} │ at {:>7.3}s        │ at frame {:>7}  │ {:<14} │\n",
                if inst.id.len() > 18 { &inst.id[..18] } else { &inst.id },
                inst.at_seconds,
                inst.at_frame,
                if label.len() > 14 { &label[..14] } else { label }
            ));
        }
    }

    out.push_str("└───────┴────────────────────────────┴─────────────────────┴───────────────────┴────────────────┘\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_duration_parsing() {
        assert_eq!(TimeDuration::parse("12f").unwrap(), TimeDuration::Frames(12));
        assert_eq!(TimeDuration::parse("250ms").unwrap(), TimeDuration::Milliseconds(250));
        assert_eq!(TimeDuration::parse("1.5s").unwrap(), TimeDuration::Seconds(1.5));
        assert_eq!(TimeDuration::parse("3").unwrap(), TimeDuration::Seconds(3.0));
    }

    #[test]
    fn test_linear_timeline_resolution() {
        let manifest = TimelineManifest {
            id: "test_program".to_string(),
            frame_rate: 30.0,
            windows: vec![
                WindowDecl {
                    id: "opening".to_string(),
                    from: Some(PointExpr::Start),
                    until: None,
                    for_duration: Some(TimeDuration::Seconds(3.0)),
                    label: None,
                },
                WindowDecl {
                    id: "body".to_string(),
                    from: Some(PointExpr::Offset(
                        Box::new(PointExpr::NamedPoint("opening.end".to_string())),
                        TimeDuration::Seconds(0.5),
                        true,
                    )),
                    until: None,
                    for_duration: Some(TimeDuration::Seconds(4.0)),
                    label: None,
                },
            ],
            instants: vec![
                InstantDecl {
                    id: "claim".to_string(),
                    at: PointExpr::Offset(
                        Box::new(PointExpr::NamedPoint("opening.end".to_string())),
                        TimeDuration::Frames(6),
                        false,
                    ),
                    label: None,
                },
            ],
            end_expr: None,
        };

        let solver = TimelineSolver::new(manifest);
        let resolved = solver.solve().unwrap();

        assert_eq!(resolved.total_seconds, 7.5);
        assert_eq!(resolved.total_frames, 225);

        let op = resolved.windows.iter().find(|w| w.id == "opening").unwrap();
        assert_eq!(op.start_seconds, 0.0);
        assert_eq!(op.end_seconds, 3.0);
        assert_eq!(op.start_frame, 0);
        assert_eq!(op.end_frame, 90);

        let body = resolved.windows.iter().find(|w| w.id == "body").unwrap();
        assert_eq!(body.start_seconds, 3.5);
        assert_eq!(body.end_seconds, 7.5);
        assert_eq!(body.start_frame, 105);
        assert_eq!(body.end_frame, 225);

        let claim = resolved.instants.iter().find(|i| i.id == "claim").unwrap();
        // 3.0s - 6 frames (6 / 30 = 0.2s) = 2.8s, frame 84
        assert_eq!(claim.at_seconds, 2.8);
        assert_eq!(claim.at_frame, 84);
    }

    #[test]
    fn test_cycle_detection() {
        let manifest = TimelineManifest {
            id: "cycle_test".to_string(),
            frame_rate: 30.0,
            windows: vec![
                WindowDecl {
                    id: "winA".to_string(),
                    from: Some(PointExpr::NamedPoint("winB.end".to_string())),
                    until: None,
                    for_duration: Some(TimeDuration::Seconds(1.0)),
                    label: None,
                },
                WindowDecl {
                    id: "winB".to_string(),
                    from: Some(PointExpr::NamedPoint("winA.end".to_string())),
                    until: None,
                    for_duration: Some(TimeDuration::Seconds(1.0)),
                    label: None,
                },
            ],
            instants: vec![],
            end_expr: None,
        };

        let solver = TimelineSolver::new(manifest);
        let res = solver.solve();
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("Cyclic dependency"));
    }

    #[test]
    fn test_script_projection_with_speech_extents() {
        use crate::script::parse_script;

        let svml = r#"
        <script id="promo">
          <hook>
            <HOST> @{claim!} Hello world!
          </hook>
          <cta>
            <HOST> Check out GitHub!
          </cta>
        </script>
        "#;

        let doc = parse_script(svml).unwrap();
        let mut measured_extents = HashMap::new();
        measured_extents.insert("hook".to_string(), 2.5);
        measured_extents.insert("cta".to_string(), 1.8);

        let timeline = project_script_to_timeline(&doc, 30.0, 0.4, Some(&measured_extents)).unwrap();

        // hook: 0.0 ~ 2.5s (0 ~ 75f)
        // gap: 0.4s
        // cta: 2.9 ~ 4.7s (87 ~ 141f)
        let hook = timeline.windows.iter().find(|w| w.id == "hook").unwrap();
        assert_eq!(hook.start_seconds, 0.0);
        assert_eq!(hook.end_seconds, 2.5);

        let cta = timeline.windows.iter().find(|w| w.id == "cta").unwrap();
        assert_eq!(cta.start_seconds, 2.9);
        assert_eq!(cta.end_seconds, 4.7);

        assert_eq!(timeline.total_seconds, 4.7);
    }

    #[test]
    fn test_script_projection_with_word_alignments() {
        use crate::script::parse_script;

        let svml = r#"
        <script id="promo">
          <hook>
            <HOST> One two @{target!} three @{highlight} four five @{/highlight} six.
          </hook>
        </script>
        "#;

        let doc = parse_script(svml).unwrap();
        let mut measured_extents = HashMap::new();
        measured_extents.insert("hook".to_string(), 6.0);

        // Suppose units have non-linear timing:
        // unit 0 ("One"): 0.0 - 0.5
        // unit 1 ("two"): 0.5 - 1.0
        // unit 2 ("three"): 3.0 - 3.5 (pause of 2 seconds before three!)
        // target! is at unit_index 2
        // highlight is around units 3 and 4
        let mut alignments = HashMap::new();
        alignments.insert(
            "hook".to_string(),
            vec![
                (0.0, 0.5), // One
                (0.5, 1.0), // two
                (3.0, 3.5), // three
                (3.6, 4.2), // four
                (4.3, 5.0), // five
                (5.1, 5.8), // six
            ],
        );

        let timeline = project_script_to_timeline_with_alignments(
            &doc,
            30.0,
            0.0,
            Some(&measured_extents),
            Some(&alignments),
        )
        .unwrap();

        // The target instant should be at 3.0s (the exact start of unit 2 "three"),
        // NOT at (2 / 6) * 6.0 = 2.0s!
        let target_inst = timeline.instants.iter().find(|i| i.id == "target").unwrap();
        assert!((target_inst.at_seconds - 3.0).abs() < 1e-4);
    }
}
