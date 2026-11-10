# 成片质量：共享语义时钟、源帧采样与视频参考

本次围绕可听见、可看见的输出差异补齐能力。数据库、任务队列、Studio、完整组件 ABI 继续后置。新增生产图能力集中在终章；共同的视频关系定义同步第 9–15 章，采样实现同步第 15 章。

## 1. 一份词时序驱动字幕、卡片、切镜和音效

`realign` 保留为按配音总时长调整的兼容 Producer。需要关键词级联动时，用 `align-speech → project-timing`：

```json
{
  "id":"local-hook",
  "action":{"producer":"align-speech", "fps":30},
  "inputs":{
    "script":{"kind":"script","path":"script.svml","view":"timing","segment":"hook"},
    "audio":{"kind":"output","output":"voice-hook.audio"},
    "transcript":{"kind":"output","output":"asr-hook.document"}
  }
}
```

`asr-hook` 应对同一份 `voice-hook.audio` 转录，得到局部时间戳；也可以显式连接经过听校的本地 transcript。原生 transcribe 保存 `evidence.source_media_digest`，若存在此字段，align-speech 必须核对音频字节。没有该字段的旧/外部证据是作者显式绑定，不能据此证明已经自动核实媒体身份。

align-speech 要求一个 Segment、有声媒体和完整 Unit，严格匹配 Spoken；输出 `alignment` JSON，记录实际媒体指纹、秒长、局部帧长、单位时序和原始 transcript。证据越出实际音频范围或无法匹配时失败，不补造时间。

```json
{
  "id":"clock",
  "action":{"producer":"project-timing","fps":30,"gap":0.13333333333333333},
  "inputs":{
    "script":{"kind":"script","path":"script.svml","view":"timing"},
    "layout":{"kind":"file","path":"layout.json","type":"project"},
    "alignment:hook":{"kind":"output","output":"local-hook.alignment"}
  }
}
```

每个 Segment 声明一个 `alignment:<segment_id>`。输出为 `clock.project` 和 `clock.clock`。layout 读取完整布局，不能使用会移除组件与 SFX 的 file view `project-timing`。

`script.view=timing` 保留 Spoken Unit、角色、Moment、Selection 和段落顺序，排除 Display 与 Cue；`speech` 仍用于 TTS，`full` 用于字幕。只改 Display 不会重新配音或建立词对齐，但字幕会更新。

每份局部媒体在给定 fps 下取完整帧长，程序 Window 与它等长，gap 也量化到整数帧。语义事件由观测 Unit 时间投影，最后量化到程序帧；不能按句子总长均分。多个 Segment 的量化结果可能与旧的整片一次 ceil 相差少量帧，应使用输出的实际总帧数。

### 轨道绑定

TrackItem.payload 中的 timing 可以绑定背景、组件、字幕、SFX 或 BGM 窗口：

```json
{"timing":{"from":"moment:reveal","duration_frames":15}}
```

```json
{"timing":{"from":"selection:proof:benefit:start","until":"selection:proof:benefit:end"}}
```

可用引用：`program:start/end`、`segment:<id>:start/end`、`moment:<name>`、`selection:<segment>:<name>:start/end`。`offset_frames` 是起点的有符号偏移；until 与 duration_frames 必须二选一。绑定越界或没有长度会失败。Moment 在程序内需唯一，Selection 在 Segment 内需唯一。

背景轨通过 payload.shot_id 关联 Shot，绑定结果会更新对应镜头窗口。原生项目创建器现在为 Selection 组件和 Moment SFX 写入绑定。没有 timing 的旧布局继续采用段落边界的分段线性调整，不能把它称为逐词精确跟随。

### 消费同一个结果

- caption-schedule 使用 `script: full` 和 `clock: clock.clock`。它与旧的 transcript 输入互斥；新路径使用时钟内已投影的同一份原始词证据。
- mix 使用 `project: clock.project`，可连接 `sfx:<track_id>` 音频。播放起点来自绑定后的窗口；没有提供素材的 SFX 不自动生成。配音字节必须符合已绑定的局部域。
- render-html 可接 clock 输入。此时 frames/fps 必须覆盖整个程序；HTML 获得 `window.PRODUCTION_CLOCK`。在既有 renderFrame(frame, progress) 中使用 timeline 的整数帧边界：

```javascript
window.renderFrame = frame => {
  const range = PRODUCTION_CLOCK.timeline.windows.find(w => w.id === 'proof:benefit');
  document.querySelector('#badge').style.opacity =
    frame >= range.start_frame && frame < range.end_frame ? '1' : '0';
};
```

