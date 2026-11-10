//! Local FlowKit bridge. Browser sessions and throttling belong to FlowKit.
use crate::{
    image_director::{
        assemble_prompt, compute_prompt_hash, ImageGenerationRequest, ImageGenerationResult,
    },
    video_director::{
        assemble_video_prompt, compute_video_prompt_hash, VideoGenerationRequest,
        VideoGenerationResult,
    },
};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use std::hash::{Hash, Hasher};
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub fn selected(kind: &str) -> Result<bool> {
    match std::env::var(format!("{}_PROVIDER", kind))
        .unwrap_or_else(|_| "openai".into())
        .as_str()
    {
        "flowkit" => Ok(true),
        "openai" => Ok(false),
        other => bail!("Unsupported {}_PROVIDER '{}'", kind, other),
    }
}

struct Client {
    http: reqwest::Client,
    base: String,
}
impl Client {
    fn new() -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(180))
                .build()?,
            base: std::env::var("FLOWKIT_BASE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8100".into())
                .trim_end_matches('/')
                .into(),
        })
    }
    async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let r = self
            .http
            .post(format!("{}{}", self.base, path))
            .json(body)
            .send()
            .await
            .context("FlowKit request failed")?;
        let status = r.status();
        if !status.is_success() {
            bail!(
                "FlowKit {} returned {}: {}",
                path,
                status,
                r.text().await.unwrap_or_default()
            );
        }
        let v: Value = r.json().await.context("Invalid FlowKit JSON")?;
        if let Some(e) = v.get("error") {
            bail!("FlowKit {}: {}", path, e);
        }
        Ok(v)
    }
    async fn project(&self, explicit: Option<&str>, output: &Path) -> Result<String> {
        let binding = workspace(output).join(".flowkit-project.json");
        let pinned = if binding.exists() {
            Some(
                serde_json::from_slice::<Value>(&std::fs::read(&binding)?)?["project_id"]
                    .as_str()
                    .context("Invalid FlowKit project binding")?
                    .to_owned(),
            )
        } else {
            None
        };
        let requested = explicit
            .map(str::to_owned)
            .or_else(|| std::env::var("FLOWKIT_PROJECT_ID").ok())
            .or(pinned.clone());
        let status: Value = self
            .http
            .get(format!("{}/api/flow/status", self.base))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if status["connected"] != true {
            bail!("FlowKit extension disconnected");
        }
        if status.pointer("/generation_throttle/cooldown_active") == Some(&json!(true)) {
            bail!("FlowKit generation cooldown is active");
        }
        let id = requested
            .or_else(|| status["flow_project_id"].as_str().map(str::to_owned))
            .context("No FlowKit project selected; use --flow-project-id")?;
        validate_uuid(&id)?;
        if pinned.is_some_and(|old| old != id) {
            bail!("FlowKit workspace is bound to another project; choose a new workspace instead of splitting asset identity");
        }
        std::fs::create_dir_all(parent(&binding))?;
        if !binding.exists() {
            let staged = tempfile::NamedTempFile::new_in(parent(&binding))?;
            std::fs::write(
                staged.path(),
                serde_json::to_vec_pretty(
                    &json!({"format":"clone-any-video.flowkit-project@1","project_id":id}),
                )?,
            )?;
            if staged.persist_noclobber(&binding).is_err() {
                let current: Value = serde_json::from_slice(&std::fs::read(&binding)?)?;
                if current["project_id"] != id {
                    bail!("Concurrent FlowKit project binding conflict; no generation submitted");
                }
            }
        }
        Ok(id)
    }
    async fn download(&self, url: &str) -> Result<Vec<u8>> {
        // Retry only idempotent media GETs, never generation submissions.
        // A CDN may close a connection before its declared Content-Length.
        let mut last_error = None;
        for attempt in 0..3 {
            match self.http.get(url).send().await {
                Ok(response) => {
                    let response = response
                        .error_for_status()
                        .context("Media download HTTP error")?;
                    match response.bytes().await {
                        Ok(bytes) => return Ok(bytes.to_vec()),
                        Err(error) => last_error = Some(error),
                    }
                }
                Err(error) => last_error = Some(error),
            }
            if attempt < 2 {
                eprintln!(
                    "Media download interrupted; retrying GET ({}/3)",
                    attempt + 2
                );
                tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
            }
        }
        Err(last_error.context("Media download failed")?.into())
    }
}
fn workspace(output: &Path) -> std::path::PathBuf {
    let parent = parent(output);
    for dir in parent.ancestors() {
        if dir.file_name().is_some_and(|n| n == "assets") {
            return dir.parent().unwrap_or(parent).to_path_buf();
        }
        if dir.join(".flowkit-project.json").exists() {
            return dir.to_path_buf();
        }
    }
    parent.to_path_buf()
}
pub(crate) fn validate_uuid(id: &str) -> Result<()> {
    let parts: Vec<_> = id.split('-').collect();
    if parts.len() != 5
        || !parts
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(p, n)| p.len() == n && p.chars().all(|c| c.is_ascii_hexdigit()))
    {
        bail!("Expected FlowKit UUID, got '{}'", id);
    }
    Ok(())
}
async fn upload(c: &Client, path: &Path, project: &str) -> Result<String> {
    let bytes = tokio::fs::read(path).await?;
    let format = image::guess_format(&bytes).context("Reference must be a decodable image")?;
    let _ = image::load_from_memory(&bytes)?;
    let mime = match format {
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::Png => "image/png",
        _ => bail!("Use PNG or JPEG references"),
    };
    let result=c.post("/api/flow/upload-image",&json!({"image_base64":STANDARD.encode(bytes),"mime_type":mime,"file_name":path.file_name().unwrap_or_default().to_string_lossy(),"project_id":project})).await?;
    let id = result["media_id"]
        .as_str()
        .context("Reference upload returned no media_id")?
        .to_owned();
    validate_uuid(&id)?;
    Ok(id)
}

