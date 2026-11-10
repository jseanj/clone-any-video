use super::{model::*, *};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[tokio::test]
async fn adopted_performance_keeps_cut_source_time_and_rejects_unpaired_sound() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // Deliberately synthetic audiovisual fixture: color changes expose a source-time restart.
    let status = tokio::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=red:s=64x64:r=30:d=0.5",
            "-f",
            "lavfi",
            "-i",
            "color=blue:s=64x64:r=30:d=0.5",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=1",
            "-filter_complex",
            "[0:v][1:v]concat=n=2:v=1:a=0[v]",
            "-map",
            "[v]",
            "-map",
            "2:a",
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
        ])
        .arg(root.join("source.mp4"))
        .status()
        .await
        .unwrap();
    assert!(status.success());
    crate::artifacts::record(
        &root.join("source.mp4"),
        crate::artifacts::Mode::Mock,
        "synthetic performance fixture",
    )
    .unwrap();
    let script = "<script id=\"pair\"><hook><HOST>Hello. || world.</hook></script>";
    std::fs::write(root.join("script.svml"), script).unwrap();
    let doc = crate::script::parse_script(script).unwrap();
    let timeline = crate::timeline::project_script_to_timeline(
        &doc,
        30.0,
        0.0,
        Some(&std::collections::HashMap::from([("hook".into(), 1.0)])),
    )
    .unwrap();
    let mut layout =
        crate::project::create_project_manifest(&doc, &timeline, None, 64, 64).unwrap();
    let mut second = layout.shots[0].clone();
    layout.shots[0].end_frame = 15;
    layout.shots[0].end_seconds = 0.5;
    layout.shots[0].duration_seconds = 0.5;
    second.id = "shot_02".into();
    second.shot_index = 1;
    second.start_frame = 15;
    second.start_seconds = 0.5;
    second.duration_seconds = 0.5;
    layout.shots.push(second);
    layout.tracks.visual_bg.clear();
    assert_eq!(layout.shots.len(), 2);
    write(
        &root.join("layout.json"),
        &serde_json::to_value(layout).unwrap(),
    );
    write(
        &root.join("transcript.json"),
        &json!({"format":"clone-any-video.transcript@1","source":"synthetic timing fixture, not real speech","language":"en","audio_seconds":1,"passages":[{"text":"Hello. world.","start_seconds":0,"end_seconds":1,"words":[{"text":"Hello.","start_seconds":0.05,"end_seconds":0.4},{"text":"world.","start_seconds":0.5,"end_seconds":0.9}]}]}),
    );
    let mut author = json!({"format":"clone-any-video.author@1","tasks":[
        {"id":"pair","action":{"producer":"adopt-performance","fps":30},"inputs":{"video":{"kind":"file","path":"source.mp4","type":"video"}}},
        {"id":"align","action":{"producer":"align-speech","fps":30},"inputs":{"script":{"kind":"script","path":"script.svml","view":"timing","segment":"hook"},"audio":{"kind":"output","output":"pair.audio"},"transcript":{"kind":"file","path":"transcript.json","type":"json"}}},
        {"id":"clock","action":{"producer":"project-timing","fps":30,"gap":0},"inputs":{"script":{"kind":"script","path":"script.svml","view":"timing"},"layout":{"kind":"file","path":"layout.json","type":"project"},"alignment:hook":{"kind":"output","output":"align.alignment"}}},
        {"id":"mix","action":{"producer":"mix"},"inputs":{"script":{"kind":"script","path":"script.svml","view":"speech"},"project":{"kind":"output","output":"clock.project"},"voice:hook":{"kind":"output","output":"pair.audio"}}},
        {"id":"compose","action":{"producer":"compose"},"inputs":{"project":{"kind":"output","output":"clock.project"},"audio":{"kind":"output","output":"mix.audio"},"background:shot_01":{"kind":"output","output":"pair.video"},"background:shot_02":{"kind":"output","output":"pair.video"}}}
    ],"outputs":{"final.video":{"task":"compose","port":"video"}}});
    write(&root.join("author.json"), &author);
    write(
        &root.join("run.json"),
        &json!({"format":"clone-any-video.run@1","author":"author.json","targets":["final.video"]}),
    );
    let result = build(&root.join("run.json"), None, None, true)
        .await
        .unwrap();
    let accepted = source::result(&result).unwrap();
    let project: crate::project::ProjectManifest = serde_json::from_value(
        plan::canonical_json(&accepted.outputs["compose.project"].path).unwrap(),
    )
    .unwrap();
    let second = project
        .tracks
        .visual_bg
        .iter()
        .find(|v| v.id == "bg_shot_02")
        .unwrap();
    assert_eq!(second.payload["sampling"]["from_frame"], 15);
    let pixels = tokio::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&accepted.outputs["final.video"].path)
        .args([
            "-vf",
            "select=eq(n\\,20)",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .output()
        .await
        .unwrap();
    assert!(pixels.status.success());
    assert!(
        pixels.stdout[2] > 200 && pixels.stdout[0] < 30,
        "second cut must continue at blue source frames"
    );
    assert_eq!(
        crate::probe::probe_media(&accepted.outputs["final.video"].path)
            .await
            .unwrap()
            .frame_count,
        Some(30)
    );
    // A picture-only rate change must fail before composition.
    let mut changed = serde_json::to_value(&project).unwrap();
    changed["tracks"]["visual_bg"][0]["payload"]["sampling"]["rate_numerator"] = json!(2);
    write(&root.join("retimed.json"), &changed);
    author["tasks"][4]["inputs"]["project"] =
        json!({"kind":"file","path":"retimed.json","type":"project"});
    write(&root.join("author.json"), &author);
    let error = build(&root.join("run.json"), None, None, true)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("Retiming visible performance"),
        "{error:#}"
    );
    // A valid audio file from another take carries no proof of this performance.
    author["tasks"][4]["inputs"]["project"] = json!({"kind":"output","output":"clock.project"});
    author["tasks"][4]["inputs"]["audio"] =
        json!({"kind":"file","path":"source.mp4","type":"audio"});
    // Use an actual unrelated WAV, rather than relying on type rejection.
    let status = tokio::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:duration=1",
        ])
        .arg(root.join("other.wav"))
        .status()
        .await
        .unwrap();
    assert!(status.success());
    crate::artifacts::record(
        &root.join("other.wav"),
        crate::artifacts::Mode::Mock,
        "unpaired fixture",
    )
    .unwrap();
    author["tasks"][4]["inputs"]["audio"]["path"] = json!("other.wav");
    write(&root.join("author.json"), &author);
    let error = build(&root.join("run.json"), None, None, true)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("retain its own adopted audio"),
        "{error:#}"
    );
}
fn write(path: &Path, value: &Value) {
    std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

#[tokio::test]
async fn screenshot_selection_keeps_lineage_and_invalidates_downstream_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let source = root.join("source.mp4");
    assert!(tokio::process::Command::new("ffmpeg").args([
        "-v","error","-f","lavfi","-i","color=red:s=32x32:r=24:d=0.5",
        "-f","lavfi","-i","color=blue:s=32x32:r=24:d=0.5",
        "-filter_complex","[0:v][1:v]concat=n=2:v=1:a=0","-c:v","libx264"
    ]).arg(&source).status().await.unwrap().success());
    crate::artifacts::record(&source,crate::artifacts::Mode::Mock,"synthetic fixture").unwrap();
    let mut author = json!({"format":"clone-any-video.author@1","tasks":[
        {"id":"frame","action":{"producer":"extract-frame","at_seconds":0.75},"inputs":{"video":{"kind":"file","path":"source.mp4","type":"video"}}}
    ],"outputs":{}});
    write(&root.join("author.json"),&author);
    let run = root.join("run.json");
    write(&run,&json!({"format":"clone-any-video.run@1","author":"author.json","targets":["frame.image"]}));
    let result = build(&run,None,None,true).await.unwrap();
    let accepted = source::result(&result).unwrap();
    let frame = &accepted.outputs["frame.image"].path;
    let pixel = image::open(frame).unwrap().to_rgb8().get_pixel(16,16).0;
    assert!(pixel[2] > 200 && pixel[0] < 30);
    assert_eq!(crate::artifacts::mode(frame),crate::artifacts::Mode::Mock);
    assert_eq!(crate::artifacts::details(frame).unwrap()["audio_reference"],false);
    reuse(&run,&result);
    assert_eq!(compile(&run,true).unwrap().tasks[0].decision,Decision::Reuse);
    author["tasks"][0]["action"]["at_seconds"]=json!(0.0);
    write(&root.join("author.json"),&author);
    assert_eq!(compile(&run,true).unwrap().tasks[0].decision,Decision::Execute);
    author["tasks"][0]["action"]["at_seconds"]=json!(1.1);
    write(&root.join("author.json"),&author);
    assert!(build(&run,None,None,true).await.is_err());
    author["tasks"][0]["action"]["at_seconds"]=json!(-1.0);
    write(&root.join("author.json"),&author);
    assert!(compile(&run,true).is_err());
}

