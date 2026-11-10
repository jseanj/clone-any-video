//! Explicit acoustic measurement; no ASR or interpolation fallback.
use crate::transcribe::{TranscriptDocument, TranscriptPassage, TranscriptWord};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::Path;
pub const DEFAULT_URL: &str = "http://127.0.0.1:8766";

pub async fn align(
    audio: &Path,
    transcript: &TranscriptDocument,
    url: &str,
    model: &str,
    min_score: f64,
    diagnostic: &Path,
) -> Result<TranscriptDocument> {
    if !min_score.is_finite() || !(0.0..=1.0).contains(&min_score) {
        bail!("min_word_score must be 0..1");
    }
    let digest = crate::artifacts::content_digest(audio)?;
    if transcript.evidence["source_media_digest"]
        .as_str()
        .is_some_and(|d| d != digest)
    {
        bail!("ASR/curated transcript belongs to different audio bytes");
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;
    let health: Value = client
        .get(format!("{}/health", url.trim_end_matches('/')))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if health["protocol"] != "clone-any-video.acoustic-alignment@1"
        || health["engine"] != "whisperx-alignment"
        || health["model"] != model
        || health["language"] != transcript.language
        || health["model_loaded"] != true
    {
        bail!("Alignment service identity/model/language does not match; no inference submitted");
    }
    let temp = tempfile::tempdir()?;
    let wav = temp.path().join("evidence.wav");
    let out = tokio::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(audio)
        .args([
            "-map",
            "0:a:0",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(&wav)
        .output()
        .await?;
    if !out.status.success() {
        bail!(
            "Cannot decode alignment audio: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let info = crate::probe::probe_media(&wav).await?;
    let segments = transcript
        .passages
        .iter()
        .map(|p| -> Result<Value> {
            let start = p
                .start_seconds
                .context("Alignment needs explicit passage start")?;
            let end = p
                .end_seconds
                .context("Alignment needs explicit passage end")?;
            if !start.is_finite()
                || !end.is_finite()
                || start < 0.0
                || end <= start
                || end > info.duration_seconds + 0.02
                || p.text.trim().is_empty()
            {
                bail!("Passage exceeds adopted local audio domain");
            }
            Ok(json!({"text":p.text,"start":start,"end":end.min(info.duration_seconds)}))
        })
        .collect::<Result<Vec<_>>>()?;
    if segments.is_empty() {
        bail!("Cannot force-align an empty transcript");
    }
    let response = client
        .post(format!("{}/align", url.trim_end_matches('/')))
        .json(&json!({"audio_path":wav,"language":transcript.language,"segments":segments}))
        .send()
        .await?;
    let status = response.status();
    let raw: Value = response.json().await?;
    if let Some(parent) = diagnostic.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(diagnostic, serde_json::to_vec_pretty(&raw)?)?;
    if !status.is_success() {
        bail!("Acoustic alignment failed; inspect {:?}", diagnostic);
    }
    if raw["forced_alignment"] != true
        || raw["interpolation"] != "none"
        || raw["engine"]["model"] != model
        || raw["engine"]["engine"] != "whisperx-alignment"
        || raw["language"] != transcript.language
    {
        bail!("Endpoint did not return the requested real acoustic evidence");
    }
    let mut passages = Vec::new();
    for segment in raw["segments"]
        .as_array()
        .context("Missing acoustic segments")?
    {
        let mut words = Vec::new();
        for word in segment["words"]
            .as_array()
            .context("Missing acoustic words")?
        {
            let text = word["word"]
                .as_str()
                .or_else(|| word["text"].as_str())
                .context("Missing acoustic word text")?;
            if !text.chars().any(|c| c.is_alphanumeric()) {
                continue;
            }
            let start = word["start"]
                .as_f64()
                .context("Untimed acoustic word; coverage requires review")?;
            let end = word["end"]
                .as_f64()
                .context("Untimed acoustic word; coverage requires review")?;
            let score = word["score"]
                .as_f64()
                .context("Missing acoustic confidence")?;
            if !start.is_finite()
                || !end.is_finite()
                || start < 0.0
                || end <= start
                || end > info.duration_seconds + 0.001
                || !score.is_finite()
                || !(min_score..=1.0).contains(&score)
            {
                bail!(
                    "Invalid/low-confidence acoustic word '{}'; inspect {:?}",
                    text,
                    diagnostic
                );
            }
            words.push(TranscriptWord {
                text: text.into(),
                start_seconds: Some(start),
                end_seconds: Some(end),
                score: Some(score),
            });
        }
        if !words.is_empty() {
            passages.push(TranscriptPassage {
                text: segment["text"].as_str().unwrap_or_default().into(),
                start_seconds: segment["start"].as_f64(),
                end_seconds: segment["end"].as_f64(),
                words,
            });
        }
    }
    let mut doc = TranscriptDocument {
        format: crate::transcribe::TRANSCRIPT_FORMAT.into(),
        source: audio.canonicalize()?.display().to_string(),
        language: transcript.language.clone(),
        audio_seconds: info.duration_seconds,
        passages,
        evidence: json!({"forced_alignment":true,"interpolation":"none","source_media_digest":digest,"engine":raw["engine"],"coverage":raw["coverage"],"raw_response":diagnostic,"input_evidence":transcript.evidence}),
    };
    // Every adopted lexical unit must remain represented; word mismatches cannot be hidden by the aligner.
    let targets: Vec<_> = transcript
        .passages
        .iter()
        .flat_map(|p| p.words.iter())
        .map(|w| w.text.clone())
        .collect();
    if targets.is_empty() {
        bail!("Alignment input must retain transcript word identities");
    }
    let _ = crate::speech_alignment::align_units(&targets, &doc)?;
    if raw["coverage"]["missing_words"]
        .as_array()
        .is_some_and(|x| !x.is_empty())
    {
        bail!(
            "Acoustic alignment has missing words; inspect {:?}",
            diagnostic
        );
    }
    doc.evidence["weak_words"] = raw["coverage"]["weak_words"].clone();
    Ok(doc)
}