fn bound_upload(
    path: &Path,
    project: &str,
    bindings: &[crate::video_director::FlowUploadBinding],
) -> Result<Option<String>> {
    let digest = crate::artifacts::content_digest(path)?;
    let mut matched = None;
    for binding in bindings {
        validate_uuid(&binding.project_id)?;
        validate_uuid(&binding.media_id)?;
        if binding.project_id != project {
            bail!("An upload binding belongs to a different Flow project");
        }
        if binding.content_digest == digest {
            if matched.is_some() {
                bail!("Ambiguous Flow upload bindings for content {}", digest);
            }
            matched = Some(binding.media_id.clone());
        }
    }
    Ok(matched)
}

#[cfg(test)]
mod source_edit_tests {
    use super::*;
    #[test]
    fn omni12_allows_one_continuous_speaking_take_without_changing_legacy_windows() {
        assert_eq!(generation_options(12.5,"omni_flash","360p","1.2").unwrap()["duration_s"],13);
        assert_eq!(generation_options(1.0,"omni_flash","360p","1.2").unwrap()["duration_s"],3);
        assert_eq!(generation_options(5.0,"omni_flash","360p","1.1").unwrap()["duration_s"],6);
        for duration in [f64::NAN,0.0,15.01] {
            assert!(generation_options(duration,"omni_flash","360p","1.2").is_err());
        }
        assert!(generation_options(13.0,"omni_flash","360p","1.1").is_err());
        assert!(generation_options(8.0,"veo","360p","1.2").is_err());
    }
    #[test]
    fn existing_upload_requires_matching_bytes_project_and_unambiguous_identity() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.mp4");
        std::fs::write(&source, b"declared source bytes").unwrap();
        let project = "11111111-2222-3333-4444-555555555555";
        let binding = crate::video_director::FlowUploadBinding {
            project_id: project.into(),
            media_id: "22222222-3333-4444-5555-666666666666".into(),
            content_digest: crate::artifacts::content_digest(&source).unwrap(),
        };
        assert_eq!(
            bound_upload(&source, project, &[binding.clone()]).unwrap(),
            Some(binding.media_id.clone())
        );
        assert!(bound_upload(
            &source,
            "33333333-4444-5555-6666-777777777777",
            &[binding.clone()]
        )
        .is_err());
        assert!(bound_upload(&source, project, &[binding.clone(), binding.clone()]).is_err());
        std::fs::write(&source, b"changed source").unwrap();
        assert_eq!(bound_upload(&source, project, &[binding]).unwrap(), None);
    }
}