#[tokio::test]
async fn source_video_edit_is_a_typed_dependency_and_mock_is_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let source = root.join("source.mp4");
    assert!(tokio::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=red:s=32x32:r=24:d=1",
            "-c:v",
            "libx264"
        ])
        .arg(&source)
        .status()
        .await
        .unwrap()
        .success());
    crate::artifacts::admit_supplied(&source).unwrap();
    let mut author = json!({"format":"clone-any-video.author@1","tasks":[{
        "id":"edit","action":{"producer":"video","flow_project_id":"11111111-2222-3333-4444-555555555555",
        "source_edit":{"start_frame":0,"end_frame":30},
        "spec":{"shot_id":"source","first_frame_path":null,"motion_prompt":"Replace only the product.","camera_motion":"StaticTripod","relationship":"SourceVideoEdit","duration_seconds":1,"fps":24,"width":360,"height":640}},
        "inputs":{"source_video":{"kind":"file","path":"source.mp4","type":"video"}}
    }],"outputs":{}});
    write(&root.join("author.json"), &author);
    write(
        &root.join("run.json"),
        &json!({"format":"clone-any-video.run@1","author":"author.json","targets":["edit.video"]}),
    );
    let plan = compile(&root.join("run.json"), false).unwrap();
    let mock = compile(&root.join("run.json"), true).unwrap();
    assert!(mock.blocked.iter().any(|v| v.contains("real FlowKit")));
    std::fs::write(
        root.join("dialogue.svml"),
        "<script id=\"edit\"><presentation><HOST>New words.</presentation></script>",
    )
    .unwrap();
    author["tasks"][0]["inputs"]["dialogue"] =
        json!({"kind":"script","path":"dialogue.svml","view":"speech","segment":"presentation"});
    write(&root.join("author.json"), &author);
    let speech = compile(&root.join("run.json"), false).unwrap();
    assert!(speech.blocked.iter().any(|v| v.contains("Flow policy")));
    author["tasks"][0]["inputs"]
        .as_object_mut()
        .unwrap()
        .remove("dialogue");
    author["tasks"][0]["action"]["source_edit"]["end_frame"] = json!(31);
    write(&root.join("author.json"), &author);
    let changed = compile(&root.join("run.json"), false).unwrap();
    assert_ne!(plan.tasks[0].request_key, changed.tasks[0].request_key);
    author["tasks"][0]["action"]["source_edit"]["end_frame"] = json!(60);
    write(&root.join("author.json"), &author);
    assert!(compile(&root.join("run.json"), false).is_err());
    author["tasks"][0]["action"]["source_edit"]["end_frame"] = json!(30);
    author["tasks"][0]["inputs"]["source_video"]["type"] = json!("image");
    write(&root.join("author.json"), &author);
    assert!(compile(&root.join("run.json"), false).is_err());
}
fn fixture(dir: &Path) -> PathBuf {
    std::fs::write(
        dir.join("script.svml"),
        "<script id=\"brand\"><hook><HOST><Original | Same spoken line>.</hook></script>",
    )
    .unwrap();
    image::RgbImage::from_pixel(16, 16, image::Rgb([100, 30, 40]))
        .save(dir.join("product.png"))
        .unwrap();
    write(&dir.join("layout.json"), &json!({"color":"white"}));
    let author = json!({"format":"clone-any-video.author@1","tasks":[
      {"id":"final","action":{"producer":"json","value":{"captions":{"$input":"captions"},"mix":{"$input":"mix"},"video":{"$input":"video"}}},"inputs":{"captions":{"kind":"output","output":"captions.value"},"mix":{"kind":"output","output":"mix.value"},"video":{"kind":"output","output":"video.value"}}},
      {"id":"voice","action":{"producer":"voice","voice":"Mia","language":"en"},"inputs":{"script":{"kind":"script","path":"script.svml","view":"speech","segment":"hook"}}},
      {"id":"captions","action":{"producer":"json","value":{"display":{"$input":"script"},"layout":{"$input":"layout"}}},"inputs":{"script":{"kind":"script","path":"script.svml"},"layout":{"kind":"file","path":"layout.json","type":"json"}}},
      {"id":"mix","action":{"producer":"json","value":{"voice":{"$input":"voice"}}},"inputs":{"voice":{"kind":"output","output":"voice.audio"}}},
      {"id":"image","action":{"producer":"json","value":{"reference":{"$input":"product"}}},"inputs":{"product":{"kind":"file","path":"product.png","type":"image"}}},
      {"id":"video","action":{"producer":"json","value":{"image":{"$input":"image"}}},"inputs":{"image":{"kind":"output","output":"image.value"}}}
    ],"outputs":{"final.video":{"task":"final","port":"value"}}});
    write(&dir.join("author.json"), &author);
    let run =
        json!({"format":"clone-any-video.run@1","author":"author.json","targets":["final.video"]});
    let path = dir.join("run.svrun");
    write(&path, &run);
    path
}
fn decisions(plan: &Plan) -> std::collections::BTreeMap<String, Decision> {
    plan.tasks
        .iter()
        .map(|t| (t.task.id.clone(), t.decision.clone()))
        .collect()
}
fn reuse(run: &Path, result: &Path) {
    let mut doc: Value = serde_json::from_slice(&std::fs::read(run).unwrap()).unwrap();
    doc["reuse"] = json!(result);
    write(run, &doc);
}

