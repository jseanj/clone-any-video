# Clone Any Video (`clone-any-video`)

> An open-source, high-performance, deterministic multi-modal video production engine and autonomous video agent written in pure Rust.

[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-2021%20edition-orange.svg)](https://www.rust-lang.org)

`clone-any-video` provides a complete, deterministic, industrial-grade media pipeline for automated video creation, reference video reverse engineering, and quality review. It replaces fragile glue scripts with strongly typed media contracts, elastic timeline solvers, headless browser recording, and a production task DAG with incremental artifact reuse.

---

## 🌟 Key Capabilities

1. **Media Ingest & Reverse Engineering**
   - `probe`: Deep audio/video metadata extraction (resolution, CFR framerate, color matrix, audio clocks).
   - `cut`: Frame-accurate non-destructive media clipping.
   - `transcribe`: ASR transcription with word-level alignment evidence.
   - `boundaries`: Adjacent-frame visual discontinuity detection (L1 mean absolute difference).
   - `tile` / `tiles`: Contact sheet visual overview grid generation with timecode stamps.

2. **Semantic Script & Elastic Timeline**
   - `script`: SVML (Semantic Video Markup Language) AST parsing with dual-text `<Display | Spoken>` support and speech pace models.
   - `dynamic-timeline`: DAG-based topological timeline solver that decouples semantic narrative from physical seconds, allowing actual TTS audio durations to elastically adjust all downstream shot boundaries.
   - `project`: Declarative project blueprint schema (`clone-any-video.project@1`) tracking shots, layers, and pending asset generation tasks.

3. **Multi-Modal Craft Directors**
   - `image`: Four-paragraph prompt synthesis, quality auditing, and image generation.
   - `video`: Camera motion intention, pan/zoom vectors, and image-to-video generation.
   - `voice`: Multi-voice speech synthesis, speech extent extraction, and secondary timeline realignment.
   - `mix`: 48 kHz broadcast audio mixing with automated speech ducking envelopes and SFX anchoring.
   - `caption`: Word-level highlight scheduling, dynamic head tracking, and Web-as-Video subtitle export.
   - `component`: Web-as-Video HTML/CSS visual component templates and frame-rate rendering.
   - `capture`: Deterministic Chrome CDP frame capture and BT.709 video encoding.
   - `compose`: Single-pass FFmpeg complex filtergraph compositing for broadcast-grade master delivery.

4. **Production Graph & Review Agent**
   - `plan`: Compiles production runs into dependency DAGs with fine-grained request fingerprints (`request_key`).
   - `build`: Executes production DAGs with incremental task reuse and artifact caching.
   - `result`: Structured inspection of executed tasks, inputs, digests, and outputs.
   - `review`: Multi-dimensional automated quality review (conformance, duration match, loudness, BT.709 color standards, and delivery evidence).

---

## 🚀 Quick Start

### Prerequisites
- [Rust](https://www.rust-lang.org/) (1.78+)
- [FFmpeg](https://ffmpeg.org/) (with `libx264`)
- Google Chrome (optional, required for headless browser component capture)

### Build
```bash
cargo build --release
```

### Run Tests
```bash
cargo test
```

---

## 📖 CLI Usage

```bash
# Display all commands
cargo run -- --help

# 1. Compile a production plan (DAG dependency resolution)
cargo run -- plan examples/book-case/run.json --mock

# 2. Build the production plan into a master video
cargo run -- build examples/book-case/run.json --mock --output output/master.mp4

# 3. Inspect production results
cargo run -- result examples/book-case/.production/results/<run-id>/result.json

# 4. Run automated technical review
cargo run -- review \
  --project examples/book-case/.production/results/<run-id>/compose/project.json \
  --video output/master.mp4 \
  --mode mock --json
```

---

## 📂 Project Layout

```text
├── Cargo.toml                  # Standalone package definition
├── src/                        # Complete media pipeline & agent engine
│   ├── main.rs                 # CLI entrypoint & subcommand dispatcher
│   ├── production_graph/       # Task graph DAG, execution, planning & source parser
│   ├── video_agent.rs          # Video agent review & self-healing feedback loop
│   ├── compositor.rs           # Single-pass FFmpeg filtergraph compositor
│   ├── dynamic_timeline.rs     # DAG topological timeline solver
│   ├── sound_mix.rs            # Audio ducking envelope & multi-track mixing
│   ├── caption_engine.rs       # Word-level highlight & tracking captions
│   ├── browser_capture.rs      # Chrome CDP deterministic frame capture
│   └── ...                     # Probe, cut, transcribe, boundaries, image & voice
├── .agents/skills/             # Standard Agent Skills (Claude, Codex, Cursor, Antigravity)
│   ├── reference-video/        # Stage 1: Reference analysis & evidence extraction
│   ├── video-transformation/   # Stage 2: Semantic script rewriting & blueprint solving
│   ├── audiovisual-craft/      # Stage 3: Voice, visual, mix, and composition execution
│   └── clone-any-video/        # Full orchestrator skill & directing playbooks
├── skills/                     # Skills directory mirror & playbooks
├── docs/                       # Architectural specifications & pipeline contracts
├── fixtures/                   # Test media assets & mock fixtures
├── examples/                   # Reproducible production run examples
└── scripts/                    # CLI runner, alignment & transcription daemons
    ├── clone-any-video         # Unified executable wrapper for agent execution
    ├── check_workflow_run.py   # Workflow provenance & audit validator
    └── ...
```

---

## 🤖 Agent Skills & Autonomy

`clone-any-video` is built from the ground up for autonomous coding and directing agents. It provides standard skills conforming to the Agent Skills specification:

| Skill | Role & Responsibility | Primary Output |
| :--- | :--- | :--- |
| **`reference-video`** | Probes reference video physics, transcripts, boundaries, and visual tiles. | `ANALYSIS.md`, `TIMELINE.md` |
| **`video-transformation`** | Reconstructs narrative intent into SVML script and solves elastic DAG timeline. | `target_script.svml`, `project.json` |
| **`audiovisual-craft`** | Directs speech synthesis, visual generation, audio ducking mix, and compositing. | `master.mp4`, `review.json` |
| **`clone-any-video`** | Master orchestrator coordinating all phases with provenance and audit feedback. | Complete production lifecycle |

Any Agent with bash tool access can read `.agents/skills/<skill-name>/SKILL.md` and operate the engine via `./scripts/clone-any-video`.

---

## 🙏 Acknowledgements

`clone-any-video` is inspired by and adapted from the pioneering concepts of the **Hypit** project. 

We express our sincere gratitude to the Hypit authors for establishing the foundational paradigms of declarative video authoring, semantic script markup (SVML), multi-modal directing playbooks, and decoupled production task graphs. This project rebuilds and extends these ideas into a standalone, deterministic, high-performance media pipeline in pure Rust with full autonomous Agent Skill support.

---

## 📄 License

This project is licensed under the MIT License OR Apache-2.0.
