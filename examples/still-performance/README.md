# 截图约束的新商品口播

适用于允许重新生成动作、表情和声音，只需延续截图中的人物、场景与构图的任务。原片只在本地抽取无标注 PNG；图片接口接收截图与商品图，视频接口接收换好商品的图和新 Script。**没有音频引用，没有原视频上传，也不要求原音色或逐帧动作一致。** 保留原片表演、原声只换商品时，继续用 `SourceVideoEdit`。

准备自己的 `source.mp4` 与 `product.png` 放入本目录，修改 author.json 两处 `flow_project_id` 为同一个真实 Flow 项目，修改 `at_seconds`、图像导演规格、新台词与视频窗口。模板内的通用文案需按实际商品事实改写。文件路径相对 Author。FlowKit 服务、Chrome 扩展和 medium ASR 服务须已就绪。

先按主 clone-any-video skill 完成原片 probe、medium 转写、边界与视盘，保存 ANALYSIS.md、TIMELINE.md 和当前 TREATMENT.md；已有同字节证据可显式复用。修改 target_script.svml，再运行阶段二。以下为人物口播、无新增音乐的例子：

```bash
python3 scripts/check_workflow_run.py record --workspace examples/still-performance --id estimate -- ./scripts/clone-any-video script examples/still-performance/target_script.svml --estimate
python3 scripts/check_workflow_run.py record --workspace examples/still-performance --id timeline -- ./scripts/clone-any-video dynamic-timeline examples/still-performance/target_script.svml --fps 24 --gap 0 --json --to examples/still-performance/target_timeline.json
python3 scripts/check_workflow_run.py record --workspace examples/still-performance --id project -- ./scripts/clone-any-video project examples/still-performance/target_script.svml --timeline examples/still-performance/target_timeline.json --width 360 --height 640 --speaking-video --no-bgm --validate --json --to examples/still-performance/project.json
./scripts/clone-any-video project examples/still-performance/target_script.svml --timeline examples/still-performance/target_timeline.json --width 360 --height 640 --speaking-video --no-bgm --validate --to examples/still-performance/BLUEPRINT.txt
```

这里不提供手写 layout.json。所有 Run 包括中间场景图 Run 都声明 blueprint.value Target；缺少原生蓝图时 Plan 会失败。后期的 layout 直接读取 project.json，再以实际声音重建时钟。

```bash
./scripts/clone-any-video admit-media --asset examples/still-performance/source.mp4
./scripts/clone-any-video admit-media --asset examples/still-performance/product.png
FLOWKIT_VIDEO_OMNI_VERSION=1.2 ./scripts/clone-any-video plan examples/still-performance/run-scene.json
FLOWKIT_VIDEO_OMNI_VERSION=1.2 ./scripts/clone-any-video build examples/still-performance/run-scene.json
```

先检查 Result 中 `scene.image` 的人物、商品、手部、包装和旧字幕清理效果。验收后在 run.json 添加 `"reuse":"<上一步实际 result.json 路径>"`，再执行：

```bash
FLOWKIT_VIDEO_OMNI_VERSION=1.2 ./scripts/clone-any-video plan examples/still-performance/run.json
FLOWKIT_VIDEO_OMNI_VERSION=1.2 ./scripts/clone-any-video build examples/still-performance/run.json --output output/still-performance.mp4
```

不要在第一步之后修改 Producer 代码、截图时间、商品图或图像提示再假定还能复用；Plan 应明确将 frame/scene 标为 Reuse。选择已有验收图也可通过 Run 的显式 Candidate + satisfy 完成。

`FLOWKIT_VIDEO_OMNI_VERSION=1.2` 使用当前 FlowKit 的 Omni 1.2，窗口向上取整到 3–15 整秒，默认仍为 Gemini Omni Flash 360p；未指定版本时保留 1.1 的 4/6/8/10 秒窗口。视频模型决定实际返回时长和音色。不要把超过窗口的台词塞入请求，也不要靠静帧补齐未说完的话。

视频和声音由同次生成返回，`adopt-performance` 成对采用；medium 转写用于检查实际台词。默认 `run.json` 继续执行 `align-speech → project-timing → caption-schedule → render-captions → compose → review`，导出有声、烧录新字幕的 `final.video`；阶段二生成的 `project.json` 提供初始蓝图，实际时长由采用的表演建立。

字幕文本来自 Script 的 Display，时序来自实际声音的词证据；用 `||` 把长句拆成短字幕。ASR 的缩写/拼写若与 Spoken 不一致，先用 `media revise-transcript` 做有理由、保留旧窗口和来源的校订，再显式选择校订证据；不要把任意目标台词伪装成识别结果。ASR 词时间戳不等于声学强制对齐。

只检查原生模型视频时，使用 `run-preview.json`，Target 为带原生音轨的 `speaker.video`；它没有后期字幕，不能当作带字幕最终导出。`performance.video` 是供合成使用的无声画面，必须与 `performance.audio` 成对使用。技术 review 通过仍需逐项审查口型、台词、人物、商品和水印；若提供方拒绝生成，保留回执并停止该请求，不反复改提示词尝试绕过限制。

真实运行的 Plan/Build 使用主 skill 的 record 包装保留回执；每次完成后生成 WORKFLOW_RUN.json、WORKFLOW_REVIEW.md，并运行流程审计。参见 docs/workflow-audit.md；该例的 blueprints/素材/字幕不能绕开主 skill 的五阶段。