#[tokio::test]
async fn incremental_changes_preserve_unaffected_production_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let run = fixture(dir.path());
    let initial = compile(&run, true).unwrap();
    assert!(initial
        .tasks
        .iter()
        .all(|t| t.decision == Decision::Execute));
    assert!(!dir.path().join(".production").exists());
    let result_path = build(&run, None, None, true).await.unwrap();
    let original = source::result(&result_path).unwrap();
    reuse(&run, &result_path);
    assert!(compile(&run, true)
        .unwrap()
        .tasks
        .iter()
        .all(|t| t.decision == Decision::Reuse));
    // Display changes affect captions and presentation, while the actual voice bytes remain selected.
    std::fs::write(
        dir.path().join("script.svml"),
        "<script id=\"brand\"><hook><HOST><New brand display | Same spoken line>.</hook></script>",
    )
    .unwrap();
    let plan = compile(&run, true).unwrap();
    let d = decisions(&plan);
    assert_eq!(d["voice"], Decision::Reuse);
    assert_eq!(d["mix"], Decision::Reuse);
    assert_eq!(d["image"], Decision::Reuse);
    assert_eq!(d["captions"], Decision::Execute);
    assert_eq!(d["final"], Decision::Execute);
    let second = build(&run, None, None, true).await.unwrap();
    let revised = source::result(&second).unwrap();
    assert_eq!(
        original.outputs["voice.audio"].path,
        revised.outputs["voice.audio"].path
    );
    assert_ne!(
        original.outputs["captions.value"].digest,
        revised.outputs["captions.value"].digest
    );
    // Spoken changes invalidate voice and its dependent mix, not product imagery.
    std::fs::write(dir.path().join("script.svml"),"<script id=\"brand\"><hook><HOST><Original | A much longer new spoken sentence>.</hook></script>").unwrap();
    let d = decisions(&compile(&run, true).unwrap());
    assert_eq!(d["voice"], Decision::Execute);
    assert_eq!(d["mix"], Decision::Execute);
    assert_eq!(d["image"], Decision::Reuse);
    std::fs::write(
        dir.path().join("script.svml"),
        "<script id=\"brand\"><hook><HOST><Original | Same spoken line>.</hook></script>",
    )
    .unwrap();
    image::RgbImage::from_pixel(16, 16, image::Rgb([0, 200, 10]))
        .save(dir.path().join("product.png"))
        .unwrap();
    let d = decisions(&compile(&run, true).unwrap());
    assert_eq!(d["image"], Decision::Execute);
    assert_eq!(d["video"], Decision::Execute);
    assert_eq!(d["voice"], Decision::Reuse);
    assert_eq!(d["captions"], Decision::Reuse);
    image::RgbImage::from_pixel(16, 16, image::Rgb([100, 30, 40]))
        .save(dir.path().join("product.png"))
        .unwrap();
    write(&dir.path().join("layout.json"), &json!({"color":"black"}));
    let d = decisions(&compile(&run, true).unwrap());
    assert_eq!(d["captions"], Decision::Execute);
    assert_eq!(d["voice"], Decision::Reuse);
    assert_eq!(d["video"], Decision::Reuse);
}
#[test]
fn complete_graph_types_targets_cycles_and_xml_are_checked() {
    let dir = tempfile::tempdir().unwrap();
    let run = fixture(dir.path());
    let path = dir.path().join("author.json");
    let original: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut a = original.clone();
    a["tasks"][1]["inputs"]["script"] = json!({"kind":"output","output":"final.video"});
    write(&path, &a);
    assert!(compile(&run, true)
        .unwrap_err()
        .to_string()
        .contains("must be Script"));
    let mut a = original.clone();
    a["tasks"][4]["inputs"]["product"] = json!({"kind":"output","output":"final.video"});
    write(&path, &a);
    assert!(compile(&run, true)
        .unwrap_err()
        .to_string()
        .contains("cycle"));
    write(&path, &original);
    write(
        &run,
        &json!({"format":"clone-any-video.run@1","author":"author.json","targets":["unknown"]}),
    );
    assert!(compile(&run, true).is_err());
    let xml = dir.path().join("xml.svrun");
    std::fs::write(&xml,"<svrun version=\"1\"><author source=\"author.json\"/><target output=\"final.video\"/><mystery/></svrun>").unwrap();
    assert!(compile(&xml, true)
        .unwrap_err()
        .to_string()
        .contains("Unsupported Run"));
}
#[tokio::test]
async fn explicit_candidates_prune_generation_and_check_types() {
    let dir = tempfile::tempdir().unwrap();
    let run = fixture(dir.path());
    write(&dir.path().join("provided.json"), &json!({"accepted":true}));
    std::fs::write(&run,"<svrun version=\"1\"><author source=\"author.json\"/><file id=\"chosen\" path=\"provided.json\" type=\"json\"/><satisfy output=\"final.video\" candidate=\"chosen\"/><target output=\"final.video\"/></svrun>").unwrap();
    let p = compile(&run, true).unwrap();
    assert!(p.tasks.is_empty());
    let result = build(&run, None, None, true).await.unwrap();
    assert_eq!(
        source::result(&result).unwrap().outputs["final.video"].path,
        dir.path().join("provided.json").canonicalize().unwrap()
    );
    std::fs::write(&run,"<svrun version=\"1\"><author source=\"author.json\"/><file id=\"chosen\" path=\"product.png\" type=\"image\"/><satisfy output=\"final.video\" candidate=\"chosen\"/><target output=\"final.video\"/></svrun>").unwrap();
    assert!(compile(&run, true)
        .unwrap_err()
        .to_string()
        .contains("wrong type"));
}
#[tokio::test]
async fn changed_accepted_bytes_do_not_reuse_a_cached_task() {
    let dir = tempfile::tempdir().unwrap();
    let run = fixture(dir.path());
    let result = build(&run, None, None, true).await.unwrap();
    let r = source::result(&result).unwrap();
    reuse(&run, &result);
    std::fs::write(&r.outputs["image.value"].path, b"corrupted").unwrap();
    let d = decisions(&compile(&run, true).unwrap());
    assert_eq!(d["image"], Decision::Execute);
    assert_eq!(d["voice"], Decision::Reuse);
}
#[test]
fn script_projection_keeps_role_identity_but_ignores_display() {
    let dir = tempfile::tempdir().unwrap();
    let run = fixture(dir.path());
    let p = compile(&run, true).unwrap();
    let key = p
        .tasks
        .iter()
        .find(|t| t.task.id == "voice")
        .unwrap()
        .request_key
        .clone();
    std::fs::write(
        dir.path().join("script.svml"),
        "<script id=\"brand\"><hook><GUEST><Original | Same spoken line>.</hook></script>",
    )
    .unwrap();
    let p = compile(&run, true).unwrap();
    assert_ne!(
        key,
        p.tasks
            .iter()
            .find(|t| t.task.id == "voice")
            .unwrap()
            .request_key
    );
    assert!(p.runtime.keys().all(|name| !name.contains("API_KEY")));
}
#[tokio::test]
async fn partial_failure_keeps_completed_results_for_next_attempt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("author.json");
    write(
        &path,
        &json!({"format":"clone-any-video.author@1","tasks":[{"id":"good","action":{"producer":"json","value":{"accepted":1}}},{"id":"bad","action":{"producer":"json","value":{"$input":"undeclared"}},"inputs":{"earlier":{"kind":"output","output":"good.value"}}}],"outputs":{"final":{"task":"bad","port":"value"}}}),
    );
    let run = dir.path().join("run.svrun");
    write(
        &run,
        &json!({"format":"clone-any-video.run@1","author":"author.json","targets":["final"]}),
    );
    assert!(build(&run, None, None, true).await.is_err());
    let entry = std::fs::read_dir(dir.path().join(".production/results"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("result.json");
    let result = source::result(&entry).unwrap();
    assert_eq!(result.state, State::Failed);
    assert_eq!(result.tasks["good"].state, State::Succeeded);
    assert_eq!(result.tasks["bad"].state, State::Failed);
    assert!(result.outputs.contains_key("good.value"));
    reuse(&run, &entry);
    assert_eq!(
        decisions(&compile(&run, true).unwrap())["good"],
        Decision::Reuse
    );
}

#[tokio::test]
async fn native_media_graph_keeps_frame_clock_and_caption_delivery_proof() {
    let dir = tempfile::tempdir().unwrap();
    let script = "<script id=\"sample\"><hook><HOST>Hello world.</hook></script>";
    std::fs::write(dir.path().join("script.svml"), script).unwrap();
    let document = crate::script::parse_script(script).unwrap();
    let timeline = crate::timeline::project_script_to_timeline(&document, 30.0, 0.0, None).unwrap();
    let project =
        crate::project::create_project_manifest(&document, &timeline, None, 64, 64).unwrap();
    write(
        &dir.path().join("layout.json"),
        &serde_json::to_value(project).unwrap(),
    );
    image::RgbImage::from_pixel(64, 64, image::Rgb([30, 60, 90]))
        .save(dir.path().join("picture.png"))
        .unwrap();
    crate::artifacts::record(
        &dir.path().join("picture.png"),
        crate::artifacts::Mode::Mock,
        "fixture",
    )
    .unwrap();
    write(
        &dir.path().join("transcript.json"),
        &json!({"format":"clone-any-video.transcript@1","source":"explicit mock fixture","language":"en","audio_seconds":1,"passages":[{"text":"Hello world.","start_seconds":0,"end_seconds":0.2,"words":[{"text":"Hello","start_seconds":0.02,"end_seconds":0.09},{"text":"world.","start_seconds":0.11,"end_seconds":0.2}]}]}),
    );
    let author = json!({"format":"clone-any-video.author@1","tasks":[
      {"id":"voice","action":{"producer":"voice","voice":"Mia","language":"en"},"inputs":{"script":{"kind":"script","path":"script.svml","view":"speech","segment":"hook"}}},
      {"id":"align","action":{"producer":"realign","fps":30,"gap":0},"inputs":{"script":{"kind":"script","path":"script.svml","view":"speech"},"layout":{"kind":"file","path":"layout.json","type":"project","view":"project-timing"},"voice:hook":{"kind":"output","output":"voice.audio"}}},
      {"id":"mix","action":{"producer":"mix"},"inputs":{"script":{"kind":"script","path":"script.svml","view":"speech"},"project":{"kind":"output","output":"align.project"},"voice:hook":{"kind":"output","output":"voice.audio"}}},
      {"id":"schedule","action":{"producer":"caption-schedule"},"inputs":{"script":{"kind":"script","path":"script.svml"},"transcript":{"kind":"file","path":"transcript.json","type":"json"}}},
      {"id":"captions","action":{"producer":"render-captions"},"inputs":{"schedule":{"kind":"output","output":"schedule.document"},"project":{"kind":"output","output":"align.project"}}},
      {"id":"compose","action":{"producer":"compose"},"inputs":{"project":{"kind":"output","output":"captions.project"},"audio":{"kind":"output","output":"mix.audio"},"background:shot_01":{"kind":"file","path":"picture.png","type":"image"},"overlay:caption_overlay":{"kind":"output","output":"captions.video"}}},
      {"id":"review","action":{"producer":"review"},"inputs":{"project":{"kind":"output","output":"compose.project"},"video":{"kind":"output","output":"compose.video"}}}
    ],"outputs":{"final.video":{"task":"compose","port":"video"},"final.review":{"task":"review","port":"report"}}});
    write(&dir.path().join("author.json"), &author);
    let run = dir.path().join("run.svrun");
    write(
        &run,
        &json!({"format":"clone-any-video.run@1","author":"author.json","targets":["final.video","final.review"]}),
    );
    let result = build(&run, None, Some(&dir.path().join("export.mp4")), true)
        .await
        .unwrap();
    let result = source::result(&result).unwrap();
    let project: crate::project::ProjectManifest = serde_json::from_value(
        plan::canonical_json(&result.outputs["compose.project"].path).unwrap(),
    )
    .unwrap();
    let media = crate::probe::probe_media(&result.outputs["final.video"].path)
        .await
        .unwrap();
    assert_eq!(media.frame_count, Some(project.meta.total_frames));
    assert_eq!(
        result.outputs["final.video"].mode,
        crate::artifacts::Mode::Mock
    );
    let review = plan::canonical_json(&result.outputs["final.review"].path).unwrap();
    assert_eq!(review["ready_for_delivery"], false);
    assert!(review["criteria"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["name"] == "Caption Delivery Evidence" && c["status"] == "Pass"));
    let exported = crate::artifacts::details(&dir.path().join("export.mp4")).unwrap();
    assert!(exported["caption_windows"].as_array().unwrap().len() > 0);
}

#[test]
fn unresolved_generation_requires_receipt_inspection_and_explicit_revision() {
    let dir = tempfile::tempdir().unwrap();
    let run = fixture(dir.path());
    write(
        &run,
        &json!({"format":"clone-any-video.run@1","author":"author.json","targets":["voice.audio"]}),
    );
    let planned = compile(&run, false).unwrap();
    let task = &planned.tasks[0];
    let result_path = dir.path().join("interrupted.json");
    write(
        &result_path,
        &json!({"format":"clone-any-video.result@1","run":run,"source_digest":"fixture","mock":false,"state":"running","targets":["voice.audio"],"outputs":{},"error":null,"tasks":{"voice":{"request_key":task.request_key,"state":"running","inputs":{},"outputs":{},"directory":dir.path(),"error":null}}}),
    );
    reuse(&run, &result_path);
    assert!(compile(&run, false)
        .unwrap()
        .blocked
        .iter()
        .any(|r| r.contains("receipt")));
    let path = dir.path().join("author.json");
    let mut author = plan::canonical_json(&path).unwrap();
    author["tasks"][1]["revision"] = json!("explicit-new-attempt-after-inspection");
    write(&path, &author);
    assert!(!compile(&run, false)
        .unwrap()
        .blocked
        .iter()
        .any(|r| r.contains("receipt")));
    assert!(!dir.path().join(".production").exists());
}

#[test]
fn declared_media_types_are_verified_from_bytes() {
    let dir = tempfile::tempdir().unwrap();
    image::RgbImage::from_pixel(16, 16, image::Rgb([0, 0, 0]))
        .save(dir.path().join("image.png"))
        .unwrap();
    assert!(plan::file(&dir.path().join("image.png"), Kind::Audio).is_err());
}

#[tokio::test]
async fn missing_provenance_invalidates_previously_accepted_media() {
    let dir = tempfile::tempdir().unwrap();
    let run = fixture(dir.path());
    let result = build(&run, None, None, true).await.unwrap();
    let accepted = source::result(&result).unwrap();
    reuse(&run, &result);
    let voice = &accepted.outputs["voice.audio"].path;
    std::fs::remove_file(voice.with_file_name("audio.wav.provenance.json")).unwrap();
    assert_eq!(
        decisions(&compile(&run, true).unwrap())["voice"],
        Decision::Execute
    );
}

#[tokio::test]
async fn shared_clock_drives_native_mix_captions_and_component_source_time() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let source="<script id=\"shared\"><hook><HOST>Hello. || @{reveal!} @{proof}world.@{/proof}</hook></script>";
    std::fs::write(root.join("script.svml"), source).unwrap();
    let doc = crate::script::parse_script(source).unwrap();
    let timeline = crate::timeline::project_script_to_timeline(
        &doc,
        30.0,
        0.0,
        Some(&std::collections::HashMap::from([("hook".into(), 4.0)])),
    )
    .unwrap();
    let layout = crate::project::create_project_manifest(&doc, &timeline, None, 64, 64).unwrap();
    write(
        &root.join("layout.json"),
        &serde_json::to_value(layout).unwrap(),
    );
    for (name, source) in [
        ("voice.wav", "anullsrc=r=48000:cl=stereo:d=4"),
        (
            "effect.wav",
            "sine=frequency=1000:sample_rate=48000:duration=0.5",
        ),
    ] {
        let status = tokio::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", source])
            .arg(root.join(name))
            .status()
            .await
            .unwrap();
        assert!(status.success());
        crate::artifacts::record(
            &root.join(name),
            crate::artifacts::Mode::Mock,
            "explicit test fixture",
        )
        .unwrap();
    }
    image::RgbImage::from_pixel(64, 64, image::Rgb([0, 0, 0]))
        .save(root.join("picture.png"))
        .unwrap();
    std::fs::write(root.join("graphic.html"),"<html><head></head><body><script>window.renderFrame=(frame)=>{document.body.style.background=frame>=PRODUCTION_CLOCK.timeline.instants[0].at_frame?'lime':'black'};</script></body></html>").unwrap();
    write(
        &root.join("transcript.json"),
        &json!({"format":"clone-any-video.transcript@1","source":"explicit test timing, not observed speech","language":"en","audio_seconds":4,"passages":[{"text":"Hello world.","start_seconds":0,"end_seconds":4,"words":[{"text":"Hello.","start_seconds":0.1,"end_seconds":0.4},{"text":"world.","start_seconds":3.1234567891234567,"end_seconds":3.5}]}]}),
    );
    let author = json!({"format":"clone-any-video.author@1","tasks":[
      {"id":"local","action":{"producer":"align-speech","fps":30},"inputs":{"script":{"kind":"script","path":"script.svml","view":"timing","segment":"hook"},"audio":{"kind":"file","path":"voice.wav","type":"audio"},"transcript":{"kind":"file","path":"transcript.json","type":"json"}}},
      {"id":"clock","action":{"producer":"project-timing","fps":30,"gap":0},"inputs":{"script":{"kind":"script","path":"script.svml","view":"timing"},"layout":{"kind":"file","path":"layout.json","type":"project"},"alignment:hook":{"kind":"output","output":"local.alignment"}}},
      {"id":"mix","action":{"producer":"mix"},"inputs":{"script":{"kind":"script","path":"script.svml","view":"speech"},"project":{"kind":"output","output":"clock.project"},"voice:hook":{"kind":"file","path":"voice.wav","type":"audio"},"sfx:sfx_reveal":{"kind":"file","path":"effect.wav","type":"audio"}}},
      {"id":"schedule","action":{"producer":"caption-schedule"},"inputs":{"script":{"kind":"script","path":"script.svml"},"clock":{"kind":"output","output":"clock.clock"}}},
      {"id":"captions","action":{"producer":"render-captions"},"inputs":{"project":{"kind":"output","output":"clock.project"},"schedule":{"kind":"output","output":"schedule.document"}}},
      {"id":"graphic","action":{"producer":"render-html","frames":120,"config":{"width":64,"height":64,"fps":30,"crf":23,"preset":"veryfast","transparent":true,"timeout_seconds":30}},"inputs":{"html":{"kind":"file","path":"graphic.html","type":"html"},"clock":{"kind":"output","output":"clock.clock"}}},
      {"id":"compose","action":{"producer":"compose"},"inputs":{"project":{"kind":"output","output":"captions.project"},"audio":{"kind":"output","output":"mix.audio"},"background:shot_01":{"kind":"file","path":"picture.png","type":"image"},"overlay:caption_overlay":{"kind":"output","output":"captions.video"},"overlay:comp_hook_proof":{"kind":"output","output":"graphic.video"}}}
    ],"outputs":{"final.video":{"task":"compose","port":"video"}}});
    write(&root.join("author.json"), &author);
    write(
        &root.join("run.json"),
        &json!({"format":"clone-any-video.run@1","author":"author.json","targets":["final.video"]}),
    );
    let path = build(&root.join("run.json"), None, None, true)
        .await
        .unwrap();
    let r = source::result(&path).unwrap();
    let clock: crate::semantic_clock::ProgramClock =
        serde_json::from_value(plan::canonical_json(&r.outputs["clock.clock"].path).unwrap())
            .unwrap();
    assert_eq!(clock.point("moment:reveal").unwrap(), 94);
    let project: crate::project::ProjectManifest =
        serde_json::from_value(plan::canonical_json(&r.outputs["compose.project"].path).unwrap())
            .unwrap();
    let card = project
        .tracks
        .visual_components
        .iter()
        .find(|c| c.id == "comp_hook_proof")
        .unwrap();
    assert_eq!(card.start_frame, 94);
    assert_eq!(card.payload["sampling"]["from_frame"], 94);
    let html = std::fs::read_to_string(r.tasks["graphic"].directory.join("program.html")).unwrap();
    assert!(html.contains("window.PRODUCTION_CLOCK="));
    assert!(html.contains("\"at_frame\":94"));
    let out = tokio::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&r.outputs["mix.audio"].path)
        .args(["-f", "f32le", "-ac", "1", "-ar", "48000", "pipe:1"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let samples: Vec<f32> = out
        .stdout
        .chunks_exact(4)
        .map(|x| f32::from_le_bytes(x.try_into().unwrap()))
        .collect();
    assert!(samples[..(3.12 * 48000.0) as usize]
        .iter()
        .all(|s| s.abs() < 0.0001));
    assert!(samples[(3.2 * 48000.0) as usize..(3.3 * 48000.0) as usize]
        .iter()
        .any(|s| s.abs() > 0.01));
    assert_eq!(
        crate::probe::probe_media(&r.outputs["final.video"].path)
            .await
            .unwrap()
            .frame_count,
        Some(120)
    );
}

#[test]
fn speaking_video_uses_spoken_identity_and_refuses_mock_performance() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("script.svml"),
        "<script id=\"speaker\"><hook><HOST><Original | Hello world.></hook></script>",
    )
    .unwrap();
    write(
        &root.join("author.json"),
        &json!({"format":"clone-any-video.author@1","tasks":[{
        "id":"speaker","action":{"producer":"video","flow_project_id":"00000000-0000-4000-8000-000000000001","spec":{"shot_id":"shot_01","first_frame_path":null,"motion_prompt":"Visible natural speaking performance","camera_motion":"StaticTripod","relationship":"TextToVideo","duration_seconds":4,"fps":30,"width":640,"height":360}},
        "inputs":{"dialogue":{"kind":"script","path":"script.svml","view":"speech","segment":"hook"}}
    }],"outputs":{}}),
    );
    write(
        &root.join("run.json"),
        &json!({"format":"clone-any-video.run@1","author":"author.json","targets":["speaker.video"]}),
    );
    let first = compile(&root.join("run.json"), false).unwrap();
    std::fs::write(
        root.join("script.svml"),
        "<script id=\"speaker\"><hook><HOST><New display | Hello world.></hook></script>",
    )
    .unwrap();
    let display = compile(&root.join("run.json"), false).unwrap();
    assert_eq!(first.tasks[0].request_key, display.tasks[0].request_key);
    std::fs::write(root.join("script.svml"),"<script id=\"speaker\"><hook><HOST><New display | Different spoken words.></hook></script>").unwrap();
    let spoken = compile(&root.join("run.json"), false).unwrap();
    assert_ne!(first.tasks[0].request_key, spoken.tasks[0].request_key);
    let mock = compile(&root.join("run.json"), true).unwrap();
    assert!(
        mock.blocked
            .iter()
            .any(|b| b.contains("Speaking") || b.contains("speaking")),
        "{:?}",
        mock.blocked
    );
}
