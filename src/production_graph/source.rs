use super::model::*;
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub fn anchored(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    }
}
pub fn read_run(path: &Path) -> Result<Run> {
    let text = std::fs::read_to_string(path)?;
    let mut run: Run = if text.trim_start().starts_with('{') {
        serde_json::from_str(&text)?
    } else {
        xml_run(&text)?
    };
    if run.format != "clone-any-video.run@1" {
        bail!("Expected clone-any-video.run@1");
    }
    let base = path.parent().unwrap_or(Path::new("."));
    run.author = anchored(base, &run.author);
    if let Some(p) = &mut run.reuse {
        *p = anchored(base, p);
    }
    for candidate in run.candidates.values_mut() {
        match candidate {
            Candidate::File { path, .. } => *path = anchored(base, path),
            Candidate::Result { result, .. } => *result = anchored(base, result),
        }
    }
    Ok(run)
}
fn xml_run(text: &str) -> Result<Run> {
    let doc = roxmltree::Document::parse(text).context("Invalid Run XML")?;
    let root = doc.root_element();
    if root.tag_name().name() != "svrun" || root.attribute("version") != Some("1") {
        bail!("Expected <svrun version=\"1\"> with explicit author and targets");
    }
    attrs(root, &["version"])?;
    let mut author = None;
    let mut targets = Vec::new();
    let mut candidates = BTreeMap::new();
    let mut satisfy = BTreeMap::new();
    let mut reuse = None;
    for child in root.children() {
        if child.is_text() && !child.text().unwrap_or("").trim().is_empty() {
            bail!("Unexpected Run text");
        }
        if !child.is_element() {
            continue;
        }
        if child
            .children()
            .any(|n| n.is_element() || n.is_text() && !n.text().unwrap_or("").trim().is_empty())
        {
            bail!("Run declarations must be empty elements");
        }
        let get = |key: &str| {
            child
                .attribute(key)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .with_context(|| format!("{} requires {}", child.tag_name().name(), key))
        };
        match child.tag_name().name() {
            "author" => {
                attrs(child, &["source"])?;
                if author.replace(PathBuf::from(get("source")?)).is_some() {
                    bail!("Duplicate author");
                }
            }
            "target" => {
                attrs(child, &["output"])?;
                targets.push(get("output")?);
            }
            "file" => {
                attrs(child, &["id", "path", "type"])?;
                let ty: Kind = serde_json::from_value(Value::String(get("type")?))?;
                let candidate = Candidate::File { path: PathBuf::from(get("path")?), ty };
                if candidates.insert(get("id")?, candidate).is_some() {
                    bail!("Duplicate candidate");
                }
            }
            "result" => {
                attrs(child, &["id", "source", "output"])?;
                let candidate = Candidate::Result {
                    result: PathBuf::from(get("source")?), output: get("output")?,
                };
                if candidates.insert(get("id")?, candidate).is_some() {
                    bail!("Duplicate candidate");
                }
            }
            "satisfy" => {
                attrs(child, &["output", "candidate"])?;
                if satisfy.insert(get("output")?, get("candidate")?).is_some() {
                    bail!("Duplicate satisfaction");
                }
            }
            "reuse" => {
                attrs(child, &["result"])?;
                if reuse.replace(PathBuf::from(get("result")?)).is_some() {
                    bail!("Duplicate reuse result");
                }
            }
            name => bail!("Unsupported Run declaration <{}>; unsupported Run declaration syntax", name),
        }
    }
    Ok(Run {
        format: "clone-any-video.run@1".into(),
        author: author.context("Run needs an author")?,
        targets,
        candidates,
        satisfy,
        reuse,
    })
}
fn attrs(node: roxmltree::Node<'_, '_>, allowed: &[&str]) -> Result<()> {
    if node.tag_name().namespace().is_some() {
        bail!("Run namespaces are unsupported");
    }
    for attr in node.attributes() {
        if attr.namespace().is_some() || !allowed.contains(&attr.name()) {
            bail!(
                "Unsupported {} attribute {}",
                node.tag_name().name(),
                attr.name()
            );
        }
    }
    Ok(())
}
pub fn graph_run(path: &Path) -> Result<bool> {
    let text = std::fs::read_to_string(path)?;
    if path.extension().and_then(|s| s.to_str()) != Some("svrun")
        && !text.trim_start().starts_with('{')
    {
        return Ok(false);
    }
    let run = read_run(path)?;
    let body = std::fs::read_to_string(&run.author)?;
    if body.trim_start().starts_with('{') {
        return Ok(true);
    }
    if run.targets != ["final.video"]
        || !run.candidates.is_empty()
        || !run.satisfy.is_empty()
        || run.reuse.is_some()
    {
        bail!("Legacy SVML build only supports target final.video; use a production Author graph for candidates, satisfactions and reuse");
    }
    Ok(false)
}
pub fn result(path: &Path) -> Result<BuildResult> {
    let doc: BuildResult = serde_json::from_slice(&std::fs::read(path)?)?;
    if doc.format != "clone-any-video.result@1" {
        bail!("Unsupported Result format");
    }
    Ok(doc)
}
