use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::project::{ProjectManifest, TrackItem};
use crate::script::ScriptDocument;

// ============================================================================
// 1. 核心视觉组件原型 (Component Archetypes)
// ============================================================================

/// 预设视觉组件原型枚举
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ComponentArchetype {
    /// 任务汇聚引擎环：展示多任务卡片汇聚到中央旋转引擎环
    TaskEngine,
    /// 动态时间轴对比槽：展示语音原始片段 vs 弹性拉伸后的时间槽与锚点
    TimelineCompare,
    /// 代码与架构卡片：带 macOS 窗口三色按钮、语法高亮与测试徽标的代码卡片
    CodeArtifact,
    /// 指标与特性徽标牌：多栏高对比度数据指标展示
    FeatureMetric,
    /// 宣传与大字版面：高质感杂志封面与行动号召标语流
    EditorialBanner,
}

// ============================================================================
// 2. 空间与样式主题模型 (Spatial & Theme)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpatialBounds {
    pub width_px: u32,
    pub height_px: u32,
    pub padding_px: u32,
}

impl Default for SpatialBounds {
    fn default() -> Self {
        Self {
            width_px: 640,
            height_px: 360,
            padding_px: 24,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComponentTheme {
    pub bg_gradient: String,
    pub card_bg: String,
    pub border_color: String,
    pub text_primary: String,
    pub text_secondary: String,
    pub accent_color: String,
    pub highlight_gold: String,
    pub font_family: String,
    pub code_font: String,
    pub border_radius_px: u32,
}

impl Default for ComponentTheme {
    fn default() -> Self {
        Self {
            bg_gradient: "radial-gradient(ellipse at 78% 64%, #3c356080, transparent 54%), linear-gradient(135deg, #101723, #20243a)".into(),
            card_bg: "linear-gradient(145deg, rgba(255, 255, 255, 0.05), rgba(255, 255, 255, 0.01))".into(),
            border_color: "#b5a6e737".into(),
            text_primary: "#eee9ff".into(),
            text_secondary: "#bcbad1".into(),
            accent_color: "#a89af7".into(),
            highlight_gold: "#d99b75".into(),
            font_family: "Arial, Helvetica, -apple-system, sans-serif".into(),
            code_font: "Menlo, Monaco, Consolas, monospace".into(),
            border_radius_px: 12,
        }
    }
}

// ============================================================================
// 3. 动态动画规格模型 (Animation Spec)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MotionType {
    Entrance,
    Looping,
    Progressive,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComponentAnimation {
    pub duration_frames: u64,
    pub fps: f64,
    pub entrance_frames: u64,
    pub exit_frames: u64,
    pub motion_type: MotionType,
}

impl Default for ComponentAnimation {
    fn default() -> Self {
        Self {
            duration_frames: 60,
            fps: 30.0,
            entrance_frames: 12,
            exit_frames: 8,
            motion_type: MotionType::Progressive,
        }
    }
}

// ============================================================================
// 4. 完整的组件规范定义 (VisualComponentSpec)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VisualComponentSpec {
    pub id: String,
    pub archetype: ComponentArchetype,
    pub title: String,
    pub bounds: SpatialBounds,
    pub theme: ComponentTheme,
    pub animation: ComponentAnimation,
    pub props: serde_json::Value,
}

// ============================================================================
// 5. Web-as-Video 动态 HTML 编译与生成实现
// ============================================================================

/// 将视觉组件规格编译为完全自包含的 Web-as-Video 动态 HTML 文件
pub fn render_component_to_html(spec: &VisualComponentSpec) -> Result<String> {
    let spec_json = serde_json::to_string(spec)?;

    let (content_markup, extra_css, render_script) = match spec.archetype {
        ComponentArchetype::TaskEngine => render_task_engine(spec)?,
        ComponentArchetype::TimelineCompare => render_timeline_compare(spec)?,
        ComponentArchetype::CodeArtifact => render_code_artifact(spec)?,
        ComponentArchetype::FeatureMetric => render_feature_metric(spec)?,
        ComponentArchetype::EditorialBanner => render_editorial_banner(spec)?,
    };

    let html = format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>{title}</title>
<style>
  * {{ box-sizing: border-box; }}
  html, body {{
    margin: 0;
    padding: 0;
    width: {width}px;
    height: {height}px;
    overflow: hidden;
    background: {bg};
    font-family: {font};
    color: {text_primary};
    -webkit-font-smoothing: antialiased;
  }}
  .scene {{
    position: relative;
    width: {width}px;
    height: {height}px;
    padding: {padding}px;
  }}
  .eyebrow {{
    font-size: 11px;
    letter-spacing: 2px;
    text-transform: uppercase;
    color: {accent};
  }}
  h1 {{
    margin: 12px 0 0 0;
    font-size: 38px;
    letter-spacing: -1.4px;
    line-height: 1.1;
  }}
  h2 {{
    margin: 10px 0 0 0;
    font-size: 26px;
    letter-spacing: -0.6px;
  }}
  p {{
    margin: 6px 0 0 0;
    font-size: 13px;
    line-height: 1.5;
    color: {text_secondary};
  }}
  .card {{
    border: 1px solid {border};
    background: {card_bg};
    border-radius: {radius}px;
    box-shadow: 0 16px 32px rgba(0, 0, 0, 0.25);
  }}
  .ring {{
    border: 2px solid {accent};
    border-radius: 50%;
    box-shadow: 0 0 36px rgba(169, 151, 255, 0.22), inset 0 0 24px rgba(169, 151, 255, 0.12);
    position: relative;
    display: grid;
    place-items: center;
  }}
  .core {{
    width: 36px;
    height: 36px;
    transform: rotate(45deg);
    border: 1px solid {gold};
    background: linear-gradient(135deg, rgba(168, 154, 247, 0.3), #23283d);
    box-shadow: 6px 6px 0 #080e1b;
  }}
  {extra_css}
</style>
</head>
<body>
<div class="scene" id="component-scene">
  {content_markup}
</div>

<script>
  window.__COMPONENT_SPEC__ = {spec_json};
  const SCENE = document.getElementById('component-scene');

  // 初始化设置
  window.setup = function(root, data) {{
    // 准备稳定结构
  }};

  // 每一帧的确定性状态渲染 (提供 localFrame 与 progress 0.0..1.0)
  window.renderFrame = function(localFrame, progress) {{
    {render_script}
  }};

  // 默认渲染第 0 帧
  window.renderFrame(0, 0.0);
</script>
</body>
</html>"#,
        title = spec.title,
        width = spec.bounds.width_px,
        height = spec.bounds.height_px,
        padding = spec.bounds.padding_px,
        bg = spec.theme.bg_gradient,
        font = spec.theme.font_family,
        text_primary = spec.theme.text_primary,
        text_secondary = spec.theme.text_secondary,
        accent = spec.theme.accent_color,
        gold = spec.theme.highlight_gold,
        border = spec.theme.border_color,
        card_bg = spec.theme.card_bg,
        radius = spec.theme.border_radius_px,
        extra_css = extra_css,
        content_markup = content_markup,
        render_script = render_script,
        spec_json = spec_json,
    );

    Ok(html)
}

// ----------------------------------------------------------------------------
// 渲染子模块 1: TaskEngine (五任务汇聚引擎环)
// ----------------------------------------------------------------------------
fn render_task_engine(spec: &VisualComponentSpec) -> Result<(String, String, String)> {
    let tasks = spec
        .props
        .get("tasks")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_else(|| {
            vec![
                serde_json::json!("Probe"),
                serde_json::json!("Transcribe"),
                serde_json::json!("Detect"),
                serde_json::json!("Inspect"),
                serde_json::json!("Sequence"),
            ]
        });

    let subtitle = spec
        .props
        .get("subtitle")
        .and_then(|v| v.as_str())
        .unwrap_or("Want five video tasks in one?");
    let engine_name = spec
        .props
        .get("engine_name")
        .and_then(|v| v.as_str())
        .unwrap_or("clone-any-video");

    let mut task_cards = String::new();
    for (i, t) in tasks.iter().enumerate() {
        let name = t.as_str().unwrap_or("Task");
        task_cards.push_str(&format!(
            r#"<div class="card task" id="task-card-{}"><span class="dot"></span>{}</div>"#,
            i, name
        ));
    }

    let markup = format!(
        r#"<div class="eyebrow">{title}</div>
<h2 style="margin-top:12px">{subtitle}</h2>
<div class="row" style="margin-top:20px;display:flex;gap:8px">
  {cards}
</div>
<div class="connect" id="engine-connect"></div>
<div class="ring" id="engine-ring" style="position:absolute;left:278px;top:186px;width:84px;height:84px">
  <div class="core" id="engine-core" style="width:32px;height:32px"></div>
</div>
<h2 style="position:absolute;top:284px;left:0;right:0;text-align:center;font-size:24px;letter-spacing:-0.5px">{engine}</h2>"#,
        title = spec.title,
        subtitle = subtitle,
        cards = task_cards,
        engine = engine_name,
    );

    let extra_css = r#"
  .task {
    flex: 1;
    font-size: 11px;
    padding: 12px 4px;
    text-align: center;
    color: #e7e1fa;
    transition: transform 0.1s ease;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #b6a7fd;
    display: inline-block;
    margin-right: 5px;
  }
  .connect {
    height: 32px;
    border: 1px solid rgba(168, 154, 247, 0.35);
    border-top: 0;
    border-radius: 0 0 36px 36px;
    width: 440px;
    margin: 0 auto;
  }
"#;

    let render_script = r#"
    const core = document.getElementById('engine-core');
    const ring = document.getElementById('engine-ring');
    const rot = 45 + progress * 180;
    if (core) core.style.transform = `rotate(${rot}deg) scale(${1 + 0.05 * Math.sin(progress * Math.PI * 4)})`;
    if (ring) ring.style.boxShadow = `0 0 ${30 + 15 * Math.sin(progress * Math.PI * 2)}px rgba(169, 151, 255, 0.3)`;
"#;

    Ok((markup, extra_css.into(), render_script.into()))
}

// ----------------------------------------------------------------------------
// 渲染子模块 2: TimelineCompare (动态时间轴对比槽)
// ----------------------------------------------------------------------------
fn render_timeline_compare(spec: &VisualComponentSpec) -> Result<(String, String, String)> {
    let top_label = spec
        .props
        .get("top_label")
        .and_then(|v| v.as_str())
        .unwrap_or("Fixed Speech Extents (Seconds)");
    let bot_label = spec
        .props
        .get("bottom_label")
        .and_then(|v| v.as_str())
        .unwrap_or("SVML Dynamic Timeline (Elastic)");

    let markup = format!(
        r#"<div class="eyebrow">{title}</div>
<h2 style="margin-top:10px">Audio Extents vs Elastic Timeline</h2>
<div class="timeline-container">
  <div class="lane-label">{top_label}</div>
  <div class="lane" id="top-lane">
    <div class="extent" style="left:10%;width:22%">Shot 1: 1.76s</div>
    <div class="extent" style="left:36%;width:18%">Shot 2: 1.45s</div>
    <div class="extent" style="left:58%;width:34%">Shot 3: 2.70s</div>
  </div>
  <div class="lane-label" style="margin-top:14px">{bot_label}</div>
  <div class="lane" id="bot-lane">
    <div class="shot-pill" style="left:8%;width:26%">Shot 1 (Elastic)</div>
    <div class="shot-pill" style="left:38%;width:22%">Shot 2 (Elastic)</div>
    <div class="shot-pill" style="left:64%;width:30%">Shot 3 (Elastic)</div>
    <div class="anchor" style="left:36%"></div>
    <div class="anchor" style="left:62%"></div>
  </div>
  <div class="playhead" id="timeline-playhead"></div>
</div>"#,
        title = spec.title,
        top_label = top_label,
        bot_label = bot_label,
    );

    let extra_css = r#"
  .timeline-container {
    position: relative;
    margin-top: 18px;
    padding: 16px;
    background: rgba(0, 0, 0, 0.25);
    border: 1px solid rgba(168, 154, 247, 0.2);
    border-radius: 12px;
  }
  .lane-label {
    font-size: 11px;
    color: #a89af7;
    letter-spacing: 0.5px;
  }
  .lane {
    position: relative;
    height: 40px;
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 8px;
    margin-top: 6px;
    overflow: hidden;
  }
  .extent {
    position: absolute;
    top: 6px;
    height: 26px;
    background: rgba(217, 155, 117, 0.35);
    border: 1px solid #d99b75;
    border-radius: 6px;
    font-size: 10px;
    color: #ffe9de;
    display: grid;
    place-items: center;
  }
  .shot-pill {
    position: absolute;
    top: 6px;
    height: 26px;
    background: rgba(168, 154, 247, 0.3);
    border: 1px solid #a89af7;
    border-radius: 6px;
    font-size: 10px;
    color: #eee9ff;
    display: grid;
    place-items: center;
  }
  .anchor {
    position: absolute;
    top: -2px;
    width: 6px;
    height: 44px;
    background: #d99b75;
    box-shadow: 0 0 10px #d99b75;
  }
  .playhead {
    position: absolute;
    top: 12px;
    bottom: 12px;
    width: 2px;
    background: #f59e0b;
    box-shadow: 0 0 12px #f59e0b;
    left: 10%;
  }
"#;

    let render_script = r#"
    const playhead = document.getElementById('timeline-playhead');
    if (playhead) {
      const leftPct = 10 + progress * 80;
      playhead.style.left = `${leftPct}%`;
    }
"#;

    Ok((markup, extra_css.into(), render_script.into()))
}

// ----------------------------------------------------------------------------
// 渲染子模块 3: CodeArtifact (代码与架构卡片)
// ----------------------------------------------------------------------------
fn render_code_artifact(spec: &VisualComponentSpec) -> Result<(String, String, String)> {
    let filename = spec
        .props
        .get("filename")
        .and_then(|v| v.as_str())
        .unwrap_or("engine.rs");
    let code_snippet = spec.props.get("code").and_then(|v| v.as_str()).unwrap_or(
        "pub struct AgentEngine {\n    timeline: DynamicTimeline,\n    compositor: Compositor,\n}",
    );
    let badge_text = spec
        .props
        .get("badge")
        .and_then(|v| v.as_str())
        .unwrap_or("test result: ok. 180 passed");

    let markup = format!(
        r#"<div class="eyebrow">{title}</div>
<h2 style="margin-top:10px">Pure Rust Engine Architecture</h2>
<div class="code-card card">
  <div class="window-header">
    <div class="dots">
      <span class="w-dot red"></span>
      <span class="w-dot yellow"></span>
      <span class="w-dot green"></span>
    </div>
    <div class="filename">{filename}</div>
    <div class="badge">{badge}</div>
  </div>
  <pre class="code-body" id="code-content"><code>{code}</code></pre>
</div>"#,
        title = spec.title,
        filename = filename,
        badge = badge_text,
        code = code_snippet,
    );

    let extra_css = r#"
  .code-card {
    margin-top: 18px;
    background: #0d121d;
    border: 1px solid rgba(168, 154, 247, 0.25);
    border-radius: 10px;
    overflow: hidden;
  }
  .window-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 14px;
    background: rgba(255, 255, 255, 0.04);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  }
  .dots {
    display: flex;
    gap: 6px;
  }
  .w-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
  }
  .red { background: #ef4444; }
  .yellow { background: #f59e0b; }
  .green { background: #10b981; }
  .filename {
    font-family: Menlo, monospace;
    font-size: 11px;
    color: #a89af7;
  }
  .badge {
    font-size: 10px;
    padding: 2px 8px;
    background: rgba(16, 185, 129, 0.2);
    border: 1px solid #10b981;
    color: #34d399;
    border-radius: 4px;
  }
  .code-body {
    margin: 0;
    padding: 16px;
    font-family: Menlo, monospace;
    font-size: 12px;
    line-height: 1.6;
    color: #c4b8f4;
  }
"#;

    let render_script = r#"
    const card = document.querySelector('.code-card');
    if (card) {
      card.style.boxShadow = `0 12px 30px rgba(0, 0, 0, 0.4), 0 0 ${12 + 10 * Math.sin(progress * Math.PI)}px rgba(168, 154, 247, 0.25)`;
    }
"#;

    Ok((markup, extra_css.into(), render_script.into()))
}

// ----------------------------------------------------------------------------
// 渲染子模块 4: FeatureMetric (指标与特性徽标牌)
// ----------------------------------------------------------------------------
fn render_feature_metric(spec: &VisualComponentSpec) -> Result<(String, String, String)> {
    let metrics = spec
        .props
        .get("metrics")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_else(|| {
            vec![
                serde_json::json!({ "val": "48 kHz", "desc": "Canonical Master Clock" }),
                serde_json::json!({ "val": "100%", "desc": "Pure Rust Memory Safe" }),
                serde_json::json!({ "val": "0 ms", "desc": "Clock Drift Across Cuts" }),
            ]
        });

    let mut metric_cards = String::new();
    for m in &metrics {
        let val = m.get("val").and_then(|v| v.as_str()).unwrap_or("0");
        let desc = m.get("desc").and_then(|v| v.as_str()).unwrap_or("Metric");
        metric_cards.push_str(&format!(
            r#"<div class="card m-card">
  <div class="m-val">{}</div>
  <div class="m-desc">{}</div>
</div>"#,
            val, desc
        ));
    }

    let markup = format!(
        r#"<div class="eyebrow">{title}</div>
<h2 style="margin-top:10px">Engine Performance Benchmarks</h2>
<div class="metric-row">
  {cards}
</div>"#,
        title = spec.title,
        cards = metric_cards,
    );

    let extra_css = r#"
  .metric-row {
    display: flex;
    gap: 12px;
    margin-top: 24px;
  }
  .m-card {
    flex: 1;
    padding: 24px 16px;
    text-align: center;
  }
  .m-val {
    font-size: 32px;
    font-weight: 800;
    color: #d99b75;
    letter-spacing: -1px;
  }
  .m-desc {
    margin-top: 8px;
    font-size: 11px;
    color: #a89af7;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
"#;

    let render_script = r#"
    const vals = document.querySelectorAll('.m-val');
    vals.forEach((v, idx) => {
      v.style.transform = `scale(${1 + 0.03 * Math.sin(progress * Math.PI * 2 + idx)})`;
    });
"#;

    Ok((markup, extra_css.into(), render_script.into()))
}

// ----------------------------------------------------------------------------
// 渲染子模块 5: EditorialBanner (宣传与大字版面)
// ----------------------------------------------------------------------------
fn render_editorial_banner(spec: &VisualComponentSpec) -> Result<(String, String, String)> {
    let headline = spec
        .props
        .get("headline")
        .and_then(|v| v.as_str())
        .unwrap_or("Build your Agent.");
    let cta = spec
        .props
        .get("cta")
        .and_then(|v| v.as_str())
        .unwrap_or("Open Source Engine");

    let markup = format!(
        r#"<div class="eyebrow">{title}</div>
<h1 style="margin-top:20px;font-size:46px">{headline}</h1>
<div class="cta-pill">{cta}</div>"#,
        title = spec.title,
        headline = headline,
        cta = cta,
    );

    let extra_css = r#"
  .cta-pill {
    display: inline-block;
    margin-top: 24px;
    padding: 10px 24px;
    background: linear-gradient(135deg, #a89af7, #d99b75);
    color: #111827;
    font-weight: 700;
    font-size: 14px;
    border-radius: 24px;
    box-shadow: 0 8px 24px rgba(217, 155, 117, 0.4);
  }
"#;

    let render_script = r#"
    const pill = document.querySelector('.cta-pill');
    if (pill) {
      pill.style.transform = `translateY(${Math.sin(progress * Math.PI * 2) * -3}px)`;
    }
"#;

    Ok((markup, extra_css.into(), render_script.into()))
}

// ============================================================================
// 6. 从项目与剧本自动设计视觉组件全家桶 (Auto Design)
// ============================================================================

/// 分析 project.json 与 SVML 剧本，自动推导并设计每镜头对应的视觉组件规格
pub fn auto_design_components_for_project(
    manifest: &ProjectManifest,
    _script_doc: &ScriptDocument,
) -> Result<Vec<VisualComponentSpec>> {
    let mut specs = Vec::new();
    let bounds = SpatialBounds {
        width_px: manifest.meta.width,
        height_px: manifest.meta.height,
        padding_px: 24,
    };

    for (idx, shot) in manifest.shots.iter().enumerate() {
        let shot_idx = idx + 1;
        let shot_id = format!("shot_{:02}", shot_idx);
        let duration_frames = shot.end_frame.saturating_sub(shot.start_frame).max(30);

        let animation = ComponentAnimation {
            duration_frames,
            fps: manifest.meta.fps,
            entrance_frames: 10,
            exit_frames: 8,
            motion_type: MotionType::Progressive,
        };

        // 依据分镜编号与语义标签推导组件原型与参数
        let (archetype, title, props) = match shot_idx {
            1 => (
                ComponentArchetype::TaskEngine,
                "Probe / Transcribe / Detect / Inspect / Sequence".into(),
                serde_json::json!({
                    "subtitle": "Want five video tasks in one?",
                    "engine_name": "clone-any-video",
                    "tasks": ["Probe", "Transcribe", "Detect", "Inspect", "Sequence"]
                }),
            ),
            2 => (
                ComponentArchetype::EditorialBanner,
                "Video Input & AST Parsing".into(),
                serde_json::json!({
                    "headline": "Read SVML & Media Specs",
                    "cta": "Native Rust AST Parser"
                }),
            ),
            3 => (
                ComponentArchetype::CodeArtifact,
                "Semantic Script Structs".into(),
                serde_json::json!({
                    "filename": "script.rs",
                    "code": "pub struct ScriptDocument {\n    pub segments: Vec<Segment>,\n    pub total_words: usize,\n}",
                    "badge": "100% Parsed"
                }),
            ),
            4 => (
                ComponentArchetype::TimelineCompare,
                "Timeline Decoupling".into(),
                serde_json::json!({
                    "top_label": "Speech Voice Extents (Seconds)",
                    "bottom_label": "SVML Elastic Dynamic Timeline"
                }),
            ),
            5 => (
                ComponentArchetype::TimelineCompare,
                "Sub-second Word Anchors".into(),
                serde_json::json!({
                    "top_label": "Whisper Timestamp Tokens",
                    "bottom_label": "Elastic Stretched Shot Markers"
                }),
            ),
            6 => (
                ComponentArchetype::FeatureMetric,
                "High Performance Foundation".into(),
                serde_json::json!({
                    "metrics": [
                        { "val": "48 kHz", "desc": "Canonical Clock" },
                        { "val": "180+", "desc": "Unit Tests Pass" },
                        { "val": "Pure Rust", "desc": "Zero C++ Drift" }
                    ]
                }),
            ),
            7 => (
                ComponentArchetype::CodeArtifact,
                "Automated Verification".into(),
                serde_json::json!({
                    "filename": "test_all.sh",
                    "code": "$ cargo test --workspace\ntest result: ok. 180 passed; 0 failed\n=== All chapters verified! ===",
                    "badge": "100% Pass"
                }),
            ),
            _ => (
                ComponentArchetype::EditorialBanner,
                "Open Source Ecosystem".into(),
                serde_json::json!({
                    "headline": "Build Your Video Agent",
                    "cta": "github.com/stoic/clone-any-video"
                }),
            ),
        };

        specs.push(VisualComponentSpec {
            id: shot_id,
            archetype,
            title,
            bounds: bounds.clone(),
            theme: ComponentTheme::default(),
            animation,
            props,
        });
    }

    Ok(specs)
}

/// 将生成的组件挂载回 ProjectManifest 的 visual_components 轨道
pub fn update_manifest_with_visual_components(
    manifest: &ProjectManifest,
    specs: &[VisualComponentSpec],
    component_dir: &std::path::Path,
) -> Result<ProjectManifest> {
    let mut updated = manifest.clone();
    let mut comp_items = Vec::new();

    for (idx, spec) in specs.iter().enumerate() {
        let shot = &manifest.shots[idx.min(manifest.shots.len() - 1)];
        let file_name = format!("{}.html", spec.id);
        let rel_path = component_dir.join(&file_name);

        comp_items.push(TrackItem {
            id: format!("comp_{:02}", idx + 1),
            name: spec.title.clone(),
            start_seconds: shot.start_seconds,
            end_seconds: shot.end_seconds,
            start_frame: shot.start_frame,
            end_frame: shot.end_frame,
            payload: serde_json::json!({
                "component_id": spec.id,
                "archetype": spec.archetype,
                "file_path": rel_path.to_string_lossy(),
                "duration_frames": spec.animation.duration_frames,
            }),
        });
    }

    updated.tracks.visual_components = comp_items;
    Ok(updated)
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_task_engine_to_html() {
        let spec = VisualComponentSpec {
            id: "shot_01".into(),
            archetype: ComponentArchetype::TaskEngine,
            title: "Task Engine".into(),
            bounds: SpatialBounds::default(),
            theme: ComponentTheme::default(),
            animation: ComponentAnimation::default(),
            props: serde_json::json!({
                "subtitle": "Want five tasks?",
                "engine_name": "test-engine",
                "tasks": ["A", "B", "C"]
            }),
        };

        let html = render_component_to_html(&spec).expect("Render HTML");
        assert!(html.contains("<!doctype html>"));
        assert!(html.contains("window.renderFrame"));
        assert!(html.contains("test-engine"));
        assert!(html.contains("task-card-0"));
    }

    #[test]
    fn test_render_timeline_compare_to_html() {
        let spec = VisualComponentSpec {
            id: "shot_04".into(),
            archetype: ComponentArchetype::TimelineCompare,
            title: "Timeline Compare".into(),
            bounds: SpatialBounds::default(),
            theme: ComponentTheme::default(),
            animation: ComponentAnimation::default(),
            props: serde_json::json!({
                "top_label": "Speech Extents",
                "bottom_label": "Elastic Timeline"
            }),
        };

        let html = render_component_to_html(&spec).expect("Render HTML");
        assert!(html.contains("timeline-playhead"));
        assert!(html.contains("Speech Extents"));
        assert!(html.contains("Elastic Timeline"));
    }

    #[test]
    fn test_render_code_artifact_to_html() {
        let spec = VisualComponentSpec {
            id: "shot_03".into(),
            archetype: ComponentArchetype::CodeArtifact,
            title: "Rust Code".into(),
            bounds: SpatialBounds::default(),
            theme: ComponentTheme::default(),
            animation: ComponentAnimation::default(),
            props: serde_json::json!({
                "filename": "main.rs",
                "code": "fn main() { println!(\"ok\"); }",
                "badge": "100% Passed"
            }),
        };

        let html = render_component_to_html(&spec).expect("Render HTML");
        assert!(html.contains("main.rs"));
        assert!(html.contains("100% Passed"));
        assert!(html.contains("fn main()"));
    }

    #[test]
    fn test_auto_design_from_project_and_manifest_update() {
        let manifest = ProjectManifest {
            master_audio: None,
            format: "clone-any-video.project.v1".into(),
            meta: crate::project::ProjectMeta {
                id: "test".into(),
                title: "Test Project".into(),
                width: 640,
                height: 360,
                fps: 30.0,
                total_seconds: 6.0,
                total_frames: 180,
            },
            shots: vec![
                crate::project::ShotDefinition {
                    id: "shot_01".into(),
                    segment_id: "seg_01".into(),
                    shot_index: 1,
                    start_seconds: 0.0,
                    end_seconds: 3.0,
                    start_frame: 0,
                    end_frame: 90,
                    duration_seconds: 3.0,
                    visual_type: crate::project::VisualType::HtmlComponent,
                    description: "Shot 1 description".into(),
                    prompt_hint: None,
                },
                crate::project::ShotDefinition {
                    id: "shot_02".into(),
                    segment_id: "seg_02".into(),
                    shot_index: 2,
                    start_seconds: 3.0,
                    end_seconds: 6.0,
                    start_frame: 90,
                    end_frame: 180,
                    duration_seconds: 3.0,
                    visual_type: crate::project::VisualType::HtmlComponent,
                    description: "Shot 2 description".into(),
                    prompt_hint: None,
                },
            ],
            tracks: crate::project::LayerTracks {
                visual_bg: vec![],
                visual_components: vec![],
                captions: vec![],
                voice: vec![],
                sfx: vec![],
                bgm: vec![],
            },
            asset_tasks: vec![],
        };

        let script = ScriptDocument {
            id: "test".into(),
            segments: vec![],
            total_words: 10,
            total_estimated_seconds: 6.0,
        };

        let specs = auto_design_components_for_project(&manifest, &script).unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].archetype, ComponentArchetype::TaskEngine);
        assert_eq!(specs[1].archetype, ComponentArchetype::EditorialBanner);

        let updated_manifest = update_manifest_with_visual_components(
            &manifest,
            &specs,
            std::path::Path::new("assets"),
        )
        .unwrap();
        assert_eq!(updated_manifest.tracks.visual_components.len(), 2);
        assert_eq!(updated_manifest.tracks.visual_components[0].id, "comp_01");
    }
}
