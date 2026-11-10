---
name: audiovisual-craft
description: 当已有 target_script.svml 和初始 project.json，需要用 clone-any-video 准备配音、视觉、混音、字幕并验证正式生产素材时使用。由外部 Agent 调用 CLI 完成制作，保留实际媒体时长、来源和验收证据。
---

# 视听导演工艺与多轨制作

工具入口：当前仓库根目录的 `./scripts/clone-any-video`。先根据会话项目位置定位同时含 Cargo.toml、chapters 和 scripts 的根目录；不要假定作者的本机路径。当前命令是 `voice`、`image`、`video`、`mix`、`caption`、`capture`、`compose`、`media`。细节按需读取 [正式生产指南](../../../docs/production-remediation.md)。

正式生产使用真实 Provider 和 Chrome；只有教学任务才显式加 `--mock`，合成占位音效使用 `--mock-sfx`。遵守用户已选模型；本机视频默认 Gemini Omni Flash 360p，语音 MiMo。

进入本阶段前必须已有主 clone-any-video skill 阶段二原生生成并校验的 target_script.svml、target_timeline.json、project.json、BLUEPRINT.txt 与命令回执。缺少时返回阶段二调用工具，不得手写布局或复制旧工程改名替代。可见同源口播使用 project --speaking-video；无新增音乐使用 --no-bgm。任务图 project-timing.layout 读取这份 project.json，再以实际声音求解新窗口。

1. **先采用表演，再确定时长**：可见口播先按 [同源口播](../../../docs/acoustic-performance.md) 使用 dialogue 视频与 adopt-performance；以下 TTS 路径用于独立旁白：`voice batch --script target_script.svml --voice Mia --language en --out-dir work/assets/voice`。音色按实际语言和用户要求选择，不靠导演提示替代 preset。检查实测时长和表演；需要缩停顿/变速时用 `media prepare-audio` 另存并记录变换，再 `voice realign --script target_script.svml --project project.json --audio-dir work/assets/voice --out-extents work/speech_extents.json --out-project work/project.aligned.json`。具体参数依 `--help`。
2. **按最终窗口生产视觉**：`image batch --project work/project.aligned.json --reference-image official-product.png --flow-project-id <uuid> --out-dir work/assets/visual`，再 `video batch --project work/project.aligned.json --visual-dir work/assets/visual --out-dir work/assets/video`。先准备人物/商品参考，再写动作与构图；检查包装与运动。同工程复用 `.flowkit-project.json`，浏览器当前项目不能覆盖已有绑定。
3. **保留有理由的镜头**：Segment、生成请求和编辑镜头不是一一对应。延续动作优先同源连续时间；不能为凑镜头数默认做 crop 放大或重复源帧。`media edit` 使用规范化后的整数帧范围和显式 zoom/continuity_group/cut_intent。不同生成请求需要继承实际首尾状态时，按 Provider 能力准备真实边界帧。
4. **混音看实际声音**：`mix plan/render` 从真实音频活动计算闪避，短气口可能被 margins 合并，不保证每次呼吸都恢复音乐。A-roll 保持独立边界。Moment 音效依具体叙事决定，需已准备 SFX 或 `--sfx-plan`；缺失即修复输入，不能在正式任务生成 Mock。母带使用 `media prepare-audio --target-lufs -16 --max-true-peak -1` 的值需依交付要求选择，并实际测量/试听。
5. **广告组件由 Agent 创作**：内置 auto 是教学模板。按品牌制作 HTML 并提供确定性的 `window.renderFrame(frame, progress)`，使用真实 `capture record/batch`；一段复用一个 Chrome 页面。字幕用 `caption schedule` 的 ASR/校订证据，随后 `media render-captions` 产生透明层并更新清单。ASR 词时间戳不等于 forced alignment；修正识别文本用 `media revise-transcript` 保留旧窗口和理由，去静音剪辑后重新转录。
6. **选择实际准备的工程并验收**：`build target_script.svml --prepared-project work/project.final.json --project-dir work --output work/master.mp4`，或 `compose render --project work/project.final.json --project-dir work --output work/master.mp4`。burn-in 缺 libass 会报错；Chrome 字幕层可不依赖 libass。软字幕只能显式 `--subtitle-delivery soft`。

`mix render` 记录的 `master_audio` 是最终合成的实际输入；母带另存后同步更新这个路径。字幕文字或窗口变化时重新 render-captions，不能只复用相同 cue ID。

