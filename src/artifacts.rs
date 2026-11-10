//! Local content-bound provenance and speech-dependent media admission.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Production,
    Mock,
    Unknown,
}
impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Mock => "mock",
            Self::Unknown => "unknown",
        }
    }
}
#[derive(Serialize, Deserialize)]
struct Provenance {
    format: String,
    digest: String,
    mode: Mode,
    provider: String,
    #[serde(default)]
    details: serde_json::Value,
    #[serde(default)]
    inputs: Vec<(PathBuf, String)>,
}
pub fn digest(bytes: &[u8]) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    format!("{:016x}", h.finish())
}
fn file_digest(p: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(p).with_context(|| format!("Cannot read media {:?}", p))?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.write(&buffer[..n]);
    }
    Ok(format!("{:016x}", hash.finish()))
}
fn sidecar(p: &Path) -> PathBuf {
    p.with_file_name(format!(
        "{}.provenance.json",
        p.file_name().unwrap_or_default().to_string_lossy()
    ))
}
fn write(p: &Path, value: &impl Serialize) -> Result<()> {
    let dir = p
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let temp = tempfile::NamedTempFile::new_in(dir)?;
    std::fs::write(temp.path(), serde_json::to_vec_pretty(value)?)?;
    temp.persist(p).map_err(|e| e.error)?;
    Ok(())
}
pub fn record(p: &Path, mode: Mode, provider: &str) -> Result<()> {
    write(
        &sidecar(p),
        &Provenance {
            format: "clone-any-video.artifact@1".into(),
            digest: file_digest(p)?,
            mode,
            provider: provider.into(),
            details: serde_json::Value::Null,
            inputs: Vec::new(),
        },
    )
}
pub fn mode(p: &Path) -> Mode {
    let read = || -> Result<Mode> {
        let r: Provenance = serde_json::from_slice(&std::fs::read(sidecar(p))?)?;
        if r.format != "clone-any-video.artifact@1" || r.digest != file_digest(p)? {
            return Ok(Mode::Unknown);
        }
        Ok(r.mode)
    };
    read().unwrap_or(Mode::Unknown)
}
pub fn combined<'a>(inputs: impl IntoIterator<Item = &'a Path>) -> Mode {
    let modes: Vec<Mode> = inputs.into_iter().map(mode).collect();
    if modes.contains(&Mode::Mock) {
        Mode::Mock
    } else if modes.contains(&Mode::Unknown) {
        Mode::Unknown
    } else {
        Mode::Production
    }
}
pub fn admit_supplied(p: &Path) -> Result<()> {
    if mode(p) == Mode::Mock {
        bail!(
            "Known Mock media cannot be relabeled production; supply the actual production asset"
        );
    }
    record(p, Mode::Production, "user-supplied")
}
#[derive(Serialize, Deserialize)]
struct MediaState {
    speech: String,
    segment_speech: BTreeMap<String, String>,
    voices: BTreeMap<String, String>,
    master: Option<String>,
}
fn speech(script: &crate::script::ScriptDocument) -> String {
    let values: Vec<_> = script
        .segments
        .iter()
        .map(|s| {
            (
                &s.id,
                s.turns
                    .iter()
                    .map(|t| (&t.role, &t.spoken_text))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    digest(&serde_json::to_vec(&values).expect("serializable speech"))
}
fn state(
    dir: &Path,
    script: &crate::script::ScriptDocument,
    master: Option<&Path>,
) -> Result<MediaState> {
    let mut voices = BTreeMap::new();
    let voice_dir = dir.join("assets/voice");
    if voice_dir.exists() {
        for entry in std::fs::read_dir(voice_dir)? {
            let p = entry?.path();
            if matches!(
                p.extension().and_then(|x| x.to_str()),
                Some("wav" | "mp3" | "m4a")
            ) {
                voices.insert(
                    p.file_name().unwrap().to_string_lossy().into(),
                    file_digest(&p)?,
                );
            }
        }
    }
    let mix = master
        .map(Path::to_path_buf)
        .unwrap_or_else(|| dir.join("assets/audio/master_mix.wav"));
    let segment_speech = script
        .segments
        .iter()
        .map(|seg| {
            (
                seg.id.clone(),
                digest(
                    &serde_json::to_vec(
                        &seg.turns
                            .iter()
                            .map(|t| (&t.role, &t.spoken_text))
                            .collect::<Vec<_>>(),
                    )
                    .expect("speech"),
                ),
            )
        })
        .collect();
    Ok(MediaState {
        speech: speech(script),
        segment_speech,
        voices,
        master: if mix.exists() {
            Some(file_digest(&mix)?)
        } else {
            None
        },
    })
}
pub fn check_media(dir: &Path, script: &crate::script::ScriptDocument) -> Result<()> {
    check_media_with_master(dir, script, None)
}
pub fn check_media_with_master(
    dir: &Path,
    script: &crate::script::ScriptDocument,
    master: Option<&Path>,
) -> Result<()> {
    // Validate generator sidecars too, including a workspace's first build.
    for seg in &script.segments {
        for name in [format!("voice_{}.json", seg.id), format!("{}.json", seg.id)] {
            let p = dir.join("assets/voice").join(name);
            if p.exists() {
                let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p)?)?;
                if let Some(text) = v["spoken_text"].as_str() {
                    if crate::speech_alignment::spoken_key(text)
                        != crate::speech_alignment::spoken_key(&seg.spoken_text)
                    {
                        bail!("Stale voice for segment '{}'; regenerate voice and master mix before build",seg.id);
                    }
                }
            }
        }
    }
    let ledger = dir.join(".media-dependencies.json");
    if ledger.exists() {
        let old: MediaState = serde_json::from_slice(&std::fs::read(ledger)?)?;
        let now = state(dir, script, master)?;
        for (id, text) in &now.segment_speech {
            if old
                .segment_speech
                .get(id)
                .is_some_and(|prior| prior != text)
            {
                for (name, hash) in &now.voices {
                    let stem = Path::new(name)
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy();
                    if (stem == *id || stem == format!("voice_{}", id))
                        && old.voices.get(name) == Some(hash)
                    {
                        bail!("Stale voice for segment '{}'; spoken content changed but {} is unchanged",id,name);
                    }
                }
            }
        }
        if (old.speech != now.speech || old.voices != now.voices)
            && now.master.is_some()
            && old.master == now.master
        {
            bail!("Stale master mix: speech or voice inputs changed but the selected master audio is unchanged. Regenerate affected voice and mix; existing files were preserved");
        }
    }
    Ok(())
}
pub fn accept_media(dir: &Path, script: &crate::script::ScriptDocument) -> Result<()> {
    accept_media_with_master(dir, script, None)
}

pub fn accept_media_with_master(
    dir: &Path,
    script: &crate::script::ScriptDocument,
    master: Option<&Path>,
) -> Result<()> {
    write(
        &dir.join(".media-dependencies.json"),
        &state(dir, script, master)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changed_content_does_not_keep_production_label() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.wav");
        std::fs::write(&p, b"a").unwrap();
        record(&p, Mode::Mock, "test").unwrap();
        assert_eq!(mode(&p), Mode::Mock);
        assert!(admit_supplied(&p).is_err());
        std::fs::write(&p, b"b").unwrap();
        assert_eq!(mode(&p), Mode::Unknown);
    }
    #[test]
    fn speech_changes_invalidate_mix_but_display_edits_reuse_it() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("assets/audio")).unwrap();
        let p = d.path().join("assets/audio/master_mix.wav");
        std::fs::write(&p, b"old").unwrap();
        let parse = |text: &str| crate::script::parse_script(text).unwrap();
        let a = parse("<script id=\"x\"><hook><HOST> <NYC | New York></hook></script>");
        accept_media(d.path(), &a).unwrap();
        let b = parse("<script id=\"x\"><hook><HOST> <纽约 | New York></hook></script>");
        check_media(d.path(), &b).unwrap();
        let c = parse("<script id=\"x\"><hook><HOST> Changed speech.</hook></script>");
        assert!(check_media(d.path(), &c).is_err());
        std::fs::write(p, b"new").unwrap();
        check_media(d.path(), &c).unwrap();
    }
    #[test]
    fn replacing_mix_cannot_bless_an_unchanged_voice_after_speech_edit() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("assets/voice")).unwrap();
        std::fs::create_dir_all(d.path().join("assets/audio")).unwrap();
        let voice = d.path().join("assets/voice/voice_hook.wav");
        let mix = d.path().join("assets/audio/master_mix.wav");
        std::fs::write(&voice, b"old voice").unwrap();
        std::fs::write(&mix, b"old mix").unwrap();
        let a = crate::script::parse_script("<script><hook><HOST> Old.</hook></script>").unwrap();
        accept_media(d.path(), &a).unwrap();
        let b = crate::script::parse_script("<script><hook><HOST> New.</hook></script>").unwrap();
        std::fs::write(&mix, b"new mix").unwrap();
        assert!(check_media(d.path(), &b)
            .unwrap_err()
            .to_string()
            .contains("Stale voice"));
        std::fs::write(&voice, b"new voice").unwrap();
        check_media(d.path(), &b).unwrap();
        accept_media(d.path(), &b).unwrap();
        std::fs::write(voice, b"another take").unwrap();
        assert!(check_media(d.path(), &b)
            .unwrap_err()
            .to_string()
            .contains("Stale master mix"));
    }
}

/// Record actual producer parameters and content-bound dependencies, without discarding lineage.
pub fn record_derived(
    path: &Path,
    mode: Mode,
    provider: &str,
    inputs: &[PathBuf],
    details: serde_json::Value,
) -> Result<()> {
    let inputs = inputs
        .iter()
        .map(|p| Ok((p.canonicalize()?, file_digest(p)?)))
        .collect::<Result<Vec<_>>>()?;
    write(
        &sidecar(path),
        &Provenance {
            format: "clone-any-video.artifact@1".into(),
            digest: file_digest(path)?,
            mode,
            provider: provider.into(),
            inputs,
            details,
        },
    )
}
pub fn details(path: &Path) -> Option<serde_json::Value> {
    if mode(path) == Mode::Unknown {
        return None;
    }
    let p: Provenance = serde_json::from_slice(&std::fs::read(sidecar(path)).ok()?).ok()?;
    Some(p.details)
}
pub fn content_digest(path: &Path) -> Result<String> {
    file_digest(path)
}
