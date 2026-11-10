# Chapter 16：生产任务图、Run 选择与 Result 复用

外部 Agent/Skill 负责剧本、导演判断、选择素材和审片。现在这些决定可以写入生产 Author JSON，由 `plan` 解释依赖，再由 `build` 调用已有 Rust 媒体工具。这个生产 DAG 与第 6 章用于求解时长的时间约束 DAG 分工不同。

这是 clone-any-video 的原生协议 `@1`，借鉴 Clone Any Video 的 Author / Run / Result 分离，**不承诺解析完整 Clone Any Video SVML/SVS/SVRun 或 Runtime Profile**。新增能力集中在第 16 章；前置章节保留渐进教学，视频关系拒绝与字幕半开区间修复已同步到对应章节。

## 跑通示例

```bash
cargo build -p ch16_video_agent_final
./target/debug/ch16_video_agent_final plan examples/production-graph/run.json --mock
./target/debug/ch16_video_agent_final build examples/production-graph/run.json --mock --output output/graph-demo.mp4
./target/debug/ch16_video_agent_final result /absolute/path/from/build/result.json
```

也可以用 `scripts/clone-any-video`，它读取本地 `.env.local`。`examples/production-graph/run.svrun` 与 JSON Run 表达相同 Targets。此示例使用 Mock 配音、图/视频及字幕层，字幕时间来自**明确标注的教学夹具**；FFmpeg 实际编码，成片仍为 mock，不能交付。正式生产需选择真实 Provider、真实 ASR/校订证据和 Chrome。

`plan` 只读本地声明和媒体（包括 ffprobe），输出 JSON，不调用生成 API、不启动浏览器、不创建构建目录。图的结构或类型错误直接返回非零；缺凭据或生产来源不合格时输出 `blocked` 理由并返回非零。`--to path.json` 可另存计划。`build` 会重新编译当前源，不接受把编辑过的 Plan 当作源。

## Author：声明生产事实

```json
{
  "format": "clone-any-video.author@1",
  "tasks": [
    {
      "id": "voice-hook",
      "action": {"producer": "voice", "voice": "Mia", "language": "en"},
      "inputs": {
        "script": {"kind": "script", "path": "script.svml", "view": "speech", "segment": "hook"}
      }
    }
  ],
  "outputs": {"preview.audio": {"task": "voice-hook", "port": "audio"}}
}
```

每个 Producer 有固定类型端口。`voice-hook.audio` 是自动生成的规范名称，`preview.audio` 是可选别名。图支持前向引用；缺输出、类型冲突、循环、重复 Task、与规范端口冲突的别名、Author/Run/Task 外层未知字段和不支持的输入名会失败。未被 Target 需要的任务仍做结构与类型检查，但不读取其源文件或执行它。

输入有三类：

- `{"kind":"output","output":"voice-hook.audio"}`：引用逻辑产物。
- `{"kind":"file","path":"product.png","type":"image"}`：外部文件，类型包括 audio/image/video/json/project/html。媒体解码/流类型、Project schema 和 JSON 语法会校验。路径相对 Author。
- `{"kind":"script","path":"script.svml","view":"full|speech|timing","segment":"hook"}`：捕获解析后的语义快照。`segment` 可省略；speech 保留 Spoken、角色、段落顺序与时长估算，排除 Display、Cue、Moment、Selection。

`file` 的 `view:"project-timing"` 仅用于 Project：保留时钟、镜头、voice 窗口，移除音频路径、字幕、视觉轨、BGM/SFX 和 AssetTask。对齐阶段可用它避免被版式改动影响；最终合成仍须依赖完整呈现事实。错误投影或漏声明依赖会导致错误复用，Agent 应按任务真实需要选取输入。

### 已接入的原生 Producer

