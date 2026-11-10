use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Audio,
    Image,
    Video,
    Json,
    Project,
    Html,
    Script,
}
impl Kind {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Audio => "wav",
            Self::Image => "png",
            Self::Video => "mp4",
            Self::Html => "html",
            _ => "json",
        }
    }
    pub fn media(self) -> bool {
        matches!(self, Self::Audio | Self::Image | Self::Video)
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ScriptView {
    #[default]
    Full,
    Speech,
    Timing,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum FileView {
    #[default]
    Whole,
    ProjectTiming,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Input {
    File {
        path: PathBuf,
        #[serde(rename = "type")]
        ty: Kind,
        #[serde(default)]
        view: FileView,
    },
    Script {
        path: PathBuf,
        #[serde(default)]
        view: ScriptView,
        #[serde(default)]
        segment: Option<String>,
    },
    Output {
        output: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: String,
    pub action: Action,
    #[serde(default)]
    pub revision: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, Input>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "producer", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Action {
    Json {
        value: Value,
    },
    Voice {
        voice: String,
        language: String,
        #[serde(default)]
        direction: String,
    },
    ExtractFrame {
        at_seconds: f64,
    },
    Image {
        spec: crate::image_director::ImagePromptSpec,
        flow_project_id: String,
        #[serde(default)]
        seed: Option<u64>,
    },
    Video {
        spec: crate::video_director::VideoPromptSpec,
        flow_project_id: String,
        #[serde(default)]
        source_edit: Option<crate::video_director::SourceVideoEdit>,
    },
    Transcribe {
        language: String,
        #[serde(default = "asr_model")]
        expected_model: String,
    },
    ForceAlign {
        expected_model: String,
        #[serde(default)]
        min_word_score: f64,
    },
    AdoptPerformance {
        fps: f64,
    },
    AlignSpeech {
        fps: f64,
    },
    ProjectTiming {
        fps: f64,
        #[serde(default = "gap")]
        gap: f64,
    },
    Realign {
        #[serde(default = "fps")]
        fps: f64,
        #[serde(default = "gap")]
        gap: f64,
    },
    Mix {
        #[serde(default)]
        ducking: crate::sound_mix::DuckingConfig,
    },
    CaptionSchedule {
        #[serde(default)]
        recipe: crate::caption_engine::CaptionRecipe,
    },
    RenderCaptions {
        #[serde(default)]
        recipe: crate::caption_engine::CaptionRecipe,
    },
    RenderHtml {
        frames: u64,
        #[serde(default)]
        config: crate::browser_capture::CaptureConfig,
    },
    Compose,
    Review {
        #[serde(default)]
        require_delivery: bool,
    },
}
fn asr_model() -> String {
    "medium".into()
}
fn fps() -> f64 {
    30.0
}
fn gap() -> f64 {
    0.15
}
impl Action {
    pub fn outputs(&self) -> BTreeMap<String, Kind> {
        let pairs: &[(&str, Kind)] = match self {
            Self::Json { .. } => &[("value", Kind::Json)],
            Self::Voice { .. } | Self::Mix { .. } => &[("audio", Kind::Audio)],
            Self::Image { .. } | Self::ExtractFrame { .. } => &[("image", Kind::Image)],
            Self::Video { .. } | Self::RenderHtml { .. } => &[("video", Kind::Video)],
            Self::ForceAlign { .. } => &[("document", Kind::Json)],
            Self::AdoptPerformance { .. } => &[
                ("video", Kind::Video),
                ("audio", Kind::Audio),
                ("document", Kind::Json),
            ],
            Self::AlignSpeech { .. } => &[("alignment", Kind::Json)],
            Self::ProjectTiming { .. } => &[("clock", Kind::Json), ("project", Kind::Project)],
            Self::Realign { .. } => &[("project", Kind::Project)],
            Self::Transcribe { .. } | Self::CaptionSchedule { .. } => &[("document", Kind::Json)],
            Self::RenderCaptions { .. } | Self::Compose => {
                &[("video", Kind::Video), ("project", Kind::Project)]
            }
            Self::Review { .. } => &[("report", Kind::Json)],
        };
        pairs.iter().map(|(n, k)| (n.to_string(), *k)).collect()
    }
    pub fn external(&self) -> bool {
        matches!(
            self,
            Self::Voice { .. } | Self::Image { .. } | Self::Video { .. }
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub task: String,
    pub port: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Author {
    pub format: String,
    pub tasks: Vec<Task>,
    pub outputs: BTreeMap<String, Output>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Candidate {
    File {
        path: PathBuf,
        #[serde(rename = "type")]
        ty: Kind,
    },
    Result {
        result: PathBuf,
        output: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub format: String,
    pub author: PathBuf,
    pub targets: Vec<String>,
    #[serde(default)]
    pub candidates: BTreeMap<String, Candidate>,
    #[serde(default)]
    pub satisfy: BTreeMap<String, String>,
    #[serde(default)]
    pub reuse: Option<PathBuf>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    #[serde(rename = "type")]
    pub ty: Kind,
    pub path: PathBuf,
    pub digest: String,
    pub mode: crate::artifacts::Mode,
    #[serde(default)]
    pub details: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Material {
    pub artifact: Artifact,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "kebab-case")]
pub enum PlannedInput {
    Material {
        material: Material,
    },
    Output {
        output: String,
        #[serde(rename = "type")]
        ty: Kind,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Decision {
    Execute,
    Reuse,
    Blocked,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedTask {
    pub task: Task,
    pub decision: Decision,
    pub reasons: Vec<String>,
    pub inputs: BTreeMap<String, PlannedInput>,
    pub request_key: Option<String>,
    pub cached: Option<TaskRecord>,
    pub previous: Option<TaskRecord>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub format: String,
    pub run: PathBuf,
    pub author: PathBuf,
    pub source_digest: String,
    pub mock: bool,
    pub runtime: BTreeMap<String, String>,
    pub targets: Vec<String>,
    pub outputs: BTreeMap<String, Output>,
    pub provided: BTreeMap<String, Artifact>,
    pub tasks: Vec<PlannedTask>,
    pub blocked: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    Running,
    Succeeded,
    Reused,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub request_key: String,
    pub state: State,
    pub inputs: BTreeMap<String, String>,
    pub outputs: BTreeMap<String, Artifact>,
    pub directory: PathBuf,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildResult {
    pub format: String,
    pub run: PathBuf,
    pub source_digest: String,
    pub mock: bool,
    pub state: State,
    pub targets: Vec<String>,
    pub tasks: BTreeMap<String, TaskRecord>,
    pub outputs: BTreeMap<String, Artifact>,
    pub error: Option<String>,
}
