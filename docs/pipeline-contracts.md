# Chapter 16 管线契约与边界

## 谁负责生产素材

外部 Agent 读取 `skills/clone-any-video/SKILL.md`，调用 CLI 完成分析、创作与素材生产。
终章 `build` / `run` 有两条入口：新的 JSON Author 生产图按 Run 显式声明调用原生 Producer，支持 Targets、Candidates、Satisfaction、plan 和 Result 复用；详见 [生产任务图](production-graph.md)。
旧 `.svml` / prepared-project 入口继续检查已有媒体、生成组件、捕获、合成和技术质检，不自动调用素材生成器。旧 SVML Author 的 XML Run 仅支持 final.video；其他选择明确报错。
两条入口都没有内部 LLM，创作与验收判断由外部 Agent 完成。
生产顺序：`voice batch → voice realign → image batch → video batch → mix render → component/capture/caption → compose render → review`。
以真实配音时长生成最终清单，并显式传给 `compose render --project <project_mixed.json>`。
辅助 `build` 的 `project.json` / `timeline.json` 受脚本指纹缓存控制；复制外部清单不保证它被采用，首次编译或源变化时可能从纸面估算重建。正式生产可使用上述显式工具流程，或 `build --prepared-project <project.final.json>` 保留精确时钟；后者校验输入并跳过估算缓存重建。
图片/视频置于 `assets/visual/<shot_id>.png` / `assets/video/<shot_id>.mp4`，母带置于
`assets/audio/master_mix.wav`。`--mock` 允许生成占位素材，产物始终标记 mock。

## 媒体依赖

工作区 `.media-dependencies.json` 记录已经通过输入检查的 Spoken、角色、Segment 顺序、配音文件和母带的内容指纹。
在组件捕获之前保存，因此失败的下游渲染也不会使旧媒体失去依赖记录。
再次构建时，台词或角色改变而对应配音未更新、或台词/配音改变而母带未更新，均拒绝复用。
生成器的 `voice_<segment>.json` / `<segment>.json` 也会检查 Spoken，覆盖首次构建。
显示文本变化且 Spoken 不变可复用配音与母带。失败时保留现有媒体以便检查。
这些指纹用于本机缓存一致性，不是防篡改签名。

首次导入且没有元数据的外部文件无法反推它对应哪版台词；来源保持 unknown。
生产者需确认外部素材与当前剧本一致，再登记并构建。不要删除依赖记录来绕过过期检查。

## 对齐证据

时间线和字幕共享单调内容匹配器：匹配 Spoken，忽略大小写及标点，支持中文字符与 ASR 词块、双文本、一个 Script Segment 横跨多个 ASR Passage。
ASR Passage 只是证据容器，不按序号充当 Segment 身份。时间线将实测匹配跨度转换为 Segment 局部时间，再投影到程序窗口。
相邻 Segment 共用一个 Passage 时，按各自匹配词的范围划分；没有共享时保留 Passage 的首尾静音。
缺词、缺时间戳、非法时间或无法完整匹配某个 Unit 会失败，不再补造 0.2 秒窗口。

ASR 只有短语级时间时，字符在实测短语范围内均分；这是受证据约束的估算，不是声学字级强制对齐。
没有 transcript 时的纸面时长和字幕线性分配仍是估算，正式生产应另提供真实音频及对齐证据。
匹配器不是模糊纠错器；识别错误应修正证据，不能静默拿别的词替代。

## 来源和验收模式

生成媒体有独立 `完整文件名.provenance.json`，绑定内容指纹，避免同名 PNG/MP4 共用 JSON 覆盖来源。
真实提供商记 production；Mock 生成器、模拟浏览器、模拟合成器和 `mix sfx` 记 mock。
图生视频、混音和合成继承输入：任一 Mock 则输出 mock，否则存在 unknown 则输出 unknown。
真实 Chrome 从 authored HTML 录制组件记 production；无依赖的真实生成亦可记 production。
这表示生产路径，不能代替画面、台词、节奏质量检查。

手工提供并确认真实的素材可显式登记：

```bash
./scripts/clone-any-video admit-media --asset /absolute/path/to/footage.mp4
./scripts/clone-any-video admit-media --asset /absolute/path/to/bgm.wav
```

已知 Mock 不能通过该命令改标。文件内容改变、缺失或无记录时为 unknown；旧版本媒体不会自动升级为 production。
`mix render` 的 Moment 路径现在要求已准备的音效，或传入明确的 `--sfx-plan`；只有显式 `--mock-sfx` 才可生成/复用教学 Mock 音效，Mock 来源沿混音传递。

`review --mode` 仅接受 `production` / `mock`，指定值是验收要求；报告始终输出文件的实际来源。
默认自动报告；unknown 和 mock 不再无条件增加一次 Pass。
非 Mock `build` / `agent` 强制要求最终来源 production。技术分数不替代多模态 Agent 的内容验收。

完整的准备音频、ASR 校订、Flow 参考身份、整数帧编辑、真实字幕和技术/创意分离验收，见 [正式生产修复指南](production-remediation.md)。

## 影响成片的统一时序与采样

终章新增 `align-speech → project-timing`，将真实/校订局部词证据投影为一份共享 clock；timing 视图保留语义 Unit/Moment/Selection，speech 视图仅用于表演依赖。需要逐词跟随的轨道显式绑定 timing，旧的总长 realign 保留为兼容路径。标准合成对短视频默认报错，hold/loop/rate/source-in/out 通过 sampling 声明。视频现支持真实 FlowKit 首尾帧和最多七张图片参考；参考视频迁移与同源口播未因此自动完成。详见 [质量时序与采样](quality-timing-and-sampling.md)。