| producer | 必要/可选输入 | 输出 |
| --- | --- | --- |
| voice | script（一个 Segment、一个 Turn）；action 指定 preset/language/direction | audio |
| extract-frame | video；action.at_seconds 指定原片内时间，不附加时间标签、不携带音频 | image |
| image | 每次请求独立的 `reference:<职责>` image；action.spec、flow_project_id、可选 seed | image |
| video | first_frame/last_frame/reference 图片，或 SourceVideoEdit 的 source_video；可选 dialogue speech Script；action.spec、flow_project_id、source_edit | video |
| transcribe | audio（audio 或 video）；action.language、expected_model 默认 medium | document JSON |
| force-align | audio、transcript JSON；expected_model、min_word_score | document JSON |
| adopt-performance | video（真实声画）；fps | video、audio、document JSON |
| align-speech | timing/full script（单 Segment）、audio、局部 transcript；fps | alignment JSON |
| project-timing | timing/full script、完整 layout、每段 `alignment:<segment>`；fps/gap | clock JSON、project |
| realign | script、layout project、每段 `voice:<segment>` audio；fps/gap | project |
| mix | script、project、每个有声段 `voice:<segment>` audio，可选 bgm、sfx:<track_id> | audio |
| caption-schedule | full script、clock JSON 或程序 transcript（二选一） | document JSON |
| render-captions | project、schedule JSON，可选 action.recipe | video、project |
| render-html | html、可选 clock JSON 和 `asset:<名字>` image；action.frames/config | video |
| compose | project、audio、每镜头 `background:<shot_id>` image/video、每组件 `overlay:<id>` video | video、project |
| review | project、video、可选 evidence JSON；可选 require_delivery | report JSON |
| json | 声明的输入，可用 `{"$input":"name"}` 整对象引用 | value JSON |

规格字段与现有 Rust 结构一致，完整示例见 `examples/production-graph/author.json`。`json` 是数据组合工具，不是任意命令或动态脚本执行器。`mix` 图适配目前接 voice/BGM；Moment/SFX 可通过 `sfx:<track_id>` 连接已准备音频；复杂声音准备仍可使用 `mix render --sfx-plan`。

生成参数是声明的值，不会自动根据未知音频时长展开新任务。需要先实测配音时，可先 Target `align.project`，外部 Agent 检查结果后更新镜头窗口与生成规格，再构建视觉和成片。真实镜头取舍仍由 Agent 决定。

HTML 中以 `graph-resource://asset:product` 引用声明的 `asset:product`，执行时转换为资源快照的 file URL，商品图改变会使该组件重建。其他 HTML 应自包含；旧相对目录、隐式网络资源或外部字体的变化不在依赖图中。这里尚未实现完整组件资源 ABI/打包系统。

图生视频实际仍为 FlowKit Gemini Omni Flash 360p。`FirstLastFrame` 连接首尾帧，`ReferenceImages` 连接最多 7 张有序图片参考；高级参考由真实 FlowKit 适配器执行，Mock/OpenAI 明确拒绝。`ReferenceTransfer` 仍未接参考视频输入。MiMo 当前为基本预置音色，不冒充 VoiceDesign/Clone 或完整多角色表演系统。非 Mock 的 transcribe 在推理前核对本机 health 的 model 与 expected_model；现有 daemon 是 faster-whisper ASR，不是 WhisperX forced alignment。

### 直接编辑已有视频

`SourceVideoEdit` 使用 FlowKit 的独立 `jIps6 / macaron_cookie_edit_360p` 路径，传入真实原视频；`ReferenceTransfer` 仍然表示尚未接入的动作迁移。编辑请求不自动附加通用 Camera Motion/推镜头提示，也不会按 4/6/8/10 秒向上取整。

```json
{
  "id": "edit",
  "action": {
    "producer": "video",
    "flow_project_id": "11111111-2222-3333-4444-555555555555",
    "source_edit": {"start_frame": 0, "end_frame": 240, "uploads": []},
    "spec": {
      "shot_id": "original", "first_frame_path": null,
      "motion_prompt": "Replace only the product; preserve the person, setting, gestures and source camera.",
      "camera_motion": "StaticTripod", "relationship": "SourceVideoEdit",
      "duration_seconds": 8, "fps": 24, "width": 360, "height": 640
    }
  },
  "inputs": {
    "source_video": {"kind": "file", "path": "source.mp4", "type": "video"},
    "reference:product": {"kind": "file", "path": "product.png", "type": "image"}
  }
}
```

