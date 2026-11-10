# 正式生产：实测媒体、显式剪辑与交付证据

本页对应 2026-11-08 origin → The Ordinary 实测的 R01–R21。第 16 章是集成实现；前面章节保留逐步教学代码。

## 生产顺序与已准备工程

Agent 先准备真实配音，探测时长并 realign，再生成或剪辑视觉，混音、制作组件和字幕，最后合成与验收。`--mock` / `--mock-sfx` 只用于演示，来源沿下游继承。

```bash
./scripts/clone-any-video voice batch --script work/target.svml \
  --voice Mia --language en --out-dir work/assets/voice
./scripts/clone-any-video voice realign --script work/target.svml \
  --project work/project.json --audio-dir work/assets/voice \
  --gap 0.15 --out-extents work/speech_extents.json --out-project work/project.aligned.json
```

`voice batch` 的 JSON 和来源记录保存实际 voice preset、language、实测时长及预算偏差。TTS 不保证服从估算时长；需要裁停顿、变速或母带处理时使用新输出文件：

```bash
./scripts/clone-any-video media prepare-audio --input voice.raw.wav \
  --output work/assets/voice/voice_hook.wav --trim-silence --tempo 1.13
./scripts/clone-any-video media prepare-audio --input master.unmastered.wav \
  --output work/assets/audio/master_mix.wav --target-lufs -16 --max-true-peak -1
```

工具保留源文件，输出实际时长、变换参数和内容指纹。去静音会改变时间映射，需重新 ASR；单纯 tempo 可显式投影时间。完成配音编辑后再 realign，重新生成受影响的字幕、剪辑和混音。

`build --prepared-project work/project.final.json --project-dir work` 显式使用已准备工程，不拿脚本缓存替换它，也不重建估算镜头。它校验清单、Spoken 身份和已有媒体依赖。背景可由 `tracks.visual_bg[].payload.source` 指向已准备文件；组件 `video_path` 相对 `project-dir` 解析。`mix render` 将实际母带路径写入工程的 `master_audio`，合成和媒体依赖检查均尊重它；未声明时兼容 `assets/audio/master_mix.wav`。若母带后期另存到新路径，Agent 须更新工程的 `master_audio` 后再构建，不能只放一个同名候选文件。

## 商品参考与 Flow 项目

```bash
./scripts/clone-any-video image batch --project work/project.aligned.json \
  --reference-image /absolute/path/official-product.png \
  --flow-project-id 11111111-1111-4111-8111-111111111111 --out-dir work/assets/visual
./scripts/clone-any-video video batch --project work/project.aligned.json \
  --visual-dir work/assets/visual --out-dir work/assets/video
```

`image generate/batch` 支持重复的 `--reference-image` 和 `--reference-media-id`，后者须属于相同 Flow 项目。上传验证真实 PNG/JPEG 和 MIME。先显式登记可信外部参考图，未知来源不会自动变成 production。参考图约束商品身份，仍需看图验收包装和动作。

图片/视频首次选定的项目写入工作区 `.flowkit-project.json`。选择顺序是 CLI → 环境变量 → 工作区绑定 → 浏览器当前项目；已有绑定与显式请求冲突时，在生成前报错。`assets` 下不同子目录共享工程绑定，不会因浏览器切页静默分散项目。视频默认仍是用户指定的 `omni_flash` / `360p`。

外部 FlowKit 已完成图片可执行：

```bash
./scripts/clone-any-video media import-flowkit-image --receipt scene.json --output work/assets/visual/shot_01.png
```

支持单条/数组 request、scene、character 图片回执（数组用 `--index`），实际下载、验证并转为 PNG，保存 media UUID、project、URL、回执指纹。它是显式外部回执导入，不是密码学来源证明。FlowKit 项目 API 对无新增实体、同 ID 同参数的创建返回已有项目；冲突返回 409，使用 PATCH 修改。

## ASR 证据与校订

本仓库 daemon 默认 medium / CPU int8，加载失败返回 503，不再伪造转录。health 与 transcript 保存实际引擎/版本/模型，faster-whisper 词时间戳明确 `forced_alignment=false`。不要把兼容 WhisperX 的协议名当作已运行 WhisperX 声学对齐。

```bash
HF_HUB_OFFLINE=1 /Users/stoic/.whisper-env/bin/python scripts/whisper_daemon.py \
  --host 127.0.0.1 --port 8765 --model medium --device cpu --compute int8
./scripts/clone-any-video transcribe voice.wav --local --language en --to transcript.json
./scripts/clone-any-video media revise-transcript --input transcript.json \
  --corrections corrections.json --output transcript.curated.json
```

校订 JSON 是数组，例如 `[{"passage":0,"word":5,"expected":"HACERUM.","replacement":"HA serum.","reason":"听校并依 Author 确认产品名"}]`。工具核对旧文本、保留同一观察窗口、清空修改词的旧置信度，记录父证据与理由。`--tempo 1.13 --audio voice.edited.wav` 只对纯变速做时间投影；去静音/剪切后的音频须重新转录。缺失置信度保持缺失；低置信度需听校。短语窗口内的字幕分配仍是估算。

## 连续性与整数帧编辑

生成次数、Script Segment 数量、编辑镜头数量不必相等。延续同一动作时，优先使用连续源跨度；不要为凑镜头数制造固定 crop 放大或源时间回退。

