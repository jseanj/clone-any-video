//! Typed production tasks, explicit Run selection, pure planning and accepted Result reuse.
mod actions;
mod execute;
pub(crate) mod model;
mod plan;
pub mod source;
#[cfg(test)]
mod tests;
use anyhow::{bail, Context, Result};
pub use model::Plan;
use std::path::{Path, PathBuf};

pub fn compile(path: &Path, mock: bool) -> Result<Plan> {
    plan::compile(path, mock)
}
pub fn inspect(path: &Path) -> Result<serde_json::Value> {
    Ok(serde_json::to_value(source::result(path)?)?)
}
pub async fn build(
    path: &Path,
    workspace: Option<&Path>,
    export: Option<&Path>,
    mock: bool,
) -> Result<PathBuf> {
    let plan = compile(path, mock)?;
    let export_target = if let Some(target) = export {
        if target.extension().and_then(|s| s.to_str()) != Some("mp4") {
            bail!("Video export uses an explicit .mp4 output");
        }
        let candidates: Vec<_> = plan
            .targets
            .iter()
            .filter(|name| {
                let o = &plan.outputs[*name];
                if let Some(a) = plan.provided.get(&format!("{}.{}", o.task, o.port)) {
                    a.ty == model::Kind::Video
                } else {
                    plan.tasks
                        .iter()
                        .find(|t| t.task.id == o.task)
                        .is_some_and(|t| t.task.action.outputs()[&o.port] == model::Kind::Video)
                }
            })
            .collect();
        if candidates.len() != 1 {
            bail!("--output requires exactly one video Target; choose Targets explicitly");
        }
        if target.exists() {
            let canonical = target.canonicalize()?;
            let mut inputs: Vec<_> = plan.provided.values().map(|a| &a.path).collect();
            for t in &plan.tasks {
                for i in t.inputs.values() {
                    if let model::PlannedInput::Material { material } = i {
                        inputs.push(&material.artifact.path);
                    }
                }
                if let Some(cached) = &t.cached {
                    inputs.extend(cached.outputs.values().map(|a| &a.path));
                }
            }
            if inputs.iter().any(|p| **p == canonical) {
                bail!("Export cannot overwrite accepted inputs/Results");
            }
        }
        Some(candidates[0].clone())
    } else {
        None
    };
    let root = workspace
        .map(Path::to_owned)
        .unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")).to_owned())
        .join(".production/results");
    let result_path = execute::run(plan, &root).await?;
    if let (Some(target), Some(name)) = (export, export_target) {
        let result = source::result(&result_path)?;
        let a = result.outputs.get(&name).context("Missing video Target")?;
        let parent = target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let stage = tempfile::NamedTempFile::new_in(parent)?;
        std::fs::copy(&a.path, stage.path())?;
        if crate::artifacts::content_digest(stage.path())? != a.digest {
            bail!("Result changed during export");
        }
        stage.persist(target).map_err(|e| e.error)?;
        crate::artifacts::record_derived(
            target,
            a.mode,
            "graph-export",
            &[a.path.clone()],
            crate::artifacts::details(&a.path).unwrap_or_default(),
        )?;
    }
    Ok(result_path)
}