`source_edit` 的帧号使用 **Flow 的 30 fps 编辑时间基准**，与源文件 fps 独立；区间为半开，最多 300 帧/10 秒。声明时长须与编辑区间相差不超过一个编辑帧。Mock/OpenAI、混入首尾帧、无 source_video 或越过实际源时长都会拒绝。目前只支持已抓取并实测的 360p 编辑入口。

CLI 等价入口：`video generate --source-video source.mp4 --source-edit edit.json --reference-image product.png --motion '...' --duration 8 --width 360 --height 640 --output edited.mp4`，其中 `edit.json` 是上述 `source_edit` 对象。先用 `admit-media --asset <path>` 接受真实输入；FlowKit 扩展需包含视频上传与编辑支持，服务和扩展更新后须重新加载。

已有上传可在 `uploads` 中显式声明 `project_id`、`media_id`、`content_digest`，指纹取自该文件的 `.provenance.json`，必须对应上传时的实际字节。只在同一项目且本地字节匹配时复用；不能拿别的账号、旧文件或 8 秒剪辑的 ID 冒充完整原片。它是可追踪的本地绑定声明，不是服务端字节所有权证明。没有匹配项时会通过 FlowKit 上传媒体；不会把其他生成片自动当成编辑源。

请求回执绑定源视频字节、图片字节、编辑范围、模型、项目和绑定声明；`submitted_request` 保存实际 UUID 与请求。重复同一 CLI 命令只继续轮询，输入变了必须另选输出。`FLOWKIT_VIDEO_POLL_TIMEOUT_SECONDS` 默认 900，可设置有限等待上限；超时保留回执，不自动重新付费生成。图中已有未完成的生成尝试仍须先检查回执或显式选择候选。

当前此路径仅用于保留原声的视觉编辑。图中 `SourceVideoEdit + dialogue` 在 Plan 和执行入口都会拒绝；CLI 的 `--dialogue-script` 也不能与 `--source-video` 混用。旧字幕、嘴型、人物动作和水印仍需要审片。

**当前 Flow 的已知限制（2026-11-09）**：用户在网页端修改上传真人视频的台词，得到明确提示：`our policies prevent editing or generating speech for people in uploaded videos`。因此当前 Flow 原片编辑应按“保留原声的视觉编辑”使用。协议保留 `dialogue` 端口供明确支持该能力的后端，但不能据此承诺 Flow 可以完成真人台词替换。生产测试中，同类 API 请求只返回长期 pending / Media not found，未可靠暴露该策略原因；遇到这种情况应保留证据并停止，不反复提交或更换提示词规避限制。

### 基于截图重新生成商品与新对白

这条路径允许动作、表情和音色重新生成，以截图约束人物、场景与构图。它与严格原片编辑分别声明：

```text
原片 → extract-frame → 无标注截图 ─┐
商品图 ────────────────────────────┴→ image → 换好商品的场景图
场景图 + Script dialogue → ImageToVideo → adopt-performance → medium 转写/验收
采用声画 + 词证据 → align-speech → project-timing → 字幕排期/渲染 → mix/compose → review
```

只有 PNG/JPEG 图片和文字进入生成请求；原片音频不传给图片/视频模型，不创建 MiMo 配音，不指定音色参考。视频模型同时生成可见说话表演及其声音。`extract-frame` 的来源指纹和时间参与依赖，原片或时间变化会使下游失效；Mock/未知来源不会被截图步骤自动升格为 production。