完成后用 `media inspect-boundaries` 查看每个真实切点前/中/后和完整成片。`review` 的 `passed` 是技术检查；商品身份、口播/字幕、视觉连续性和实际水印需单独审片，证据绑定成片字节。`review --evidence review.json --require-delivery` 才阻断未完成的创意验收。来源 production 不能代表无水印或包装逐字正确。找到问题先修对应脚本、源采样、几何、组件或音频，再重跑下游。

需要可重复生产与局部重建时，将上述原生步骤写入 JSON Author + Run；先 `plan` 检查实际请求，`build` 执行，`result` 查结果。按 [生产任务图](../../../docs/production-graph.md) 声明每镜头引用、speech/full 脚本投影和明确的候选绑定。修改需求后复核 satisfy；reuse 仅从指定 Result 按任务身份复用，不自动搜索历史。先获取真实配音时长再确定视觉规格；现有手工 CLI 仍可用于需要 SFX/复杂剪辑的准备阶段。

涉及关键词触发卡片、切镜或 SFX 时，按 [质量时序与采样](../../../docs/quality-timing-and-sampling.md) 使用 `align-speech → project-timing`：配音依赖 speech，时钟依赖 timing，字幕依赖 full，全部消费者采用同一份词证据。布局保留完整 timing 绑定；HTML 可消费整程序 clock，合成检查组件的时钟身份。源视频不足默认报错，明确选择 source-in/out、rate、hold 或 loop，不能靠隐式冻结凑长。

FlowKit 视频现可选首帧/首尾帧/多图参考：图中 `FirstLastFrame` 连接 first_frame 和 last_frame；`ReferenceImages` 连接有序 reference 输入，最多七张图片，不能和首尾帧混用。CLI 用 `--last-frame` 或重复 `--reference-image`，保留用户指定 Omni Flash 360p。ReferenceTransfer 动作迁移仍未接入；原片局部修改用 `SourceVideoEdit`：图中声明 source_video，action.source_edit 指定 Flow 30 fps 起止帧，CLI 用 `--source-video` 与 `--source-edit` JSON。可显式绑定同项目、同文件指纹的已有上传，不能把参考图片生成当作原片编辑。当前仅接 Omni 1.2 Flash 360p，最长 10 秒（300 个编辑帧）；请求成功后必须核对真实对白、嘴型、字幕和水印。可见口播按 [声学对齐与同源口播](../../../docs/acoustic-performance.md) 使用 Video dialogue 输入 → adopt-performance，采用同次表演的声音与画面；独立旁白继续用 MiMo。真正声学测量在 ASR 后加入 force-align，缺词或置信不足先检查诊断，不能插值冒充观测。

当前 Flow 上传真人视频的语音编辑受产品策略禁止（2026-11-09 已由网页回执确认）。SourceVideoEdit 只用于保留原声的视觉修改；图中 SourceVideoEdit + dialogue 会预检阻断，不要反复提交或靠提示词规避策略。修改原视频表演必须先接入明确开放该能力的其他后端，随后核对真实声画，不能只覆盖 TTS。

用户允许重新生成动作且不要求音色一致时，可用独立的新表演路径：`extract-frame(video, at_seconds) → image(截图 + 商品参考) → ImageToVideo(first_frame + dialogue) → adopt-performance → transcribe`。不传原视频/原声音作为生成参考，不使用 MiMo 覆盖嘴型。先检查新场景图的商品、手部、人物与旧字幕清理，再显式复用 Result 生成整段。模板见 `examples/still-performance/`；13 秒等长口播设置 `FLOWKIT_VIDEO_OMNI_VERSION=1.2`，窗口向上取整到 3–15 秒，保持 Omni Flash 360p，避免为窗口限制机械拼接。API 接受不代表实际台词/口型合格，提供方拒绝时保留回执并停止该请求。

该广告路径默认还要完成实际声音对齐、字幕排期、真实字幕层渲染和声画配对合成。模型视频要求不生成字幕，是为了后期使用可验证的文字与时间；不能将无后期字幕的 speaker.video 预览当作完整广告交付。模板默认 run.json 导出带字幕 final.video，run-preview.json 仅用于检查模型原始视频。

每次真实执行或修复后，按主 skill 产出 WORKFLOW_RUN.json、WORKFLOW_REVIEW.md，运行 scripts/check_workflow_run.py audit 并保存 workflow-audit.json。逐项比较实际执行与流程，记录问题归属、流程修改或防漏约束；保留原始失败和修复记录，不能事后补文件追认原运行合规。
