# MiMo TTS 与 FlowKit 媒体接入

统一入口 `scripts/clone-any-video` 在启动程序前加载项目根目录 `.env.local`。
该文件被 Git 忽略；可复制 `.env.local.example` 后填写自己的配置。
直接使用 `cargo run` 时，需要自行把相同变量导入环境。

| 能力 | 变量 | 实现 |
| --- | --- | --- |
| TTS | `TTS_PROVIDER=mimo`、`MIMO_API_KEY` | `mimo-v2.5-tts` 预置音色，非流式 WAV |
| 图片 | `IMAGE_PROVIDER=flowkit` | 本机 FlowKit `/api/flow/generate-image` |
| 图生视频 | `VIDEO_PROVIDER=flowkit` | 上传首帧后调用 `/api/flow/generate-video` |
| 文生视频 | `VIDEO_PROVIDER=flowkit`，省略首帧 | `/api/flow/generate-video-omni-text`，Omni Flash |

OpenAI 旧接口继续可用：分别把对应 provider 设为 `openai`，使用原有
`OPENAI_API_KEY` 和可选 `OPENAI_BASE_URL`。MiMo 和 FlowKit 不会占用这些变量。
`--mock` 仍明确使用教学模拟器，生产请求失败不会自动降级。

## MiMo