async fn upload_edit_input(
    c: &Client,
    path: &Path,
    project: &str,
    bindings: &[crate::video_director::FlowUploadBinding],
    video: bool,
) -> Result<String> {
    if let Some(id) = bound_upload(path, project, bindings)? {
        eprintln!("Reusing Flow upload {} for {:?}", id, path.file_name());
        return Ok(id);
    }
    if !video {
        return upload(c, path, project).await;
    }
    let bytes = tokio::fs::read(path).await?;
    if bytes.is_empty() || bytes.len() > 20 * 1024 * 1024 {
        bail!("Flow source video upload must contain 1 byte to 20 MiB");
    }
    let mime = match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "wmv" => "video/x-ms-wmv",
        _ => bail!("Source video uploads need MP4, MOV, AVI or WMV"),
    };
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        )
        .mime_str(mime)?;
    let response = c
        .http
        .post(format!("{}/api/flow/upload-video-file", c.base))
        .multipart(
            reqwest::multipart::Form::new()
                .text("project_id", project.to_owned())
                .part("file", part),
        )
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        bail!(
            "Flow source upload returned {}: {}",
            status,
            response.text().await?
        );
    }
    let result: Value = response.json().await?;
    let id = result["media_id"]
        .as_str()
        .context("Video upload returned no media_id")?;
    validate_uuid(id)?;
    Ok(id.to_owned())
}
fn aspect(width: u32, height: u32, video: bool) -> String {
    format!(
        "{}_ASPECT_RATIO_{}",
        if video { "VIDEO" } else { "IMAGE" },
        if !video && width == height {
            "SQUARE"
        } else if width >= height {
            "LANDSCAPE"
        } else {
            "PORTRAIT"
        }
    )
}
async fn save_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    tokio::fs::write(path, serde_json::to_vec_pretty(value)?).await?;
    Ok(())
}
fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

pub async fn image_with_inputs(
    req: &ImageGenerationRequest,
    refs: &[std::path::PathBuf],
    media_ids: &[String],
    project: Option<&str>,
) -> Result<ImageGenerationResult> {
    let start = Instant::now();
    let c = Client::new()?;
    let pid = c.project(project, &req.output_path).await?;
    let mut reference_ids = media_ids.to_vec();
    for id in &reference_ids {
        validate_uuid(id)?;
    }
    for path in refs {
        reference_ids.push(upload(&c, path, &pid).await?);
    }
    let prompt = assemble_prompt(&req.spec);
    let (w, h) =
        crate::image_director::resolve_dimensions(&req.spec.aspect_ratio, &req.spec.resolution);
    if req
        .seed
        .is_some_and(|seed| seed == 0 || seed > 1_000_000_000)
    {
        bail!("FlowKit image seed must be between 1 and 1000000000");
    }
    tokio::fs::create_dir_all(parent(&req.output_path)).await?;
    let response=c.post("/api/flow/generate-image",&json!({"prompt":prompt,"project_id":pid,"aspect_ratio":aspect(w,h,false),"image_model":std::env::var("FLOWKIT_IMAGE_MODEL").ok(),"count":1,"seed":req.seed,"reference_media_ids":reference_ids})).await?;
    save_json(&req.output_path.with_extension("flowkit.json"), &response).await?;
    let item = response
        .pointer("/media/0/image/generatedImage")
        .context("FlowKit image response missing media[0].image.generatedImage")?;
    let bytes = if let Some(encoded) = item["encodedImage"].as_str() {
        STANDARD.decode(encoded)?
    } else {
        c.download(
            item["fifeUrl"]
                .as_str()
                .or_else(|| item["imageUri"].as_str())
                .context("FlowKit image response has no download URL")?,
        )
        .await?
    };
    let img = image::load_from_memory(&bytes).context("FlowKit returned invalid image")?;
    let stage = tempfile::Builder::new()
        .suffix(".png")
        .tempfile_in(parent(&req.output_path))?;
    img.save_with_format(stage.path(), image::ImageFormat::Png)?;
    stage.persist(&req.output_path).map_err(|e| e.error)?;
    let result = ImageGenerationResult {
        shot_id: req.shot_id.clone().unwrap_or_else(|| "direct".into()),
        output_path: req.output_path.to_string_lossy().into(),
        metadata_path: req
            .output_path
            .with_extension("json")
            .to_string_lossy()
            .into(),
        width: img.width(),
        height: img.height(),
        aspect_ratio: req.spec.aspect_ratio.clone(),
        provider: "flowkit".into(),
        model: std::env::var("FLOWKIT_IMAGE_MODEL").unwrap_or_else(|_| "flowkit-default".into()),
        prompt_hash: compute_prompt_hash(&prompt),
        elapsed_ms: start.elapsed().as_millis() as u64,
        spec: req.spec.clone(),
        full_prompt: prompt,
    };
    save_json(&req.output_path.with_extension("json"), &result).await?;
    crate::artifacts::record_derived(
        &req.output_path,
        crate::artifacts::combined(refs.iter().map(|p| p.as_path())),
        "flowkit",
        refs,
        json!({"project_id":pid,"reference_media_ids":reference_ids,"model":result.model,"watermark":"unverified"}),
    )?;
    Ok(result)
}

