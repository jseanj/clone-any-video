use super::{model::*, source};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub fn hash(value: &impl serde::Serialize) -> Result<String> {
    Ok(crate::artifacts::digest(&serde_json::to_vec(value)?))
}
pub fn input_identity(a: &Artifact) -> Result<String> {
    hash(&(
        a.ty,
        &a.digest,
        a.mode,
        json!({"caption_windows":a.details["caption_windows"],"fps":a.details["fps"],"subtitle_delivery":a.details["subtitle_delivery"],"caption_expectations":a.details["caption_expectations"],"frame_domain":a.details["frame_domain"],"program_clock_id":a.details["program_clock_id"],"total_frames":a.details["total_frames"],"performance":a.details["performance"],"voice_media_digests":a.details["voice_media_digests"]}),
    ))
}
pub fn canonical_json(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}
pub fn verify(artifact: &Artifact) -> Result<()> {
    if crate::artifacts::content_digest(&artifact.path)? != artifact.digest {
        bail!("Artifact bytes changed: {:?}", artifact.path);
    }
    if artifact.ty.media() {
        let mut current = artifact.clone();
        current.mode = crate::artifacts::mode(&artifact.path);
        current.details = crate::artifacts::details(&artifact.path).unwrap_or(Value::Null);
        if input_identity(&current)? != input_identity(artifact)? {
            bail!(
                "Artifact provenance or delivery evidence changed: {:?}",
                artifact.path
            );
        }
    }
    Ok(())
}
pub fn file(path: &Path, ty: Kind) -> Result<Artifact> {
    let path = path
        .canonicalize()
        .with_context(|| format!("Missing input {:?}", path))?;
    if !path.is_file() {
        bail!("Input is not a file");
    }
    match ty {
        Kind::Json => {
            let _ = canonical_json(&path)?;
        }
        Kind::Project => {
            let p: crate::project::ProjectManifest =
                serde_json::from_value(canonical_json(&path)?)?;
            crate::project::validate_project(&p)?;
        }
        Kind::Image => {
            let _ = image::open(&path)?;
        }
        Kind::Audio | Kind::Video => {
            let out = std::process::Command::new("ffprobe")
                .args([
                    "-v",
                    "error",
                    "-show_entries",
                    "stream=codec_type",
                    "-of",
                    "json",
                ])
                .arg(&path)
                .output()?;
            if !out.status.success() {
                bail!("Invalid {:?} media {:?}", ty, path);
            }
            let v: Value = serde_json::from_slice(&out.stdout)?;
            let expected = if ty == Kind::Audio { "audio" } else { "video" };
            if !v["streams"]
                .as_array()
                .is_some_and(|streams| streams.iter().any(|s| s["codec_type"] == expected))
            {
                bail!("Supplied {:?} lacks a {} stream", path, expected);
            }
        }
        Kind::Html => {
            let _ = std::fs::read_to_string(&path)?;
        }
        Kind::Script => bail!("Script files use kind=script to capture parsed semantic input"),
    }
    Ok(Artifact {
        details: crate::artifacts::details(&path).unwrap_or(Value::Null),
        digest: crate::artifacts::content_digest(&path)?,
        mode: if ty.media() {
            crate::artifacts::mode(&path)
        } else {
            crate::artifacts::Mode::Production
        },
        path,
        ty,
    })
}
fn material(input: &Input, base: &Path) -> Result<Material> {
    match input {
        Input::File { path, ty, view } => {
            let mut artifact = file(&source::anchored(base, path), *ty)?;
            let value = if *view == FileView::ProjectTiming {
                if *ty != Kind::Project {
                    bail!("project-timing view requires a Project input");
                }
                let mut p: crate::project::ProjectManifest =
                    serde_json::from_value(canonical_json(&artifact.path)?)?;
                p.master_audio = None;
                p.tracks.captions.clear();
                p.tracks.visual_components.clear();
                p.tracks.visual_bg.clear();
                p.tracks.bgm.clear();
                p.tracks.sfx.clear();
                p.asset_tasks.clear();
                Some(serde_json::to_value(p)?)
            } else {
                None
            };
            if let Some(v) = &value {
                artifact.digest = hash(v)?;
            }
            Ok(Material { artifact, value })
        }
        Input::Script {
            path,
            view,
            segment,
        } => {
            let path = source::anchored(base, path).canonicalize()?;
            let mut script = crate::script::parse_script(&std::fs::read_to_string(&path)?)?;
            if let Some(id) = segment {
                script.segments.retain(|s| s.id == *id);
                if script.segments.is_empty() {
                    bail!("Unknown Script segment {}", id);
                }
            }
            if *view == ScriptView::Speech {
                for seg in &mut script.segments {
                    seg.moments.clear();
                    seg.selections.clear();
                    for turn in &mut seg.turns {
                        turn.units.clear();
                        turn.cues.clear();
                    }
                }
            }
            if *view == ScriptView::Timing {
                for seg in &mut script.segments {
                    for turn in &mut seg.turns {
                        turn.cues.clear();
                        for unit in &mut turn.units {
                            unit.display.clear();
                            unit.attributes.clear();
                            unit.is_dual = false;
                        }
                    }
                }
            }
            script.total_words = script
                .segments
                .iter()
                .map(|s| s.spoken_text.chars().count())
                .sum();
            script.total_estimated_seconds =
                script.segments.iter().map(|s| s.estimated_seconds).sum();
            let value = serde_json::to_value(script)?;
            Ok(Material {
                artifact: Artifact {
                    ty: Kind::Script,
                    path,
                    digest: hash(&value)?,
                    mode: crate::artifacts::Mode::Production,
                    details: Value::Null,
                },
                value: Some(value),
            })
        }
        Input::Output { .. } => bail!("Output resolved through graph"),
    }
}
pub fn runtime() -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for (name, default) in [
        ("TTS_PROVIDER", "openai"),
        ("MIMO_TTS_MODEL", "mimo-v2.5-tts"),
        ("MIMO_BASE_URL", "https://api.xiaomimimo.com/v1"),
        ("OPENAI_BASE_URL", "https://api.openai.com/v1"),
        ("IMAGE_PROVIDER", "openai"),
        ("VIDEO_PROVIDER", "openai"),
        ("FLOWKIT_BASE_URL", "http://127.0.0.1:8100"),
        ("ALIGNMENT_BASE_URL", "http://127.0.0.1:8766"),
        ("FLOWKIT_VIDEO_FAMILY", "omni_flash"),
        ("FLOWKIT_VIDEO_RESOLUTION", "360p"),
        ("FLOWKIT_VIDEO_OMNI_VERSION", "1.1"),
        ("FLOWKIT_IMAGE_MODEL", ""),
    ] {
        let value = std::env::var(name).unwrap_or_else(|_| default.into());
        if name.ends_with("BASE_URL") {
            let url = reqwest::Url::parse(&value)?;
            if !url.username().is_empty() || url.password().is_some() || url.query().is_some() {
                bail!("Provider base URLs in plans must not contain credentials or query strings");
            }
        }
        values.insert(name.into(), value);
    }
    Ok(values)
}
fn implementation(action: &Action) -> String {
    let code = match action {
        Action::Voice { .. } => include_str!("../voice_director.rs"),
        Action::Image { .. } => include_str!("../image_director.rs"),
        Action::ExtractFrame { .. } => include_str!("../tiles.rs"),
        Action::Video { .. } => include_str!("../video_director.rs"),
        Action::ForceAlign { .. } => concat!(
            include_str!("../force_alignment.rs"),
            include_str!("../../scripts/alignment_daemon.py")
        ),
        Action::AdoptPerformance { .. } => include_str!("../performance.rs"),
        Action::Transcribe { .. } => include_str!("../transcribe.rs"),
        Action::AlignSpeech { .. } | Action::ProjectTiming { .. } => {
            include_str!("../semantic_clock.rs")
        }
        Action::Realign { .. } => include_str!("../voice_director.rs"),
        Action::Mix { .. } => include_str!("../sound_mix.rs"),
        Action::CaptionSchedule { .. } | Action::RenderCaptions { .. } => {
            include_str!("../caption_engine.rs")
        }
        Action::RenderHtml { .. } => include_str!("../browser_capture.rs"),
        Action::Compose => include_str!("../compositor.rs"),
        Action::Review { .. } => include_str!("../video_agent.rs"),
        Action::Json { .. } => "json-input-template@1",
    };
    let helpers = match action {
        Action::Realign { .. } => concat!(
            include_str!("../timeline.rs"),
            include_str!("../project.rs")
        ),
        Action::RenderCaptions { .. } => concat!(
            include_str!("../production.rs"),
            include_str!("../browser_capture.rs"),
            include_str!("../chrome_session.rs"),
            include_str!("../delivery.rs")
        ),
        Action::RenderHtml { .. } => include_str!("../chrome_session.rs"),
        Action::CaptionSchedule { .. } => include_str!("../speech_alignment.rs"),
        Action::Review { .. } | Action::Compose => {
            concat!(include_str!("../delivery.rs"), include_str!("../probe.rs"))
        }
        Action::Transcribe { .. } => include_str!("../audio.rs"),
        _ => "",
    };
    crate::artifacts::digest(
        format!(
            "graph-actions@1:{}:{}:{}:{}:{}",
            code,
            helpers,
            include_str!("actions.rs"),
            include_str!("execute.rs"),
            concat!(
                include_str!("../flowkit.rs"),
                include_str!("../semantic_clock.rs"),
                include_str!("../sampling.rs")
            )
        )
        .as_bytes(),
    )
}
pub fn key(
    task: &Task,
    inputs: &BTreeMap<String, String>,
    runtime: &BTreeMap<String, String>,
    mock: bool,
) -> Result<String> {
    // Only provider settings relevant to this producer affect its identity.
    let settings: BTreeMap<_, _> = runtime
        .iter()
        .filter(|(name, _)| match &task.action {
            Action::Voice { .. } => {
                name.starts_with("TTS_") || name.starts_with("MIMO_") || name.starts_with("OPENAI_")
            }
            Action::Image { .. } => {
                name.starts_with("IMAGE_")
                    || name.starts_with("FLOWKIT_IMAGE")
                    || name.as_str() == "FLOWKIT_BASE_URL"
                    || name.starts_with("OPENAI_")
            }
            Action::ForceAlign { .. } => name.starts_with("ALIGNMENT_"),
            Action::Video { .. } => {
                name.starts_with("VIDEO_")
                    || name.starts_with("FLOWKIT_VIDEO")
                    || name.as_str() == "FLOWKIT_BASE_URL"
                    || name.starts_with("OPENAI_")
            }
            _ => false,
        })
        .collect();
    hash(
        &json!({"task":task.id,"action":task.action,"revision":task.revision,"implementation":implementation(&task.action),"inputs":inputs,"runtime":settings,"mock":mock}),
    )
}
fn id(id: &str) -> Result<()> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        || id == "."
        || id == ".."
    {
        bail!("Invalid task/output id {}", id);
    }
    Ok(())
}
pub fn contract(task: &Task, types: &BTreeMap<String, Kind>) -> Result<()> {
    let require = |name: &str, kind: Kind| -> Result<()> {
        if types.get(name) != Some(&kind) {
            bail!("{} input {} must be {:?}", task.id, name, kind);
        }
        Ok(())
    };
    let optional = |name: &str, kind: Kind| -> Result<()> {
        if types.contains_key(name) {
            require(name, kind)?;
        }
        Ok(())
    };
    let dynamic = |prefix: &str, kind: Kind| -> Result<()> {
        for (name, ty) in types {
            if name.starts_with(prefix) && *ty != kind {
                bail!("{} input {} must be {:?}", task.id, name, kind);
            }
        }
        Ok(())
    };
    let allowed: Vec<&str> = match &task.action {
        Action::Json { .. } => return Ok(()),
        Action::Voice {
            voice, language, ..
        } => {
            if voice.is_empty() || language.is_empty() {
                bail!("Voice and language are explicit");
            }
            require("script", Kind::Script)?;
            vec!["script"]
        }
        Action::Image { seed, .. } => {
            if seed.is_some_and(|seed| seed == 0 || seed > 1_000_000_000) {
                bail!("Image seed must be between 1 and 1000000000");
            }
            dynamic("reference:", Kind::Image)?;
            vec!["reference:"]
        }
        Action::ExtractFrame { at_seconds } => {
            if !at_seconds.is_finite() || *at_seconds < 0.0 {
                bail!("extract-frame needs a finite, nonnegative at_seconds");
            }
            require("video", Kind::Video)?;
            vec!["video"]
        }
        Action::Video {
            spec,
            source_edit,
            flow_project_id,
        } => {
            if source_edit.as_ref().is_some_and(|edit| {
                edit.uploads
                    .iter()
                    .any(|binding| binding.project_id != *flow_project_id)
            }) {
                bail!("Source edit upload bindings must match the declared Flow project");
            }
            if !spec.duration_seconds.is_finite()
                || spec.duration_seconds <= 0.0
                || !spec.fps.is_finite()
                || spec.fps <= 0.0
                || spec.width == 0
                || spec.height == 0
            {
                bail!("Invalid video clock or canvas");
            }
            optional("dialogue", Kind::Script)?;
            optional("first_frame", Kind::Image)?;
            optional("last_frame", Kind::Image)?;
            optional("source_video", Kind::Video)?;
            dynamic("reference:", Kind::Image)?;
            if spec.first_frame_path.is_some() {
                bail!("Graph video uses first_frame input, not paths in params");
            }
            let mut check = spec.clone();
            check.first_frame_path = types
                .contains_key("first_frame")
                .then(|| PathBuf::from("declared-input"));
            let extras = crate::video_director::VideoInputs {
                source_video: types
                    .contains_key("source_video")
                    .then(|| PathBuf::from("declared-source-video")),
                source_edit: source_edit.clone(),
                last_frame: types
                    .contains_key("last_frame")
                    .then(|| PathBuf::from("declared-last-frame")),
                reference_images: types
                    .keys()
                    .filter(|n| n.starts_with("reference:"))
                    .map(|n| PathBuf::from(n))
                    .collect(),
            };
            crate::video_director::validate_inputs(&check, &extras)?;
            vec![
                "first_frame",
                "last_frame",
                "reference:",
                "dialogue",
                "source_video",
            ]
        }
        Action::Transcribe { .. } => {
            let ty = types.get("audio").context("Transcribe needs audio")?;
            if !matches!(ty, Kind::Audio | Kind::Video) {
                bail!("Transcribe needs audio/video");
            }
            vec!["audio"]
        }
        Action::ForceAlign {
            expected_model,
            min_word_score,
        } => {
            if expected_model.is_empty()
                || !min_word_score.is_finite()
                || !(0.0..=1.0).contains(min_word_score)
            {
                bail!("Invalid acoustic alignment configuration");
            }
            require("audio", Kind::Audio)?;
            require("transcript", Kind::Json)?;
            vec!["audio", "transcript"]
        }
        Action::AdoptPerformance { fps } => {
            if !fps.is_finite() || *fps <= 0.0 {
                bail!("Invalid performance frame clock");
            }
            require("video", Kind::Video)?;
            vec!["video"]
        }
        Action::AlignSpeech { fps } => {
            if !fps.is_finite() || *fps <= 0.0 {
                bail!("Invalid alignment fps");
            }
            require("script", Kind::Script)?;
            require("audio", Kind::Audio)?;
            require("transcript", Kind::Json)?;
            vec!["script", "audio", "transcript"]
        }
        Action::ProjectTiming { fps, gap } => {
            if !fps.is_finite() || *fps <= 0.0 || !gap.is_finite() || *gap < 0.0 {
                bail!("Invalid projection clock");
            }
            require("script", Kind::Script)?;
            require("layout", Kind::Project)?;
            dynamic("alignment:", Kind::Json)?;
            vec!["script", "layout", "alignment:"]
        }
        Action::Realign { fps, gap } => {
            if !fps.is_finite() || *fps <= 0.0 || !gap.is_finite() || *gap < 0.0 {
                bail!("Invalid realign clock");
            }
            require("script", Kind::Script)?;
            require("layout", Kind::Project)?;
            dynamic("voice:", Kind::Audio)?;
            vec!["script", "layout", "voice:"]
        }
        Action::Mix { .. } => {
            require("script", Kind::Script)?;
            require("project", Kind::Project)?;
            optional("bgm", Kind::Audio)?;
            dynamic("sfx:", Kind::Audio)?;
            dynamic("voice:", Kind::Audio)?;
            vec!["script", "project", "bgm", "voice:", "sfx:"]
        }
        Action::CaptionSchedule { .. } => {
            require("script", Kind::Script)?;
            optional("transcript", Kind::Json)?;
            optional("clock", Kind::Json)?;
            if types.contains_key("clock") == types.contains_key("transcript") {
                bail!("Caption schedule needs exactly one clock or transcript");
            }
            vec!["script", "transcript", "clock"]
        }
        Action::RenderCaptions { .. } => {
            require("schedule", Kind::Json)?;
            require("project", Kind::Project)?;
            vec!["schedule", "project"]
        }
        Action::RenderHtml { frames, config } => {
            if *frames == 0
                || config.fps <= 0.0
                || !config.fps.is_finite()
                || config.width == 0
                || config.height == 0
            {
                bail!("Invalid capture clock");
            }
            require("html", Kind::Html)?;
            dynamic("asset:", Kind::Image)?;
            optional("clock", Kind::Json)?;
            vec!["html", "asset:", "clock"]
        }
        Action::Compose => {
            require("project", Kind::Project)?;
            require("audio", Kind::Audio)?;
            dynamic("overlay:", Kind::Video)?;
            for (name, ty) in types {
                if name.starts_with("background:") && !matches!(ty, Kind::Image | Kind::Video) {
                    bail!("Background needs image/video");
                }
            }
            vec!["project", "audio", "background:", "overlay:"]
        }
        Action::Review { .. } => {
            require("project", Kind::Project)?;
            require("video", Kind::Video)?;
            optional("evidence", Kind::Json)?;
            vec!["project", "video", "evidence"]
        }
    };
    for name in types.keys() {
        if !allowed.iter().any(|a| {
            if a.ends_with(':') {
                name.starts_with(a)
            } else {
                name == a
            }
        }) {
            bail!("{} has unsupported input {}", task.id, name);
        }
    }
    Ok(())
}