根据[官方说明](https://mimo.mi.com/docs/zh-CN/quick-start/usage-guide/audio/speech-synthesis-v2.5)，
请求发送到 `/v1/chat/completions`，目标台词放在 assistant 消息，导演描述放在 user 消息，
响应的 `choices[0].message.audio.data` 为 Base64 音频。适配器解码并探测音频后发布文件和 JSON 元数据。

```bash
./scripts/clone-any-video voice synthesize --segment-id demo \
  --spoken '你好，这是视频复刻项目的语音测试。' \
  --direction '自然清晰的普通话，温暖、从容。' \
  --voice 冰糖 --output output/demo.wav

./scripts/clone-any-video voice batch --script target_script.svml \
  --out-dir output/assets/voice --direction '自然清晰的普通话'
```

批量命令读取脚本的 Spoken 投影，支持 `--voice` 和 `--language`；未显式选择时由 `MIMO_TTS_VOICE` 指定。结果和 provenance 保存实际预置音色、语言及实测时长。
可用预置音色包括冰糖、茉莉、苏打、白桦、Mia、Chloe、Milo、Dean。
当前适配器支持基础预置音色模型；voice-design/voice-clone 需要另外的输入契约，尚未开放。
返回时长、采样率和声道以 ffprobe 实测为准，不把估算时长当作结果。后续按现有
`voice realign` 和 `mix render` 流程继续执行；生成 WAV 不能代替词级转录和对齐。

## FlowKit

`FLOWKIT_BASE_URL` 默认 `http://127.0.0.1:8100`；需要服务运行、扩展连接、Chrome 已登录且 Flow 页面打开。
参数契约以[本机接口文档](http://127.0.0.1:8100/docs)为准。
可选 `--flow-project-id` / `FLOWKIT_PROJECT_ID`；优先复用工作区 `.flowkit-project.json` 绑定，首次无绑定时读取浏览器 Flow 项目；已有绑定冲突会明确失败。图片模型默认使用服务配置，
也可设置 `FLOWKIT_IMAGE_MODEL`。视频默认 Gemini Omni Flash 360p：`FLOWKIT_VIDEO_FAMILY=omni_flash`、`FLOWKIT_VIDEO_RESOLUTION=360p`。
可显式改用 `veo` 或 `720p`；Veo 接口的分辨率由 FlowKit/模型决定。分辨率进入任务回执身份，切换设置不会误复用旧任务。
默认 `FLOWKIT_VIDEO_OMNI_VERSION=1.1` 的 Omni 时长向上选择 4/6/8/10 秒；显式设置 `FLOWKIT_VIDEO_OMNI_VERSION=1.2` 可使用 3–15 整秒，例如声明 12.5 秒发送 13 秒。版本进入图任务与请求回执身份，Plan 和执行入口在上传前检查窗口；已有上传原片的 `SourceVideoEdit` 继续固定使用独立的 360p 编辑接口。Veo 的具体时长由服务/模型决定。保存实测时长和分辨率，不假定符合请求估算。

无需保留音色的口播可用 `video generate --first-frame scene.png --dialogue-script script.svml --segment presentation ...`；它只传图片和台词，模型选择自然声音并生成声画。截图换商品后重新生成的完整任务图模板位于 `examples/still-performance/`。

```bash
./scripts/clone-any-video image generate \
  --person 'A white ceramic cup, no people, no text' --shot 'Wide tabletop shot' \
  --setting 'Soft morning daylight' --aspect-ratio 16:9 --output output/cup.png

./scripts/clone-any-video video generate --first-frame output/cup.png \
  --motion '0-4s: Steam rises gently. 4-8s: The camera slowly pushes in.' \
  --duration 8 --output output/cup.mp4

./scripts/clone-any-video video generate \
  --motion '0-2s: A paper boat drifts slowly. 2-4s: Ripples spread outward.' \
  --duration 4 --output output/boat.mp4
```

`image batch` 和 `video batch` 使用相同 provider 配置。
视频提交后保存 `*.flowkit-job.json`，轮询完成、下载并探测媒体后才返回成功。
中断或超时后，重复相同命令恢复该回执，不重新提交模型生成；输入改变需要新的输出路径。
回执包含媒体访问链接，和媒体一起留在本地工程输出目录。媒体 GET 遇到传输中断最多重试 3 次，生成 POST 不自动重试。服务返回 429、502、断连或失败任务会明确报错，
不会通过自动重试生成提交绕过服务节流。

## 验证

```bash
cargo build -p ch16_video_agent_final --offline
python3 scripts/test_media_providers.py
python3 scripts/test_pipeline_contracts.py
cargo test -p ch16_video_agent_final --offline
```

离线契约测试覆盖 MiMo 单段/批量、Spoken 投影、坏音频拒绝，FlowKit 图片、首帧上传、视频轮询、
回执恢复与 HTTP 错误，测试仅访问临时本机 HTTP 服务，不调用付费接口。

2026-11-08 本机实测：

- MiMo 基础 TTS 成功生成 3.2 秒、24 kHz、单声道 WAV。
- FlowKit 扩展移除重复 reCAPTCHA 库加载并修复账号路由后，从本项目统一 CLI 成功生成 1376×768 图片，约 23 秒。
- 图生视频的首帧上传、任务提交、轮询和下载均已通过真实接口验证。
- 按当前默认配置，Gemini Omni Flash 360p 成功生成 4.032 秒、640×360、24 fps 视频，约 55 秒；FFmpeg 全文件解码无错误，并抽帧确认茶杯与蒸汽画面。
- 之前已提交的 Veo 测试任务下载恢复成功（7.93 秒、1280×720）。后续默认采用 Omni Flash 360p。
- 曾出现 CDN 响应体提前断开。客户端只对媒体 GET 做最多 3 次重试，不重试生成提交；已有任务也可从回执恢复下载。
- 非空首帧的图生视频已真实验证；Omni 纯文生视频目前只有受控接口测试，修复后尚未另做真实生成。

旧的 `no ogiZ0b/YhhmEf/maseQ envelope` 502 为修复前记录，不代表当前生图/图生视频状态。
本机 FlowKit 源码位于 `/Users/stoic/Code/skills/flowkit`，其扩展本地补丁应在更新上游时保留。

商品参考图、回执导入及同工程项目身份见 [正式生产修复指南](production-remediation.md#商品参考与-flow-项目)。

### 切换 Flow 账号与项目

先在当前账号创建或打开实际可访问的项目，使用页面 URL 中的 project UUID。当前 FlowKit 的 status.flow_project_id 可能是服务端配置的旧值，extension connected 也不能证明旧项目仍可访问。

已有工作区绑定其他项目时，建立新的生产工作区，保留旧 Result/回执用于历史。新目录的 .flowkit-project.json、Author 内每个 image/video action.flow_project_id 和本次显式项目参数都指向新 UUID。Author 参数会优先于环境变量，仅修改 FLOWKIT_PROJECT_ID 不足以迁移任务图。

清空 source_edit.uploads 中旧账号的媒体绑定，让适配器重新上传原片和本地参考图。此前下载到本地且来源仍可验证的素材可明确采用，但旧 media_id、远端操作回执和未完成 job 不跨账号复用。先运行 Plan，再以实际小图生成验证账号/项目路由，并记录迁移前后项目及工作区。

原片编辑时长与新视频生成时长不同：2026-11-10 Chrome 网页直测明确提示编辑最多 10 秒。SourceVideoEdit 限制每次 300 个编辑帧（30 fps），不沿用 Omni 1.2 新生成的 15 秒上限。超长原片必须明确选择片段或设计分段方案，不得静默截短整片需求。

同日网页直测：相同的上传人物口播原片与虚构人物参考，在选取 0–10 秒、明确只换人物并保留原声后，网页显示 3% 排队，随后 Failed，原文为 “Our policies prevent editing or generating speech for people in uploaded videos.” 因此“未改台词”不能保证这类换人编辑通过；本案例不能归因于网络或把它当作仍在排队。保留网页失败事实，不自动切换成截图重演或宣称其他都不变。
