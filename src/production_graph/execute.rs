use super::{actions, model::*, plan};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub fn atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().context("Missing parent directory")?;
    std::fs::create_dir_all(parent)?;
    let temp = tempfile::NamedTempFile::new_in(parent)?;
    std::fs::write(temp.path(), serde_json::to_vec_pretty(value)?)?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
fn canonical(plan: &Plan, name: &str) -> Result<String> {
    let o = plan.outputs.get(name).context("Missing output binding")?;
    Ok(format!("{}.{}", o.task, o.port))
}
fn inputs(
    plan: &Plan,
    task: &PlannedTask,
    known: &BTreeMap<String, Artifact>,
) -> Result<BTreeMap<String, Material>> {
    task.inputs
        .iter()
        .map(|(name, input)| {
            let material = match input {
                PlannedInput::Material { material } => {
                    if let Some(v) = &material.value {
                        if plan::hash(v)? != material.artifact.digest {
                            bail!("Material snapshot mismatch");
                        }
                    } else {
                        plan::verify(&material.artifact)?;
                    }
                    material.clone()
                }
                PlannedInput::Output { output, ty } => {
                    let artifact = known
                        .get(&canonical(plan, output)?)
                        .context("Dependency has not completed")?
                        .clone();
                    if artifact.ty != *ty {
                        bail!("Dependency type changed");
                    }
                    plan::verify(&artifact)?;
                    Material {
                        artifact,
                        value: None,
                    }
                }
            };
            if !plan.mock
                && material.artifact.ty.media()
                && material.artifact.mode != crate::artifacts::Mode::Production
            {
                bail!("Runtime input {} is not production media", name);
            }
            Ok((name.clone(), material))
        })
        .collect()
}
fn stage(
    inputs: &BTreeMap<String, Material>,
    directory: &Path,
) -> Result<BTreeMap<String, Material>> {
    let dir = directory.join("inputs");
    std::fs::create_dir_all(&dir)?;
    let mut staged = BTreeMap::new();
    for (index, (name, material)) in inputs.iter().enumerate() {
        let path = dir.join(format!("{}.{}", index, material.artifact.ty.extension()));
        if let Some(v) = &material.value {
            std::fs::write(&path, serde_json::to_vec_pretty(v)?)?;
        } else {
            std::fs::copy(&material.artifact.path, &path)?;
            if crate::artifacts::content_digest(&path)? != material.artifact.digest {
                bail!("Input changed while staging {}", name);
            }
        }
        crate::artifacts::record_derived(
            &path,
            material.artifact.mode,
            "graph-input-snapshot",
            &[],
            material.artifact.details.clone(),
        )?;
        let mut m = material.clone();
        m.artifact.path = path;
        staged.insert(name.clone(), m);
    }
    Ok(staged)
}
fn cached(record: &TaskRecord, key: &str, outputs: &BTreeMap<String, Kind>) -> bool {
    record.request_key == key
        && matches!(record.state, State::Succeeded | State::Reused)
        && outputs.iter().all(|(p, k)| {
            record
                .outputs
                .get(p)
                .is_some_and(|a| a.ty == *k && plan::verify(a).is_ok())
        })
}

