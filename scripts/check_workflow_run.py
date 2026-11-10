"""Record native CLI evidence and audit a real run against the skill's mandatory stages.

This is an execution audit, not a second media production pipeline. Only the
existing ./scripts/clone-any-video command may be executed by `record`.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time

REPO = Path(__file__).resolve().parents[1]
STAGES = ("reference", "blueprint", "materials", "finishing", "review", "retrospective")
REQUIRED_FILES = {
    "reference": {"source_video", "probe", "transcript", "boundaries", "overview", "analysis", "timeline"},
    "blueprint": {"script", "timeline", "project", "summary", "treatment"},
    "materials": {"result", "video", "audio", "transcript"},
    "finishing": {"plan", "result", "video", "author", "run"},
    "review": {"report", "visual_review"},
    "retrospective": {"report"},
}


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read(path):
    return json.loads(Path(path).read_text())


def require(ok, message):
    if not ok:
        raise ValueError(message)


def evidence(item):
    path = Path(item["path"])
    require(path.is_absolute() and path.is_file(), f"Missing absolute evidence file: {path}")
    require(digest(path) == item["sha256"], f"Evidence bytes changed: {path}")
    return path


def receipt(item, command):
    data = read(evidence(item))
    require(data["exit_code"] == 0, f"Command failed: {data['argv']}")
    require(Path(data["argv"][0]).resolve() == REPO / "scripts/clone-any-video", "Use the native CLI")
    require(data["argv"][1] == command, f"Expected native {command} command")
    evidence(data["stdout"])
    evidence(data["stderr"])
    require(data["finished_ns"] >= data["started_ns"], "Invalid command timestamps")
    return data


def audit(manifest):
    require(manifest["format"] == "clone-any-video.workflow-run@1", "Unknown workflow audit format")
    evidence(manifest["skill_baseline"])
    stages = manifest["stages"]
    for name in STAGES:
        require(name in stages, f"Missing mandatory stage: {name}")
        stage = stages[name]
        if stage["mode"] == "blocked":
            raise ValueError(f"Stage blocked: {name}: {stage.get('reason', 'reason missing')}")
        require(stage["mode"] in ("executed", "reused"), f"Stage cannot be silently skipped: {name}")
        require(stage.get("reason", "").strip(), f"Missing execution/reuse reason: {name}")
        require(stage.get("files"), f"Missing stage evidence: {name}")
        require(REQUIRED_FILES[name] <= stage["files"].keys(), f"Missing required evidence files: {name}")
        for item in stage["files"].values():
            evidence(item)
    require(stages["blueprint"]["mode"] == "executed", "Compile the current blueprint; a copied layout is insufficient")
    b = stages["blueprint"]
    estimate_cmd = receipt(b["commands"]["estimate"], "script")
    timeline_cmd = receipt(b["commands"]["timeline"], "dynamic-timeline")
    project_cmd = receipt(b["commands"]["project"], "project")
    require("--estimate" in estimate_cmd["argv"], "Script estimate step was not run")
    for cmd in (estimate_cmd, timeline_cmd, project_cmd):
        source = Path(cmd["argv"][2])
        if not source.is_absolute():
            source = Path(cmd["cwd"]) / source
        require(source.resolve() == evidence(b["files"]["script"]).resolve(), "Blueprint commands must use the current target Script")
    require("--validate" in project_cmd["argv"] and "--json" in project_cmd["argv"], "Blueprint must be generated and validated by project --validate --json")
    for cmd, key in ((timeline_cmd, "timeline"), (project_cmd, "project")):
        argv = cmd["argv"]
        require("--to" in argv, "Native blueprint command must explicitly write its output")
        dest = Path(argv[argv.index("--to") + 1])
        if not dest.is_absolute():
            dest = Path(cmd["cwd"]) / dest
        require(dest.resolve() == evidence(b["files"][key]).resolve(), "Command output differs from audited blueprint")
        produced = cmd.get("outputs", {}).get("--to")
        require(produced is not None and evidence(produced).resolve() == dest.resolve(), "Missing captured native --to output")
        if key == "project":
            require(read(cmd["stdout"]["path"]) == read(dest), "Blueprint is not the native command's JSON output")
    project = read(b["files"]["project"]["path"])
    require(project["format"] == "clone-any-video.project@1" and project["shots"], "Invalid project blueprint")
    material_stage = stages["materials"]
    if material_stage["mode"] == "executed":
        material_plan = receipt(material_stage["commands"]["plan"], "plan")
        material_build = receipt(material_stage["commands"]["build"], "build")
        require(project_cmd["finished_ns"] <= material_plan["started_ns"] <= material_build["started_ns"], "New material production started before the blueprint")
        require(f"Production Result: {material_stage['files']['result']['path']}" in Path(material_build["stdout"]["path"]).read_text(), "Materials Result is not from its recorded Build")
    if manifest["performance_mode"] == "speaking-video":
        require("--speaking-video" in project_cmd["argv"], "Speaking performance needs the speaking-video blueprint")
        require(not any(t["task_type"] == "TtsSpeech" for t in project["asset_tasks"]), "Speaking-video cannot schedule independent replacement TTS")
    f = stages["finishing"]
    plan_cmd = receipt(f["commands"]["plan"], "plan")
    build_cmd = receipt(f["commands"]["build"], "build")
    require(project_cmd["finished_ns"] <= plan_cmd["started_ns"] <= build_cmd["started_ns"], "Blueprint must exist before production planning/build")
    plan = read(f["files"]["plan"]["path"])
    require(read(plan_cmd["stdout"]["path"]) == plan, "Plan differs from the recorded native command")
    require(not plan["blocked"], "Production plan is blocked")
    layout_paths = [Path(t["task"]["inputs"]["layout"]["path"]) for t in plan["tasks"] if t["task"]["action"]["producer"] == "project-timing"]
    author_base = Path(plan["author"]).parent
    require(any((p if p.is_absolute() else author_base / p).resolve() == Path(b["files"]["project"]["path"]).resolve() for p in layout_paths), "Production must consume the generated project.json, not a handwritten substitute")
    if stages["materials"]["mode"] == "reused":
        require(not any(t["decision"] == "execute" and t["task"]["action"]["producer"] in ("voice", "image", "video", "transcribe") for t in plan["tasks"]), "Reuse run unexpectedly executes new generation/ASR")
        run = read(plan["run"])
        require(run.get("satisfy") and run.get("candidates"), "Existing materials need explicit Run Candidate selection")
        if manifest["performance_mode"] == "speaking-video":
            kept = read(material_stage["files"]["result"]["path"])
            require(kept.get("format") == "clone-any-video.result@1" and kept["state"] == "succeeded", "Keep materials from a completed native Result")
            for output, kind in (("performance.video", "video"), ("performance.audio", "audio")):
                selected = run["candidates"][run["satisfy"][output]]
                require(selected["kind"] == "result", "Adopt paired media through Result Candidates")
                candidate_result = Path(selected["result"])
                if not candidate_result.is_absolute():
                    candidate_result = Path(plan["run"]).parent / candidate_result
                require(candidate_result.resolve() == Path(material_stage["files"]["result"]["path"]).resolve(), "Candidate comes from a different materials Result")
                artifact = kept["outputs"][selected["output"]]
                require(digest(artifact["path"]) == digest(material_stage["files"][kind]["path"]), "Selected material differs from recorded media")
            audio_digest = kept["outputs"][run["candidates"][run["satisfy"]["performance.audio"]]["output"]]["digest"]
            transcript = read(material_stage["files"]["transcript"]["path"])
            require(transcript.get("evidence", {}).get("source_media_digest") == audio_digest, "Transcript is not bound to the adopted sound")
    result_path = evidence(f["files"]["result"])
    require(f"Production Result: {result_path}" in Path(build_cmd["stdout"]["path"]).read_text(), "Result is not from the recorded Build")
    result = read(result_path)
    require(result["state"] == "succeeded", "Build did not succeed")
    final = evidence(f["files"]["video"])
    require(digest(final) == digest(result["outputs"]["final.video"]["path"]), "Final video differs from accepted Result")
    report = read(stages["review"]["files"]["report"]["path"])
    require(report["passed"], "Technical review failed")
    require(report["artifact_digest"] == result["outputs"]["final.video"]["digest"], "Review belongs to a different video")
    require(any(c["name"] == "Caption Delivery Evidence" and c["status"] == "Pass" for c in report["criteria"]), "Missing caption delivery evidence")
    return {"format": "clone-any-video.workflow-audit@1", "passed": True,
            "run_kind": manifest["run_kind"], "delivery_ready": report["ready_for_delivery"],
            "note": "This audit covers the recorded run only; corrective runs do not retroactively validate the original execution."}


def record(args):
    argv = args.command[1:] if args.command[:1] == ["--"] else args.command
    require(argv and Path(argv[0]).resolve() == REPO / "scripts/clone-any-video", "record accepts only ./scripts/clone-any-video")
    argv[0] = str(REPO / "scripts/clone-any-video")
    directory = Path(args.workspace).resolve() / "workflow/commands"
    directory.mkdir(parents=True, exist_ok=True)
    require(args.id and all(c.isalnum() or c in "-_" for c in args.id), "Invalid receipt id")
    target = directory / f"{args.id}.json"
    require(not target.exists(), "Preserve existing command receipts; choose a new id")
    started = time.time_ns()
    with target.with_suffix(".stdout.log").open("wb") as out, target.with_suffix(".stderr.log").open("wb") as err:
        result = subprocess.run(argv, stdout=out, stderr=err)
    def fact(path):
        return {"path": str(path), "sha256": digest(path)}
    outputs = {}
    for flag in ("--to", "--output"):
        if flag in argv:
            path = Path(argv[argv.index(flag) + 1]).resolve()
            if path.is_file():
                outputs[flag] = fact(path)
    target.write_text(json.dumps({"argv": argv, "cwd": str(Path.cwd()), "started_ns": started,
        "finished_ns": time.time_ns(), "exit_code": result.returncode,
        "outputs": outputs, "stdout": fact(target.with_suffix(".stdout.log")), "stderr": fact(target.with_suffix(".stderr.log"))}, indent=2) + "\n")
    print(f"Recorded exit={result.returncode}: {target}")
    return result.returncode


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    p = sub.add_parser("record")
    p.add_argument("--workspace", required=True)
    p.add_argument("--id", required=True)
    p.add_argument("command", nargs=argparse.REMAINDER)
    p = sub.add_parser("audit")
    p.add_argument("manifest", type=Path)
    p.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.action == "record":
            return record(args)
        report = audit(read(args.manifest))
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
        print(f"PASS: workflow audit; delivery_ready={report['delivery_ready']}")
        return 0
    except (ValueError, KeyError, OSError, IndexError) as error:
        if args.action == "audit":
            args.output.write_text(json.dumps({"passed": False, "error": str(error)}, ensure_ascii=False, indent=2) + "\n")
        print(f"FAIL: {error}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