fn video_url(v: &Value) -> Option<&str> {
    if let Some(url) = v
        .pointer("/operation/metadata/video/fifeUrl")
        .and_then(Value::as_str)
    {
        return Some(url);
    }
    if v.get("done") == Some(&json!(true))
        || v.get("status").and_then(Value::as_str) == Some("MEDIA_GENERATION_STATUS_SUCCESSFUL")
    {
        if let Some(url) = v.pointer("/media/url").and_then(Value::as_str) {
            return Some(url);
        }
    }
    match v {
        Value::Array(a) => a.iter().find_map(video_url),
        Value::Object(o) => o.values().find_map(video_url),
        _ => None,
    }
}
fn failed(v: &Value) -> bool {
    if v.get("status")
        .and_then(Value::as_str)
        .is_some_and(|s| s.ends_with("FAILED") || s.ends_with("CANCELLED"))
    {
        return true;
    }
    match v {
        Value::Array(a) => a.iter().any(failed),
        Value::Object(o) => o.values().any(failed),
        _ => false,
    }
}

/// Validate the request window before project creation or media upload.
pub fn generation_options(duration: f64, family: &str, resolution: &str, omni_version: &str) -> Result<Value> {
    if !matches!(family, "veo" | "omni_flash") {
        bail!("FLOWKIT_VIDEO_FAMILY must be veo or omni_flash");
    }
    if !matches!(resolution, "360p" | "720p") {
        bail!("FLOWKIT_VIDEO_RESOLUTION must be 360p or 720p");
    }
    if !matches!(omni_version, "1.1" | "1.2") || (family == "veo" && omni_version != "1.1") {
        bail!("FLOWKIT_VIDEO_OMNI_VERSION must be 1.1 or 1.2; 1.2 requires omni_flash");
    }
    let max = if omni_version == "1.2" { 15.0 } else { 10.0 };
    if !duration.is_finite() || duration <= 0.0 || duration > max {
        bail!("FlowKit {family} {omni_version} duration must be in (0,{max}] seconds");
    }
    let seconds = if omni_version == "1.2" {
        duration.ceil().max(3.0) as u32
    } else {
        [4, 6, 8, 10].into_iter().find(|d| *d as f64 >= duration).unwrap()
    };
    Ok(json!({"model_family":family,"duration_s":seconds,"resolution":resolution,"omni_version":omni_version}))
}