```json
{"fps":30,"clips":[
  {"shot_id":"shot_01","source":"scene.mp4","source_start_frame":0,"source_end_frame":56,"target_start_frame":0,"target_end_frame":56,"continuity_group":"action","zoom":1},
  {"shot_id":"shot_02","source":"scene.mp4","source_start_frame":56,"source_end_frame":127,"target_start_frame":56,"target_end_frame":127,"continuity_group":"action","zoom":1}
]}
```

```bash
./scripts/clone-any-video media edit --plan edit.json --project work/project.aligned.json \
  --out-dir work/assets/video --update-project work/project.edited.json
```

工具先将每个源一次性规范化至工程 fps，再按半开整数区间裁剪，记录 normalization、源/目标窗口和 zoom。当前支持等长 1:1 映射；需变速时先显式准备媒体。同一 `continuity_group` 禁止默认源跳跃/尺度阶跃；有意跳切可写 `cut_intent: "intentional-jump"`，仍需审片。不能据结构合法断言语义连续。

合成器使用 `trim=end_frame`、统一帧时基和 `-frames:v`，输出后复查实际帧数。`media inspect-boundaries --project work/project.final.json --video work/master.mp4 --out-dir work/boundaries` 导出每个真实编码切点前后各 3 帧，并绑定当前成片指纹。看切点和全片，修复源时间/几何/导演意图，不靠溶解掩盖错误。

## 字幕和商品版式

`component auto` 是教学模板。广告由 Agent 创作 HTML，提供无墙上时钟依赖的 `window.renderFrame(frame, progress)`，通过 `capture record/batch` 实际捕获。Unix Chrome 使用隔离 profile、CDP pipe 和每段一个持久页面，等待字体/图片就绪，不再每帧启动浏览器。

```bash
./scripts/clone-any-video caption schedule --script work/target.svml \
  --project work/project.edited.json --transcript program.transcript.json --to work/captions.json
./scripts/clone-any-video media render-captions --project work/project.edited.json \
  --schedule work/captions.json --output work/assets/video_components/captions.mp4 \
  --update-project work/project.captioned.json
```

原始 token separator 决定双文本字幕的标点和词间空格，不改 Unit 身份。字幕通过真实 Chrome 渲染为透明层，不依赖 libass。合成清单和来源记录绑定每条字幕的 ID、文字及半开帧窗口；移动/裁短叠层会改变实际覆盖。文字或时钟变化时，旧字幕层会被拒绝，不能只匹配 cue ID。已覆盖的字幕不再重复叠原始 ASS。

`compose render` 默认 burn-in；需要 ASS 但本机无 libass 时明确失败。软字幕必须显式 `--subtitle-delivery soft`；无字幕可显式 `none`。来源不明的旧成片字幕是否烧录保持 Unverified，不能仅凭清单有 captions 就 Pass。

## 混音与交付

`mix render` 以实际音频 `silencedetect`（-40 dB、最短 0.12 s）测有声活动，再计算闪避。它是幅度活动检测，不是词对齐；非常短的气口可能仍被 lead/tail 合并，不能承诺每口呼吸都恢复音乐。计划记录观察窗口和包络，保留 A-roll 位置。实测声音与听感仍须检查。

混音遇到缺失的必需配音或显式指定但不存在的音乐会报错；不会静默生成缺人声或换音乐的母带。音效/配音片段按目标采样窗裁切，执行声明的淡入淡出；包络控制点为绝对线性增益。Moment 的默认路径只接收已准备 SFX；缺失时报错。`--sfx-plan` 可显式输入 `AudioClip[]` 并指定时刻；`--mock-sfx` 才启用教学合成，已知 Mock 来源向下传递。没有音效意图可不加 Moment。

review 增加实际编码帧数、R128 LUFS/true peak 和字幕交付证据。-18 至 -14 LUFS、≤ -1 dBTP 是当前在线口播的提示范围，响度越界是 Warning，可依具体平台要求处理。

`passed` 仅为技术结论。`ready_for_delivery` 还要求 production 来源，以及绑定当前视频字节的显式创意验收。使用 `review --evidence review.json --require-delivery` 才将未完成创意验收作为退出码阻断：

```json
{"artifact_digest":"从 review 输出复制当前 artifact_digest","reviewer":"实际审片人/Agent","checks":[
 {"name":"product_identity","status":"pass","note":"具体包装/产品核验依据"},
 {"name":"speech_and_captions","status":"pass","note":"完整听校与字幕同步观察"},
 {"name":"visual_continuity","status":"pass","note":"实际切点和全片复核"},
 {"name":"watermark_acceptance","status":"unverified","note":"记录实际水印及 Brief 的接受标准"}
],"attachments":[]}
```

状态只接受 pass/fail/unverified，必需项未通过就未就绪；重复的验收项会被拒绝，避免 pass 与 unverified 互相覆盖。可附 `{path,digest}` 的审片证据；视频或证据附件改变则旧结论失效。这是审片断言的可追踪记录，不是自动商品识别、自动 forced alignment 或水印去除工具。

## Clone Any Video 对照

固定本地 Clone Any Video `2d31e887`：`playbooks/craft/voice-direction.md` 的 prepared voice、`generated-dependencies.md` 的参考身份；`packages/visual-track/src/source-time.ts` 的规范化和源/目标帧域；`video-direction.md` 的 Segment 与 Shot 区别；`production/review.md` 的切点前/中/后审查及 owning fact 修复。这里只复用这些职责划分，没有移植 Clone Any Video 全部运行时。
