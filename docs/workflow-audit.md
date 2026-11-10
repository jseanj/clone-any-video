# 真实运行的流程复盘与执行约束

每次正式生成、续跑、修复或采用已有媒体，都以运行开始时的 clone-any-video skill 为基线。复盘的目标是比较实际执行与流程，不能只列一遍成功命令。

## 运行前与运行后

1. 保存主 skill 快照到工作目录 `workflow/skill-baseline.md`。已有旧运行时保存原始记录；修复运行单独记录，不覆盖历史。
2. 阶段一的原片、probe、ASR、边界、视盘、ANALYSIS/TIMELINE 可以复用，但必须记录原路径、字节 SHA-256 和理由。不能用其他视频的同名文件代替。
3. 当前目标 Script 必须重新经过原生 `script --estimate → dynamic-timeline → project --validate --json`。输出 `target_script.svml`、`target_timeline.json`、`project.json` 和同参数的 `BLUEPRINT.txt`。使用实测前的时间预算，后续由 project-timing 对齐。
4. 新生成材料前先执行 Plan；已有材料用 Run Candidate + satisfy 显式选择。可见人物口播用 `project --speaking-video`，声音随视频生成和采用；`--no-bgm` 明确省略默认音乐。两者是原生 project 选项，不是手改 ProjectManifest。
5. 最终对齐必须读取原生 project.json。字幕排期、真实渲染、同源声音混音、合成与技术 review 均须完成。模型预览不能代替带字幕成片。
6. 写 WORKFLOW_RUN.json、WORKFLOW_REVIEW.md，运行 `scripts/check_workflow_run.py audit`。技术通过、流程检查通过、创意交付通过是不同结论。

## 原生命令回执

`record --workspace <dir> --id <unique-id> -- ./scripts/clone-any-video ...` 只包装原有 CLI。它保留 argv、cwd、开始/结束时间、退出码、stdout/stderr 文件和哈希，以及命令完成时 `--to` / `--output` 文件的哈希；不生成计划、不改素材、不重试请求。不覆盖旧 ID，失败日志同样保留。

`project` 的 JSON stdout 与 `--to` 文件必须一致；dynamic-timeline 带 `--to` 时仅打印保存提示，因此检查写出的 JSON 文件及其回执哈希。复制旧布局无法满足这项检查。记录器需要在实际命令执行时使用，不能事后编造成功回执。

## WORKFLOW_RUN.json 字段

根对象使用 `format: clone-any-video.workflow-run@1`，另包含 `run_kind`（如 corrective）、`performance_mode`（如 speaking-video）、`skill_baseline` 与 `stages`。每份文件事实均为：

```json
{"path":"/absolute/path/to/file","sha256":"actual SHA-256 of these bytes"}
```

每阶段声明 `mode: executed|reused`、非空 `reason` 与 `files` 文件事实映射，不允许用空对象或无理由的 skip 充数。

| stages 键 | 必要证据 |
| --- | --- |
| reference | 原片、probe、原片 ASR、边界、视盘、ANALYSIS.md、TIMELINE.md；复用来源与本次原片一致 |
| blueprint | files 中的 script、timeline、project、summary、treatment；commands 中 estimate、timeline、project 的记录器回执 |
| materials | 采用或生成的媒体、相关 Result、ASR/校订证据；说明商品、人物、台词为何仍满足本次需求 |
| finishing | files 中 plan、result、video、author、run；commands 中 plan、build 的回执 |
| review | files.report 为当前视频的原生 review；另存实际视觉检查与创意状态 |
| retrospective | files 中的 WORKFLOW_REVIEW.md，记录差异、归属事实、修复与复验 |

`commands` 中的每份回执也使用 path/sha256 文件事实。Plan/Build 必须晚于本次蓝图生成。采用已有材料时，审计检查 Run 的显式候选，且不允许选定 Plan 意外执行 image/video/voice/transcribe。最终文件必须等于 Result 的 final.video，review 必须绑定其字节，字幕覆盖证据必须通过。

```bash
python3 scripts/check_workflow_run.py audit output/<case>/WORKFLOW_RUN.json \
  --output output/<case>/workflow-audit.json
```

缺阶段、证据变化、无原生命令、手工布局替代、使用旧布局、不受控生成或审查对象错误都会返回非零。检查器核对本地结构和记录，不能证明人工描述、商品身份、完整口型和水印接受度正确；这些仍需要实际审片。

## 复盘必须回答的问题

WORKFLOW_REVIEW.md 按五阶段列出“skill 要求、实际执行/复用、证据、是否符合”。每个运行问题记录发现时机、影响、归属事实和处理结果：

- 流程缺失或与工具行为冲突：修正 skill 或对应原生能力，说明为何改变及新版验证。
- 流程合理但漏跑：保留偏离记录，增加预检/证据约束，再修复下游。
- 工具或 Provider 失败：保留真实回执；停止无意义重试，修复实际原因。失败是检验流程的证据。

事后修正只证明修复运行通过，不能把原运行改成“从一开始就符合流程”。未完成创意审查时，流程检查可以 passed，但 delivery_ready 必须保留 false。