整程序组件记录自己的帧域、fps、总帧数和所用 clock JSON 的内容身份。compose 拒绝其他时钟的整程序组件；当它只在局部窗口出现且没有自定 sampling 时，自动选用对应程序源帧，避免从组件第 0 帧重新播放。

ASR 保持 medium faster-whisper；需要真正的声学观测时，在 transcribe 与 align-speech 之间加入独立 force-align Producer。服务、缺词和置信边界参见 [声学对齐与同源口播](acoustic-performance.md)。完整的 M:N 容错和受约束插值仍未实现。共享时钟解决的是消费者使用不同时间依据的问题。

## 2. 明确选择源帧与素材不足策略

背景和组件 payload 可带 sampling：

```json
{
  "sampling":{
    "from_frame":30,
    "until_frame":90,
    "rate_numerator":1,
    "rate_denominator":1,
    "shortage":"error"
  }
}
```

所有源帧均属于**规范化到程序 fps 后**的半开区间。实际渲染先测帧率与帧数；需要时规范化，得到真实帧数，再验证和编码。归一化临时文件不覆盖原始素材，源时钟/采样策略记录在成片 sidecar。

- from_frame 默认 0，until_frame 默认实际源尾。
- rate 为正有理数；2/1 表示两倍速，1/2 表示半速。采样覆盖完整变速后的源帧间隔，再限制到目标长度。
- shortage 默认 error：素材不足就报错。hold 明确冻结末帧；loop 明确循环选择的源区间。
- 越出真实源帧的 until_frame 始终报错，hold/loop 不会掩盖无效的源区间。
- 静态图片在窗口内持续显示，没有源时间域；不接受这组视频采样参数。

兼容旧清单时，长度足够的素材行为保持一致；此前依赖隐式尾帧填充的工程需要补写有意 hold 或修正素材。命令 `media edit` 仍提供原来的等长 rate=1 剪辑；新的采样在 compose 的背景/叠层路径生效。

视觉 rate 只处理画面。人物可见口播需要同源声画的对应变换，不能独立改变嘴型画面的速度来适配另一段 TTS。

## 3. FlowKit 的真实参考输入

保持 Gemini Omni Flash 360p；当前接入：

```bash
./scripts/clone-any-video video generate \
  --first-frame first.png --last-frame last.png \
  --motion "Continue the product action between these states" \
  --duration 4 --flow-project-id <uuid> --output work/first-last.mp4

./scripts/clone-any-video video generate \
  --reference-image person.png --reference-image product.png \
  --motion "Keep the person, product and setting consistent" \
  --duration 4 --flow-project-id <uuid> --output work/references.mp4
```

图中的 Video spec.relationship 对应 `FirstLastFrame` 或 `ReferenceImages`；媒体通过 `first_frame` / `last_frame` / `reference:<名字>` 输入声明，不能把首帧文件写进 spec 参数。多个 reference 按输入名排序，可用 `reference:01-person`、`reference:02-product` 明确顺序。

- FirstLastFrame：只接首帧和末帧，调用 generate-video 并真实携带两个 media ID。
- ReferenceImages：最多 7 张已接受图片，调用 generate-video-refs；不能与首尾帧混用。
- ImageToVideo 和 TextToVideo 保留。
- ReferenceTransfer 仍拒绝：多图参考不等于参考视频动作迁移。

图片解码、来源和关系组合在上传/提交前验证；末帧和有序参考图指纹进入原生回执及图的输入身份。相同回执继续查询/下载，输入变化不能冒充旧请求。高级参考仅在 FlowKit 适配器落地，Mock/OpenAI 不静默忽略。

这提供模型能接收的约束，不承诺人物和包装必然正确。连续动作需要时使用边界帧；普通 UGC 可以共享人物/场景图后独立生成，不必强制尾帧链。同源口播采用视频自己的声音与帧域，见 [声学对齐与同源口播](acoustic-performance.md)。保留原演员动作只换商品、特定声线和任意音频驱动嘴型仍需相应能力与实样验收。

## 验收

```bash
cargo test --workspace --offline
python3 scripts/test_production_graph.py
python3 scripts/test_media_providers.py
python3 scripts/test_pipeline_contracts.py
python3 scripts/test_production_remediation.py
```

新增测试实际验证：同样 4 秒的句子改变停顿后，字幕/Selection/SFX 随词移动；混音波形在正确时刻出现；整程序组件使用正确源窗口；彩色帧区分裁切、冻结、循环、加速和减速；HTTP 夹具确认首尾帧、多图参考、360p、输入变化后的拒绝续用及提交前关系校验。夹具不是云端生成质量保证。