pub fn compile(path: &Path, mock: bool) -> Result<Plan> {
    let run_path = path.canonicalize()?;
    let run = source::read_run(&run_path)?;
    let author_path = run.author.canonicalize()?;
    let author_bytes = std::fs::read(&author_path)?;
    let author: Author = serde_json::from_slice(&author_bytes)?;
    if author.format != "clone-any-video.author@1" {
        bail!("Expected production Author graph clone-any-video.author@1");
    }
    if run.targets.is_empty() {
        bail!("Run needs explicit targets");
    }
    let mut tasks = BTreeMap::new();
    let mut outputs = author.outputs;
    for task in author.tasks {
        id(&task.id)?;
        if tasks.insert(task.id.clone(), task).is_some() {
            bail!("Duplicate task id");
        }
    }
    for task in tasks.values() {
        for port in task.action.outputs().keys() {
            let name = format!("{}.{}", task.id, port);
            if outputs.contains_key(&name) {
                bail!("Output alias collides with canonical port {}", name);
            }
            outputs.insert(
                name,
                Output {
                    task: task.id.clone(),
                    port: port.clone(),
                },
            );
        }
    }
    for (name, output) in &outputs {
        id(name)?;
        let task = tasks
            .get(&output.task)
            .context("Output references missing task")?;
        if !task.action.outputs().contains_key(&output.port) {
            bail!("Unknown output port {}.{}", output.task, output.port);
        }
    }
    let canonical = |name: &str| -> Result<String> {
        let o = outputs
            .get(name)
            .with_context(|| format!("Unknown logical output {}", name))?;
        Ok(format!("{}.{}", o.task, o.port))
    };
    let mut bindings = BTreeMap::new();
    for (output, candidate) in &run.satisfy {
        let name = canonical(output)?;
        if !run.candidates.contains_key(candidate) {
            bail!("Unknown candidate {}", candidate);
        }
        if bindings.insert(name, candidate.clone()).is_some() {
            bail!("Logical output satisfied twice through aliases");
        }
    }
    for target in &run.targets {
        canonical(target)?;
    }
    // Validate the entire Author graph, including unused components. Selection only prunes execution.
    let mut edges = BTreeMap::new();
    for task in tasks.values() {
        let mut types = BTreeMap::new();
        let mut dependencies = Vec::new();
        for (name, input) in &task.inputs {
            let ty = match input {
                Input::File { ty, .. } => *ty,
                Input::Script { .. } => Kind::Script,
                Input::Output { output } => {
                    let o = outputs
                        .get(output)
                        .with_context(|| format!("Unknown input output {}", output))?;
                    dependencies.push(o.task.clone());
                    tasks[&o.task].action.outputs()[&o.port]
                }
            };
            types.insert(name.clone(), ty);
        }
        contract(task, &types)?;
        edges.insert(task.id.clone(), dependencies);
    }
    fn cycles(
        name: &str,
        edges: &BTreeMap<String, Vec<String>>,
        active: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
    ) -> Result<()> {
        if done.contains(name) {
            return Ok(());
        }
        if !active.insert(name.into()) {
            bail!("Production dependency cycle at {}", name);
        }
        for parent in &edges[name] {
            cycles(parent, edges, active, done)?;
        }
        active.remove(name);
        done.insert(name.into());
        Ok(())
    }
    let mut done = BTreeSet::new();
    for task in tasks.keys() {
        cycles(task, &edges, &mut BTreeSet::new(), &mut done)?;
    }
    let cache = run.reuse.as_ref().map(|p| source::result(p)).transpose()?;
    let mut builder = Builder {
        base: author_path.parent().unwrap().into(),
        runtime: runtime()?,
        mock,
        tasks,
        outputs: outputs.clone(),
        run,
        bindings,
        cache,
        planned: BTreeMap::new(),
        order: Vec::new(),
        provided: BTreeMap::new(),
        known: BTreeMap::new(),
        blocked: Vec::new(),
    };
    for target in builder.run.targets.clone() {
        builder.output(&target)?;
    }
    Ok(Plan {
        format: "clone-any-video.plan@1".into(),
        run: run_path,
        author: author_path,
        source_digest: crate::artifacts::digest(&author_bytes),
        mock,
        runtime: builder.runtime,
        targets: builder.run.targets,
        outputs,
        provided: builder.provided,
        tasks: builder
            .order
            .into_iter()
            .map(|id| builder.planned.remove(&id).unwrap())
            .collect(),
        blocked: builder.blocked,
    })
}
struct Builder {
    base: PathBuf,
    runtime: BTreeMap<String, String>,
    mock: bool,
    tasks: BTreeMap<String, Task>,
    outputs: BTreeMap<String, Output>,
    run: Run,
    bindings: BTreeMap<String, String>,
    cache: Option<BuildResult>,
    planned: BTreeMap<String, PlannedTask>,
    order: Vec<String>,
    provided: BTreeMap<String, Artifact>,
    known: BTreeMap<String, Artifact>,
    blocked: Vec<String>,
}
impl Builder {
    fn output(&mut self, name: &str) -> Result<Option<Artifact>> {
        let o = self.outputs.get(name).context("Unknown output")?.clone();
        let canonical = format!("{}.{}", o.task, o.port);
        let ty = self.tasks[&o.task].action.outputs()[&o.port];
        if let Some(a) = self.known.get(&canonical) {
            return Ok(Some(a.clone()));
        }
        if let Some(candidate) = self.bindings.get(&canonical) {
            let a = match &self.run.candidates[candidate] {
                Candidate::File { path, ty } => file(path, *ty)?,
                Candidate::Result { result, output } => {
                    let r = source::result(result)?;
                    let a = r
                        .outputs
                        .get(output)
                        .context("Result has no selected output")?
                        .clone();
                    verify(&a)?;
                    a
                }
            };
            if a.ty != ty {
                bail!("Candidate for {} has wrong type", name);
            }
            if !self.mock && a.ty.media() && a.mode != crate::artifacts::Mode::Production {
                self.blocked.push(format!(
                    "{} supplied media is {}; admit actual production media first",
                    name,
                    a.mode.name()
                ));
            }
            self.provided.insert(canonical.clone(), a.clone());
            self.known.insert(canonical, a.clone());
            return Ok(Some(a));
        }
        self.task(&o.task)?;
        Ok(self.known.get(&canonical).cloned())
    }
    fn task(&mut self, id: &str) -> Result<()> {
        if self.planned.contains_key(id) {
            return Ok(());
        }
        let task = self.tasks[id].clone();
        let mut inputs = BTreeMap::new();
        let mut digests = BTreeMap::new();
        let mut reasons = Vec::new();
        let mut uncertain = false;
        for (name, input) in &task.inputs {
            match input {
                Input::Output { output } => {
                    let known = self.output(output)?;
                    let o = &self.outputs[output];
                    let ty = self.tasks[&o.task].action.outputs()[&o.port];
                    if let Some(a) = known {
                        digests.insert(name.clone(), input_identity(&a)?);
                    } else {
                        uncertain = true;
                        reasons.push(format!("Input {} awaits {}", name, output));
                    }
                    inputs.insert(
                        name.clone(),
                        PlannedInput::Output {
                            output: output.clone(),
                            ty,
                        },
                    );
                }
                _ => {
                    let m = material(input, &self.base)?;
                    if !self.mock
                        && m.artifact.ty.media()
                        && m.artifact.mode != crate::artifacts::Mode::Production
                    {
                        self.blocked.push(format!(
                            "{} input {} is {} media; admit actual production input",
                            id,
                            name,
                            m.artifact.mode.name()
                        ));
                    }
                    digests.insert(name.clone(), input_identity(&m.artifact)?);
                    inputs.insert(name.clone(), PlannedInput::Material { material: m });
                }
            }
        }
        if let Some(PlannedInput::Material { material }) = inputs.get("script") {
            if let Some(value) = &material.value {
                let doc: crate::script::ScriptDocument = serde_json::from_value(value.clone())?;
                if matches!(
                    task.action,
                    Action::AlignSpeech { .. } | Action::ProjectTiming { .. }
                ) {
                    if doc
                        .segments
                        .iter()
                        .any(|s| s.turns.iter().all(|t| t.units.is_empty()))
                    {
                        bail!("Shared timing needs full/timing Script view, not speech projection");
                    }
                    if matches!(task.action, Action::AlignSpeech { .. }) && doc.segments.len() != 1
                    {
                        bail!("align-speech requires one explicit segment");
                    }
                    if matches!(task.action, Action::ProjectTiming { .. }) {
                        for segment in &doc.segments {
                            if !inputs.contains_key(&format!("alignment:{}", segment.id)) {
                                bail!("Missing alignment:{}", segment.id);
                            }
                        }
                        if matches!(
                            task.inputs.get("layout"),
                            Some(Input::File {
                                view: FileView::ProjectTiming,
                                ..
                            })
                        ) {
                            bail!("project-timing producer needs the full layout, including semantic bindings");
                        }
                    }
                }
                if matches!(task.action, Action::Voice { .. })
                    && (doc.segments.len() != 1 || doc.segments[0].turns.len() != 1)
                {
                    bail!("{} Voice requires exactly one Script segment/turn", id);
                }
                if matches!(task.action, Action::Realign { .. } | Action::Mix { .. }) {
                    for segment in &doc.segments {
                        if matches!(task.action, Action::Mix { .. })
                            && segment.spoken_text.trim().is_empty()
                        {
                            continue;
                        }
                        if !inputs.contains_key(&format!("voice:{}", segment.id)) {
                            bail!("{} missing declared voice:{} input", id, segment.id);
                        }
                    }
                }
            }
        }
        if let Action::Video { spec, .. } = &task.action {
            if spec.relationship != crate::video_director::VideoGenRelationship::SourceVideoEdit
                && self.runtime["VIDEO_PROVIDER"] == "flowkit"
            {
                crate::flowkit::generation_options(
                    spec.duration_seconds,
                    if task.inputs.contains_key("first_frame") { &self.runtime["FLOWKIT_VIDEO_FAMILY"] } else { "omni_flash" },
                    &self.runtime["FLOWKIT_VIDEO_RESOLUTION"],
                    &self.runtime["FLOWKIT_VIDEO_OMNI_VERSION"],
                )?;
            }
            if spec.relationship == crate::video_director::VideoGenRelationship::SourceVideoEdit
                && task.inputs.contains_key("dialogue")
            {
                self.blocked.push(format!("{}: Flow policy prevents editing or generating speech for people in uploaded videos; the current SourceVideoEdit adapter supports visual edits with original audio only", id));
            }
            if matches!(
                spec.relationship,
                crate::video_director::VideoGenRelationship::FirstLastFrame
                    | crate::video_director::VideoGenRelationship::ReferenceImages
                    | crate::video_director::VideoGenRelationship::SourceVideoEdit
            ) && (self.mock || self.runtime["VIDEO_PROVIDER"] != "flowkit")
            {
                self.blocked.push(format!(
                    "{} needs the real FlowKit advanced video-input adapter",
                    id
                ));
            }
        }
        if self.mock
            && matches!(task.action, Action::Video { .. })
            && task.inputs.contains_key("dialogue")
        {
            self.blocked.push(format!(
                "{} requires a real speaking video; no mock sound-picture performance",
                id
            ));
        }
        if self.mock && matches!(task.action, Action::ForceAlign { .. }) {
            self.blocked.push(format!("{} requires the real acoustic service; select explicit fixture evidence for offline tests",id));
        }
        let key = if uncertain {
            None
        } else {
            Some(key(&task, &digests, &self.runtime, self.mock)?)
        };
        let old = self.cache.as_ref().and_then(|r| r.tasks.get(id)).cloned();
        let mut cached = None;
        if let (Some(key), Some(old)) = (&key, &old) {
            if key == &old.request_key
                && matches!(old.state, State::Succeeded | State::Reused)
                && task.action.outputs().iter().all(|(p, k)| {
                    old.outputs
                        .get(p)
                        .is_some_and(|a| a.ty == *k && verify(a).is_ok())
                })
            {
                cached = Some(old.clone());
            } else if key == &old.request_key
                && task.action.external()
                && !self.mock
                && matches!(old.state, State::Running | State::Failed)
            {
                self.blocked.push(format!("{} has a prior unfinished/failed generation attempt; inspect its receipt before a new revision or explicit candidate",id));
            }
        }
        let decision = if let Some(record) = &cached {
            for (port, a) in &record.outputs {
                let canonical = format!("{}.{}", id, port);
                if !self.bindings.contains_key(&canonical) {
                    self.known.insert(canonical, a.clone());
                }
            }
            reasons.push(
                "Explicit reuse Result has matching producer, parameters and input bytes".into(),
            );
            Decision::Reuse
        } else {
            if let Some(old) = &old {
                for (name, digest) in &digests {
                    if old.inputs.get(name) != Some(digest) {
                        reasons.push(format!("Input {} content changed", name));
                    }
                }
                if reasons.is_empty() {
                    reasons.push(
                        "Producer/parameters/revision changed or cached files unavailable".into(),
                    );
                }
            } else {
                reasons
                    .push("No accepted task output in the explicitly selected reuse Result".into());
            }
            if !self.mock {
                let missing = match &task.action {
                    Action::Voice { .. } => {
                        let provider = &self.runtime["TTS_PROVIDER"];
                        if !matches!(provider.as_str(), "mimo" | "openai") {
                            self.blocked
                                .push(format!("{} unsupported TTS provider", id));
                        }
                        if provider == "mimo" && self.runtime["MIMO_TTS_MODEL"] != "mimo-v2.5-tts" {
                            self.blocked
                                .push(format!("{} voice design/clone inputs are not wired", id));
                        }
                        Some(if provider == "mimo" {
                            "MIMO_API_KEY"
                        } else {
                            "OPENAI_API_KEY"
                        })
                    }
                    Action::Image { .. } | Action::Video { .. } => {
                        let provider = if matches!(task.action, Action::Image { .. }) {
                            &self.runtime["IMAGE_PROVIDER"]
                        } else {
                            &self.runtime["VIDEO_PROVIDER"]
                        };
                        if !matches!(provider.as_str(), "flowkit" | "openai") {
                            self.blocked
                                .push(format!("{} unsupported media provider", id));
                        }
                        if provider == "openai" {
                            if matches!(task.action, Action::Image { .. })
                                && !task.inputs.is_empty()
                            {
                                self.blocked.push(format!(
                                    "{} OpenAI image adapter cannot carry reference inputs",
                                    id
                                ));
                            }
                            Some("OPENAI_API_KEY")
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(key) = missing {
                    if std::env::var(key).unwrap_or_default().trim().is_empty() {
                        self.blocked.push(format!(
                            "{} requires configured {} (secret is not stored)",
                            id, key
                        ));
                    }
                }
            }
            Decision::Execute
        };
        self.order.push(id.into());
        self.planned.insert(
            id.into(),
            PlannedTask {
                task,
                decision,
                reasons,
                inputs,
                request_key: key,
                cached,
                previous: old,
            },
        );
        Ok(())
    }
}