可复用模板位于 `examples/still-performance/`，含 Author、Run、单独检查场景图的 Run 与 target_script.svml；先按主 skill 原生生成 target_timeline.json 和 project.json，不再内置手工 layout。所有示例 Run 的 blueprint.value Target 要求蓝图文件存在，后期 project-timing.layout 使用同一份 project.json。先验收换好商品的图，再显式复用 Result 或选择候选，最后生成整段口播。默认 Run 的目标是经过字幕排期、真实 Chrome 渲染与声画配对合成的 final.video；run-preview.json 才导出无后期字幕的模型预览。直接 CLI 也支持 `video generate --first-frame scene.png --dialogue-script target.svml --segment presentation --motion '...' --camera static_tripod --duration 13 --width 360 --height 640 --output candidate.mp4`，该命令仅生成候选，字幕仍需执行后期流程。

需要 10 秒以上的单次口播时，在 Plan/Build/CLI 使用相同 `FLOWKIT_VIDEO_OMNI_VERSION=1.2`：当前 FlowKit 支持 3–15 整秒，按声明时长向上取整，模型选择和实际请求写入回执，版本参与任务身份。默认 1.1 继续使用 4/6/8/10 秒窗口。使用 `FLOWKIT_VIDEO_RESOLUTION=360p` 保持用户所选分辨率。音轨存在只是一项技术条件，仍须确认新台词、口型、商品与人物实际合格；提供方拒绝时停止并记录。

## Run：选择本次执行目标与素材

```json
{
  "format": "clone-any-video.run@1",
  "author": "author.json",
  "targets": ["final.video", "final.review"],
  "candidates": {
    "accepted-image": {"kind":"file", "path":"accepted.png", "type":"image"},
    "accepted-voice": {"kind":"result", "result":"previous/result.json", "output":"voice.audio"}
  },
  "satisfy": {"image.image":"accepted-image", "voice.audio":"accepted-voice"},
  "reuse": "previous/result.json"
}
```

Run 的路径相对 Run 文件。`satisfy` 是显式选材：所选 Candidate 满足一个逻辑输出，剪掉其默认 Producer 的上游闭包。多个端口的 Producer 只被部分满足时，其他被需要的端口仍可能执行整个 Producer；已选端口不会被执行结果覆盖。

`reuse` 是不同用途：只检查**明确指定的一份**旧 Result 中的 Task，输入身份、Producer 实现、action 参数、revision、相关公开 Provider 配置和 mock 模式都匹配，且输出仍完整，才复用。不扫描历史、不自动选“最佳素材”。

显式 Candidate 不做当前请求身份匹配，因为它正是 Agent 覆盖默认生成的决定；系统检查类型、内容指纹和来源，但无法据此证明“旧口播符合新台词”或“包装正确”。修改 Spoken 或商品时应重新审视这些绑定，必要时删除 satisfy。`reuse` 则会按依赖判断是否失效。两者不能混为一谈。

XML 原生子集如下；未知元素、属性、namespace 或嵌套声明报错：

```xml
<svrun version="1">
  <author source="author.json"/>
  <target output="final.video"/>
  <file id="image" path="accepted.png" type="image"/>
  <result id="voice" source="previous/result.json" output="voice.audio"/>
  <satisfy output="image.image" candidate="image"/>
  <satisfy output="voice.audio" candidate="voice"/>
  <reuse result="previous/result.json"/>
</svrun>
```

旧的 `.svml` 直接 build / prepared-project 路径保留；旧 SVML Author 的 XML Run 仅支持单个 `final.video` Target。传入其他 Target、Candidate、Satisfaction 或 reuse 会报错，不再悄悄忽略。该路径保持原来的手动素材准备方式；只有新的 JSON Author 图会按声明调用生成器。

## Plan 与按依赖重建

每个任务列出 `execute/reuse`、原因和已知 request_key。上游需要执行且新字节尚未知时，下游先保守列为 execute、request_key 为 null；执行后若实际输入与旧 Result 完全一致，可再复用。计划解释“可能执行”，不承诺提前预测模型输出。

`script.view=speech` 的效果由真实 CLI 媒体流程验证：