pub async fn video_with_project(
    req: &VideoGenerationRequest,
    project: Option<&str>,
) -> Result<VideoGenerationResult> {
    video_with_inputs(req, project, &crate::video_director::VideoInputs::default()).await
}
pub async fn video_with_inputs(
    req: &VideoGenerationRequest,
    project: Option<&str>,
    inputs: &crate::video_director::VideoInputs,
) -> Result<VideoGenerationResult> {
    crate::video_director::validate_inputs(&req.spec, inputs)?;
    let timeout_seconds: u64 = std::env::var("FLOWKIT_VIDEO_POLL_TIMEOUT_SECONDS")
        .unwrap_or_else(|_| "900".into())
        .parse()
        .context("Invalid FlowKit polling timeout")?;
    if timeout_seconds == 0 {
        bail!("FlowKit polling timeout must be positive");
    }
    let mut media: Vec<_> = req
        .spec
        .first_frame_path
        .iter()
        .chain(inputs.last_frame.iter())
        .chain(inputs.reference_images.iter())
        .cloned()
        .collect();
    // Decode and admit all references before HTTP or uploads.
    for p in &media {
        let _ = image::open(p)?;
        if crate::artifacts::mode(p) != crate::artifacts::Mode::Production {
            bail!("Video references must be admitted production media");
        }
    }

    if let Some(source) = &inputs.source_video {
        if crate::artifacts::mode(source) != crate::artifacts::Mode::Production {
            bail!("Source video must be admitted production media");
        }
        let probe = crate::probe::probe_media(source).await?;
        if probe.width.is_none()
            || inputs.source_edit.as_ref().unwrap().end_frame as f64 / 30.0
                > probe.duration_seconds + 1.0 / 30.0
        {
            bail!("Source edit range exceeds the actual source video");
        }
        media.push(source.clone());
    }

    let family = if req.spec.first_frame_path.is_none() {
        "omni_flash".into()
    } else {
        std::env::var("FLOWKIT_VIDEO_FAMILY").unwrap_or_else(|_| "omni_flash".into())
    };
    let resolution = std::env::var("FLOWKIT_VIDEO_RESOLUTION").unwrap_or_else(|_| "360p".into());
    let omni_version = std::env::var("FLOWKIT_VIDEO_OMNI_VERSION").unwrap_or_else(|_| "1.1".into());
    let options = if inputs.source_video.is_some() {
        if resolution != "360p" {
            bail!("Source video editing currently supports only the captured Omni 1.2 360p model");
        }
        Value::Null
    } else {
        generation_options(req.spec.duration_seconds, &family, &resolution, &omni_version)?
    };
    let start = Instant::now();
    let c = Client::new()?;
    let pid = c.project(project, &req.output_path).await?;
    let prompt = assemble_video_prompt(&req.spec);
    tokio::fs::create_dir_all(parent(&req.output_path)).await?;
    let receipt = req.output_path.with_extension("flowkit-job.json");
    // Keep an exact request identity; retrying a matching job resumes polling.
    let frame_digest = if let Some(p) = &req.spec.first_frame_path {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        tokio::fs::read(p).await?.hash(&mut h);
        Some(format!("{:016x}", h.finish()))
    } else {
        None
    };
    let mut identity = json!({"spec":req.spec,"first_frame_digest":frame_digest,"project_id":pid,"family":family,"resolution":resolution});
    if inputs.source_video.is_none() && omni_version == "1.2" {
        identity["omni_version"] = json!(omni_version);
    }
    if let Some(p) = &inputs.last_frame {
        identity["last_frame_digest"] = json!(crate::artifacts::content_digest(p)?);
    }
    if !inputs.reference_images.is_empty() {
        identity["reference_image_digests"] = json!(inputs
            .reference_images
            .iter()
            .map(|p| crate::artifacts::content_digest(p))
            .collect::<Result<Vec<_>>>()?);
    }
    if let Some(source) = &inputs.source_video {
        identity["source_video_digest"] = json!(crate::artifacts::content_digest(source)?);
        identity["source_edit"] = serde_json::to_value(&inputs.source_edit)?;
        identity["model"] = json!("macaron_cookie_edit_360p");
        // Validate explicit bindings even when a matching job can be resumed.
        for input in &media {
            bound_upload(input, &pid, &inputs.source_edit.as_ref().unwrap().uploads)?;
        }
    }

    let submitted_request: Value;
    let mut state = if receipt.exists() {
        let mut old: Value = serde_json::from_slice(&tokio::fs::read(&receipt).await?)?;
        // Legacy receipts predate the resolution setting and requested 720p.
        if old["request"].get("resolution").is_none() {
            old["request"]["resolution"] = json!("720p");
        }
        if old["request"] != identity {
            bail!("Existing FlowKit job belongs to different inputs; choose a new output path to start a new job");
        }
        submitted_request = old.get("submitted_request").cloned().unwrap_or(Value::Null);
        old["response"].clone()
    } else {
        let v = if let Some(source) = &inputs.source_video {
            let edit = inputs.source_edit.as_ref().unwrap();
            let source_id = upload_edit_input(&c, source, &pid, &edit.uploads, true).await?;
            let mut references = Vec::new();
            for p in &inputs.reference_images {
                references.push(upload_edit_input(&c, p, &pid, &edit.uploads, false).await?);
            }
            submitted_request = json!({"project_id":pid,"source_video_media_id":source_id,
                "prompt":prompt,"start_frame":edit.start_frame,"end_frame":edit.end_frame,
                "reference_media_ids":references,"aspect_ratio":aspect(req.spec.width,req.spec.height,true)});
            c.post("/api/flow/edit-video-omni", &submitted_request)
                .await?
        } else {
            let mut body = options.clone();
            body["prompt"] = json!(prompt);
            body["project_id"] = json!(pid);
            body["scene_id"] = json!(req.spec.shot_id);
            body["aspect_ratio"] = json!(aspect(req.spec.width,req.spec.height,true));
            let path = if !inputs.reference_images.is_empty() {
                let mut ids = Vec::new();
                for p in &inputs.reference_images {
                    ids.push(upload(&c, p, &pid).await?);
                }
                body["reference_media_ids"] = json!(ids);
                "/api/flow/generate-video-refs"
            } else if let Some(frame) = &req.spec.first_frame_path {
                body["start_image_media_id"] = json!(upload(&c, frame, &pid).await?);
                if let Some(last) = &inputs.last_frame {
                    body["end_image_media_id"] = json!(upload(&c, last, &pid).await?);
                }
                "/api/flow/generate-video"
            } else {
                "/api/flow/generate-video-omni-text"
            };
            submitted_request = body.clone();
            c.post(path, &body).await?
        };
        // Keep the submission acknowledgement distinct from mutable poll state.
        save_json(
            &req.output_path.with_extension("flowkit-submit.json"),
            &json!({"request":identity,"submitted_request":submitted_request,"response":v}),
        )
        .await?;
        save_json(
            &receipt,
            &json!({"request":identity,"submitted_request":submitted_request,"response":v}),
        )
        .await?;
        v
    };
    loop {
        if failed(&state) {
            bail!(
                "FlowKit video job failed; receipt retained at {:?}",
                receipt
            );
        }
        if let Some(url) = video_url(&state) {
            let bytes = c.download(url).await?;
            let stage = tempfile::Builder::new()
                .suffix(".mp4")
                .tempfile_in(parent(&req.output_path))?;
            tokio::fs::write(stage.path(), bytes).await?;
            let m = crate::probe::probe_media(stage.path()).await?;
            if m.width.is_none() || m.duration_seconds <= 0.0 {
                bail!("FlowKit downloaded invalid video");
            }
            stage.persist(&req.output_path).map_err(|e| e.error)?;
            let result = VideoGenerationResult {
                shot_id: req.spec.shot_id.clone(),
                output_path: req.output_path.to_string_lossy().into(),
                metadata_path: req
                    .output_path
                    .with_extension("json")
                    .to_string_lossy()
                    .into(),
                duration_seconds: m.duration_seconds,
                frames_count: (m.duration_seconds * m.fps.unwrap_or(0.0)).round() as u64,
                width: m.width.unwrap(),
                height: m.height.unwrap_or(0),
                fps: m.fps.unwrap_or(0.0),
                camera_motion: if inputs.source_video.is_some() {
                    "preserve_source".into()
                } else {
                    req.spec.camera_motion.as_str().into()
                },
                relationship: format!("{:?}", req.spec.relationship),
                provider: "flowkit".into(),
                model: identity
                    .get("model")
                    .unwrap_or(&identity["family"])
                    .as_str()
                    .unwrap()
                    .into(),
                prompt_hash: compute_video_prompt_hash(&prompt),
                elapsed_ms: start.elapsed().as_millis() as u64,
                full_prompt: prompt,
            };
            save_json(&req.output_path.with_extension("json"), &result).await?;
            crate::artifacts::record_derived(
                &req.output_path,
                crate::artifacts::combined(media.iter().map(|p| p.as_path())),
                "flowkit",
                &media,
                json!({"project_id":pid,"family":family,"resolution":resolution,"receipt":receipt,"generation_inputs":identity,"watermark":"unverified"}),
            )?;
            return Ok(result);
        }
        if start.elapsed() > Duration::from_secs(timeout_seconds) {
            bail!(
                "FlowKit video polling timed out; rerun the same command to resume {:?}",
                receipt
            );
        }
        let hint = state.get("flowkitPolling").unwrap_or(&state);
        let mut body = json!({"project_id":pid});
        if let Some(ops) = hint.get("operations").or_else(|| state.get("operations")) {
            body["operations"] = ops.clone();
        }
        if let Some(wfs) = hint.get("workflows").or_else(|| state.get("workflows")) {
            body["workflows"] = wfs.clone();
        }
        if body.get("operations").is_none() && body.get("workflows").is_none() {
            bail!(
                "FlowKit response has no polling receipt; retained at {:?}",
                receipt
            );
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
        let next = c.post("/api/flow/check-status", &body).await?;
        state = next;
        save_json(
            &receipt,
            &json!({"request":identity,"submitted_request":submitted_request,"response":state}),
        )
        .await?;
    }
}

/// Explicit receipt import downloads the selected media itself, retaining external identity.
pub async fn import_image(receipt: &Path, output: &Path, index: usize) -> Result<()> {
    if output.extension().and_then(|v| v.to_str()) != Some("png") {
        bail!("Imported images use a .png output (validated, normalized bytes)");
    }
    let data: Value = serde_json::from_slice(&std::fs::read(receipt)?)?;
    let row = if let Some(rows) = data.as_array() {
        rows.get(index).context("Receipt index missing")?
    } else {
        &data
    };
    let complete = row["status"] == "COMPLETED"
        || row["horizontal_image_status"] == "COMPLETED"
        || row["media_id"].is_string() && row["reference_image_url"].is_string();
    if !complete {
        bail!("Import requires a completed FlowKit image receipt");
    }
    let id = row["media_id"]
        .as_str()
        .or_else(|| row["horizontal_image_media_id"].as_str())
        .context("Receipt lacks media UUID")?;
    validate_uuid(id)?;
    let url = row["output_url"]
        .as_str()
        .or_else(|| row["horizontal_image_url"].as_str())
        .or_else(|| row["reference_image_url"].as_str())
        .context("Receipt lacks image URL")?;
    let bytes = Client::new()?.download(url).await?;
    let image = image::load_from_memory(&bytes)?;
    std::fs::create_dir_all(parent(output))?;
    let stage = tempfile::Builder::new()
        .suffix(".png")
        .tempfile_in(parent(output))?;
    image.save_with_format(stage.path(), image::ImageFormat::Png)?;
    stage.persist(output).map_err(|e| e.error)?;
    crate::artifacts::record_derived(
        output,
        crate::artifacts::Mode::Production,
        "flowkit-receipt-import",
        &[receipt.to_path_buf()],
        json!({"media_id":id,"project_id":row["project_id"],"receipt_digest":crate::artifacts::content_digest(receipt)?,"source_url":url,"watermark":"unverified","claim":"explicit external receipt import"}),
    )?;
    println!(
        "Imported FlowKit image {} as validated PNG {:?}",
        id, output
    );
    Ok(())
}
