# 声学强制对齐与同源人物口播

本能力集中在第 16 章。两条路径分别解决词边界和可见人物口播的声画关系；独立旁白仍使用 MiMo，视频仍按用户选择使用 Gemini Omni Flash 360p。

## 识别、声学对齐与语义时钟

三个步骤各有输入事实：medium ASR 识别实际说了什么；WhisperX 的语言专用 CTC 模型测量这些词在实际声音中的边界；align-speech / project-timing 把观测结果绑定 Spoken Unit 并投影到程序帧。

本机原有 `scripts/whisperx_daemon.py` 是 faster-whisper 转写服务，继续在 8765 运行。新增 `scripts/alignment_daemon.py` 是独立的声学对齐服务，默认 8766；模型未加载时 health / align 返回 503，不回退到 ASR 或估算时间。协议为 `clone-any-video.acoustic-alignment@1`。

### 安装和启动

Intel macOS、Python 3.11 的对齐专用环境可按以下方式准备；与 ASR 环境分开，避免 Torch / NumPy 依赖影响现有 medium：

```bash
python3.11 -m venv ~/.whisper-align-env
~/.whisper-align-env/bin/python -m pip install -r scripts/requirements-alignment-intel-macos.txt
~/.whisper-align-env/bin/python -m pip install --no-deps whisperx==3.8.6
~/.whisper-align-env/bin/python scripts/alignment_daemon.py --prepare
~/.whisper-align-env/bin/python scripts/alignment_daemon.py --port 8766
```

这组依赖只供 WhisperX alignment 路径使用，未安装其完整 ASR、VAD 和说话人分离依赖；不能据此声称完整 WhisperX 环境受支持。英文默认 `WAV2VEC2_ASR_BASE_960H`，CPU 推理。`--prepare` 显式下载 NLTK punkt_tab 与模型并验证加载；普通启动要求已缓存资源。WhisperX 的 torchaudio 路径不遵守 cache-only 标志，因此服务另行检查本地权重，避免普通启动暗中下载。