pub async fn run(plan: Plan, root: &Path) -> Result<PathBuf> {
    if !plan.blocked.is_empty() {
        bail!("Production plan blocked: {}", plan.blocked.join("; "));
    }
    // Configuration is captured for identity; never silently execute a different routing selection.
    if plan::runtime()? != plan.runtime {
        bail!("Runtime settings changed after planning; plan again");
    }
    std::fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let directory = root.join(format!("run-{}-{}", stamp, std::process::id()));
    std::fs::create_dir(&directory)?;
    let result_path = directory.join("result.json");
    atomic(&directory.join("plan.json"), &plan)?;
    let mut result = BuildResult {
        format: "clone-any-video.result@1".into(),
        run: plan.run.clone(),
        source_digest: plan.source_digest.clone(),
        mock: plan.mock,
        state: State::Running,
        targets: plan.targets.clone(),
        tasks: BTreeMap::new(),
        outputs: BTreeMap::new(),
        error: None,
    };
    let mut known = plan.provided.clone();
    atomic(&result_path, &result)?;
    for a in known.values() {
        if let Err(error) = plan::verify(a) {
            result.state = State::Failed;
            result.error = Some(error.to_string());
            atomic(&result_path, &result)?;
            bail!(
                "Provided artifact changed; Result {:?}: {}",
                result_path,
                error
            );
        }
    }

    publish(&plan, &known, &mut result)?;
    atomic(&result_path, &result)?;
    for task in &plan.tasks {
        eprintln!("[graph] {} {:?}", task.task.id, task.decision);
        let resolved = match inputs(&plan, task, &known) {
            Ok(v) => v,
            Err(error) => {
                result.state = State::Failed;
                result.error = Some(error.to_string());
                atomic(&result_path, &result)?;
                bail!(
                    "Input admission failed; Result retained at {:?}: {}",
                    result_path,
                    error
                );
            }
        };
        let input_digests = resolved
            .iter()
            .map(|(n, m)| Ok((n.clone(), plan::input_identity(&m.artifact)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        let key = plan::key(&task.task, &input_digests, &plan.runtime, plan.mock)?;
        let work = directory.join(&task.task.id);
        std::fs::create_dir(&work)?;
        let mut record = TaskRecord {
            request_key: key.clone(),
            state: State::Running,
            inputs: input_digests,
            outputs: BTreeMap::new(),
            directory: work.clone(),
            error: None,
        };
        result.tasks.insert(task.task.id.clone(), record.clone());
        atomic(&result_path, &result)?;
        let outcome: Result<()> = async {
            let ports = task.task.action.outputs();
            if let Some(old) = &task.cached {
                if !cached(old, &key, &ports) {
                    bail!("Explicitly planned reuse no longer valid; plan again before executing");
                }
                record.outputs = old.outputs.clone();
                record.state = State::Reused;
                return Ok(());
            }
            if let Some(old) = &task.previous {
                if cached(old, &key, &ports) {
                    record.outputs = old.outputs.clone();
                    record.state = State::Reused;
                    return Ok(());
                }
                if !plan.mock && task.task.action.external() && old.request_key == key
                    && matches!(old.state, State::Running | State::Failed) {
                    bail!("Previous generation outcome requires receipt inspection; no automatic resubmission");
                }
            }
            let staged = stage(&resolved, &work)?;
            let outputs = actions::execute(&task.task, &staged, &work, plan.mock).await?;
            for (port, kind) in ports {
                let path = outputs.get(&port).context("Producer omitted an output")?;
                let artifact = plan::file(path, kind)?;
                if !plan.mock && kind.media() && artifact.mode != crate::artifacts::Mode::Production {
                    bail!("Producer {} emitted non-production media", task.task.id);
                }
                record.outputs.insert(port, artifact);
            }
            record.state = State::Succeeded;
            Ok(())
        }.await;
        if let Err(error) = outcome {
            record.state = State::Failed;
            record.error = Some(error.to_string());
            result.tasks.insert(task.task.id.clone(), record);
            result.state = State::Failed;
            result.error = Some(format!("{}: {}", task.task.id, error));
            publish(&plan, &known, &mut result)?;
            atomic(&result_path, &result)?;
            bail!("Production failed; inspect {:?}: {}", result_path, error);
        }
        for (port, a) in &record.outputs {
            let name = format!("{}.{}", task.task.id, port);
            if !plan.provided.contains_key(&name) {
                known.insert(name, a.clone());
            }
        }
        result.tasks.insert(task.task.id.clone(), record);
        publish(&plan, &known, &mut result)?;
        atomic(&result_path, &result)?;
    }
    publish(&plan, &known, &mut result)?;
    for target in &plan.targets {
        if !result.outputs.contains_key(target) {
            bail!("Target {} did not resolve", target);
        }
    }
    result.state = State::Succeeded;
    atomic(&result_path, &result)?;
    eprintln!("[graph] Result {:?}", result_path);
    Ok(result_path)
}
fn publish(
    plan: &Plan,
    known: &BTreeMap<String, Artifact>,
    result: &mut BuildResult,
) -> Result<()> {
    for name in plan.outputs.keys() {
        if let Some(a) = known.get(&canonical(plan, name)?) {
            result.outputs.insert(name.clone(), a.clone());
        }
    }
    Ok(())
}