| 修改 | 复用 | 重建/待确定 |
| --- | --- | --- |
| 只改 Display | voice、时长对齐、mix、image、video | schedule、captions、compose、review |
| 改 Spoken | 无关图/视频分支 | 对应 voice、align、mix、字幕和相关呈现 |
| 改商品参考图 | 无关 voice、mix | 声明引用它的 image → video → compose |
| 改字幕布局 recipe | voice、mix、image、video | captions → compose → review |

这些关系来自图中的真实输入，不是靠全局文件名猜测。action.revision 可在检查后主动指定新尝试；不要用它掩盖未确认的远端任务。

Provider 配置只捕获公开路由/模型，API Key 不写入 Plan/Result。带用户名、密码或 query 的 base URL 会被拒绝，以免把凭据放进计划。实现指纹覆盖当前原生模块及其主要辅助源码，粒度较保守；外部工具版本、服务端模型更新和隐式网络资源尚无完整锁定，不是跨机器位级复现承诺。内容指纹沿用本机缓存算法，不是安全签名。

## Result、失败与所有权

每次 build 在 `<workspace>/.production/results/run-<stamp>-<pid>/` 保存 `plan.json`、`result.json`、每任务的输入快照、输出及原生回执。`--project-dir` 指定 workspace；省略时为 Run 所在目录。中间状态每任务前后原子更新；失败保留已接受输出及诊断。`result PATH` 读取快照，不声称某个进程仍活着。

未完成/失败的同身份外部生成任务，在**显式选入该旧 Result**后会阻断自动重提，要求先查看原生 receipt。执行时也做同样检查，覆盖计划阶段尚未知输入的情况。当前没有全局去重账本、通用 submit/poll/collect、限流队列或跨进程自动恢复；若不选旧 Result，新 Build 不会自动发现它。

只复用的产物继续指向旧 Result 的拥有目录，避免重复生成和复制。外部 Candidate 仍指向原文件。删除这些目录/文件会破坏后续引用；这不是自包含工程导出。`--output file.mp4` 只导出一个视频 Target 与来源记录，不能打包全部工程资源。导出检查目标类型、当前字节和覆盖输入冲突。

来源 sidecar 与字节绑定，真实媒体才能进入非 Mock 图。字幕层在快照和导出时保留文本/帧窗口验收依据，防止搬到 Result 后丢证据。`review` 的 passed 是技术结论；require_delivery 仍要求绑定当前视频的创意验收。失败的 Review 诊断留在任务目录，不能伪装为已接受输出。

## 验证与后续边界

```bash
cargo test -p ch16_video_agent_final --offline
python3 scripts/test_production_graph.py
python3 scripts/test_media_providers.py
```

真实 The Ordinary 广告也通过此图：显式选择五段已验收视频、品牌卡、母带和校订 ASR，仅运行排期/Chrome 字幕/合成/审查。输出为 production、464 帧、30fps；完整解码通过；再次指定 Result 后四个执行任务均复用，没有新生成请求。创意交付状态仍需审片签认，不能因技术通过而自动变成 ready。

当前补齐的是生产图与显式选择的第一阶段。统一持久 Operation/RuntimeProfile、多角色/VoiceClone/VoiceDesign、完整 Clip/组件 ABI、Studio 评论与局部修复闭环仍是后续能力。

## 质量能力补充

关键词级联动使用 `align-speech → project-timing`；speech 投影继续给 TTS，timing 投影保留词与语义事件，full 给字幕。caption-schedule、mix、HTML 和轨道窗口消费同一时钟；旧 realign 的总时长调整保留。compose 的短视频默认不足报错，通过 sampling 显式选择源帧、rate、hold/loop。完整字段、FlowKit 首尾帧/多图参考与验收见 [质量时序与采样](quality-timing-and-sampling.md)。

声学测量新增 force-align（audio + transcript → document），可见口播新增 Video 的 dialogue 输入及 adopt-performance（video → video/audio/document）；对应输入、依赖和帧域约束见 [声学对齐与同源口播](acoustic-performance.md)。