其他语言需启动相应语言和模型实例；请求语言必须与加载模型一致。模型资源来自 [torchaudio 官方模型](https://download.pytorch.org/torchaudio/models/wav2vec2_fairseq_base_ls960_asr_ls960.pth)。调用依据为 [WhisperX alignment 实现](https://github.com/m-bain/whisperX/blob/main/whisperx/alignment.py)。

### 调用

```bash
./scripts/clone-any-video transcribe work/voice.wav --local --language en --to work/asr.json
./scripts/clone-any-video media force-align \
  --audio work/voice.wav --transcript work/asr.json --output work/acoustic.json \
  --expected-model WAV2VEC2_ASR_BASE_960H --min-word-score 0.5
```

force-align 在推理前核对协议、引擎、模型、语言；有 source_media_digest 的 transcript 必须对应同一份音频。先解码为临时 16kHz mono WAV，模型只在给定的局部 passage 域内对齐。若要改变输入文本，用 revise-transcript 记录显式校订后再测量，不能把台词直接塞入模型就当作它实际说过的证明。

服务调用真实 whisperx.align，`interpolate_method="ignore"`，保留字符、缺词和低置信记录，不用 nearest / linear 填补未测到的词。CLI 原始响应保存在 `<output>.alignment-response.json`；缺词、非法/零长度区间、低于显式阈值或词身份不匹配时失败，不输出可被下游误用的文档。默认 min-word-score 为 0，低于 0.5 的词仍列入 weak_words 供审查；需要阻断时显式设置阈值。

模型已加载不保证每个词都能成功对齐；数字、缩写、模型字表外文字需要听校及适当的发音文本。声学置信度也不能代替台词内容验收。

生产图增加：

```json
{
  "id":"acoustic-hook",
  "action":{"producer":"force-align","expected_model":"WAV2VEC2_ASR_BASE_960H","min_word_score":0.5},
  "inputs":{
    "audio":{"kind":"output","output":"voice-hook.audio"},
    "transcript":{"kind":"output","output":"asr-hook.document"}
  }
}
```

输出 `acoustic-hook.document` 连接 align-speech 的 transcript，随后 project-timing。公开配置 `ALIGNMENT_BASE_URL` 默认为 `http://127.0.0.1:8766`，参与请求身份。Mock 图不能执行 force-align 并伪造测量；离线教学可显式选择测试证据剪去该 Producer。ASR → force-align → align-speech → project-timing 的中间事实均在 Result 中可查看。

## 人物口播采用本次视频的真实声音

图中 Video Producer 可增加 `dialogue` Script 输入，view=speech、单个 Segment/Turn；实际 Spoken 会加入视频请求，要求可见人物用该句台词产生同源表演。dialogue 的 Spoken 内容参与任务身份，改 Display 不重新生成视频。返回视频必须含音轨，但有音轨仍需 ASR / 听校确认它确实说了要求的内容；无对白或错误台词不能以另一段 TTS 偷换声音。

```json
{"dialogue":{"kind":"script","path":"script.svml","view":"speech","segment":"hook"}}
```

接着采用实际声画：

```json
{
  "id":"performance-hook",
  "action":{"producer":"adopt-performance","fps":30},
  "inputs":{"video":{"kind":"output","output":"speaker.video"}}
}
```

输出 `video`、`audio` 和 `document`。adopt-performance 将画面规范化到程序 fps，按实际视频帧数建立等长声域；音频转为 48kHz stereo PCM，保留测得的音轨/视频起始偏移，头部必要补静音、尾部裁切或补齐。记录原始/视频/音频字节身份、帧数、样本数和偏移。缺视频/音频或无法测量原点时直接失败。自备素材先明确登记来源；未知来源不会自动升格为 production。

`performance-hook.audio` 依次进入 transcribe、force-align、align-speech 和 mix；`performance-hook.video` 进入对应 background。最后 compose 校验画面、语义时钟和母带实际使用的配音字节属于同一次表演。多镜头采用同一表演时，各镜头从 `shot.start_frame - voice.start_frame` 连续取源帧，不因切镜从 0 重播。画面越界、换声音、单独变速或错误源窗口会失败。当前支持 rate=1 的成对采用；需要变速时应先同步处理声画并作为新表演重新采用、转录、对齐。

母带的常规响度处理可保留 voice_media_digests；变速/去静音会失去原配对证明，需要重新准备相应画面。图上的原生 mix 记录实际使用的各段声音指纹，任意外部音频文件不能自行冒充配对母带。

这是采用视频模型自身表演的路径。当前没有任意音频驱动嘴型、跨镜头固定音色或 MiMo VoiceClone 对视频口型的接入。生成模型仍可能念错词、嘴型异常、包装变形，需审片；独立旁白继续使用 MiMo preset。

### 换商品和台词，不要求保留音色

先用生产图 `extract-frame` 从原片取一张无标注截图，再将截图与目标商品图作为 `image` 的独立参考，得到换好商品、去掉旧字幕的场景图。该图接入 `ImageToVideo.first_frame`，新文案接入 `dialogue`。模型自行选择自然声音，同次生成画面、口型与语音；无原音频、声音参考或独立 TTS。完整模板在 `examples/still-performance/`。

这条方式保留截图中的视觉身份与环境，动作和表情需要模型重新生成，不承诺原片逐帧一致。用户只希望换商品并保留全部原声表演时，继续使用 `SourceVideoEdit`；当前 Flow 对上传人物视频修改语音有明确限制，原片编辑与新表演必须分开声明与验收。

## 验证边界

离线协议测试使用明确的假服务和合成信号，只验证服务身份、音频身份、缺词、置信阈值与失败状态。Rust 合成色块/正弦夹具验证源帧连续、声画配对、音轨起始偏移和拒绝独立变速。真实生成与声学推理结果另存生产记录；这些测试不保证云端每次表演成功。

```bash
python3 scripts/test_acoustic_contracts.py
cargo test -p ch16_video_agent_final
```
