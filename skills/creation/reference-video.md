# 理解参考原片 (Understanding a Reference Video)

> **触发时机**：当用户提供一个参考视频文件（或视频链接），要求“以这个视频为模板”、“复刻这条视频”或“借鉴其节奏与形式”时阅读并执行此 Skill。  
> **核心产出**：时间码级别的视听语义解读报告（`ANALYSIS.md` 宏观模型与 `TIMELINE.md` 逐秒对位），明确原片传达了什么，以及它是如何通过具体的视听手段实现这一传播效果的。

---

## 0. 核心心法：保留功能，重构形式

1. **从宏观整体到微观细节**：
   从头到尾完整通读原片。关注其开篇钩子（Hook）、核心论点、注意力转移点、情绪高潮（Payoff）以及预期的观众反应。
2. **镜头边界用于辅助导航，语义与连续性决定归类**：
   一个在镜头切换后依然存续的对象（如常驻画面的小黑板、统一风格的字号、连续播放的背景音乐），其生命周期超越了单个分镜。记录一次系统，并在时间轴中定位其状态演变。
3. **拒绝主观幻觉，必须拿物理证据说话**：
   大模型绝不能仅凭模糊印象编造分析。每一次判断必须有物理时间码、台词对位或画面视盘佐证。

---

## 1. 让时间可见：工具矩阵调用规范

在分析视频时，Agent 必须调用 `clone-any-video` CLI 工具箱获取确定性的物理证据。

### 1.1 主流程核心工具

- **`probe`（物理特征探测）**：
  获取视频的精确时长（秒）、画面宽高比、帧率（FPS）以及是否存在音频流。
  ```bash
  clone-any-video probe <source.mp4> --json
  ```

- **`transcribe`（听觉骨架与词级对齐）**：
  提取原片的高精度语音转录与词级时间戳（生成 `clone-any-video.transcript@1` 规范），为全片的台词、节奏、停顿气口建立时间刻度基准。
  ```bash
  clone-any-video transcribe <source.mp4> --language zh --to <notes/transcript.json>
  ```

- **`boundaries`（机械分镜与视觉突变点定位）**：
  通过 32×32 宏观色彩空间 L1 差分算法，毫秒级扫描视频中画面反差剧烈的物理跳变点（Candidates），辅助导航。
  ```bash
  clone-any-video boundaries <source.mp4> --rate 12 --threshold 0.1 --json
  ```

- **`tile` 与 `tiles`（声画合一全景视盘生成）**：
  将关键帧以网格形式（如 3×2、3×3）排版为单张图片，并在每格下方自动附带**高亮时间码**与**当时正在播出的台词单词**，供视觉模型进行单次全局注意力洞察。
  ```bash
  # 生成黄金开篇 3x2 全景视盘 (带台词)
  clone-any-video tile <source.mp4> --start 0 --end 6 --frames 6 \
    --transcript <notes/transcript.json> --columns 3 \
    --to <notes/evidence/opening_grid.jpg>

  # 长视频分页生成多页视盘矩阵
  clone-any-video tiles <source.mp4> --transcript <notes/transcript.json> \
    --frames 12 --columns 3 --rows 2 \
    --to <notes/evidence/paged_grids>
  ```

- **`frames`（离散/等间隔关键帧抽取）**：
  提取选定时间戳或时段的高清单帧，用于深入检视排版文字、LOGO、色彩构成或人物微表情。
  ```bash
  clone-any-video frames <source.mp4> --at 1.0,2.5,4.0 --label-time \
    --to <notes/evidence/keyframes>
  ```

- **`cut`（物理片段提取）**：
  截取具有代表性的微观片段，建立动作或动态演变的参考证据样本。
  ```bash
  clone-any-video cut <source.mp4> --start 2.0 --end 4.5 --output <notes/evidence/clip_hook.mp4>
  ```

<!-- 待后续扩展工具：
- fetch: 当用户提供网页视频链接 (如 Bilibili/YouTube/TikTok) 时，调用 yt-dlp 自动抓取并保存本地 MP4
- snapshot: 从无头 Chromium 浏览器捕获渲染画布快照，用于与原片进行像素级视觉对比
-->

---

## 2. Agent 标准执行工作流

当 Agent 被指派分析一条原片时，必须严格执行以下四步标准化流程：

### 第一步：物理探测与时空骨架建立
1. 执行 `probe` 命令获取时长、分辨率和帧率；
2. 若存在音频，执行 `transcribe` 生成 `transcript.json`，获得全文字词对齐。

### 第二步：视觉突变与分镜定位
1. 执行 `boundaries` 获取所有可能发生切镜的时间戳列表；
2. 对比 `transcript.json` 中的句子起止时间，寻找“声画对位点”（例如台词停顿与画面切换的交汇瞬间）。

### 第三步：全景视盘观察
1. 针对全片执行 `tile`，生成融合时间码与台词的 Contact Sheet 图片；
2. 视觉大模型阅读这张视盘，提取画面机位（景别）、主体动作、包装图表（MG）与字幕风格。

### 第四步：沉淀结构化逆向档案
在项目目录下输出两份核心笔记：
1. **`ANALYSIS.md`（宏观叙事模型）**：
   - 原片的叙事诉求（吸引注意力、建立权威、论据累加、促成行动）；
   - 视觉系统的分类（A-roll 人物主画面、B-roll 实物演示、MG 动态图表、字幕排版）；
   - 贯穿全片的核心恒定元素（人物身份、色调体系、背景音乐）。
2. **`TIMELINE.md`（时间码精读对照）**：
   - 按秒级时段划分（如 `0.00s–2.30s · 开门见山抛出痛点`）；
   - 明确每个时段内：谁在说话、画面展示什么、字幕如何运动、为什么这样设计。

---

## 3. 产出规范模板

### 3.1 `ANALYSIS.md` 模板
```markdown
# 原片宏观解读：[作品标题]

- **时长**：X.X 秒 | **画幅**：WxH (比例) | **帧率**：XX fps
- **核心目标**：[该视频试图解决什么问题，激发观众何种情绪]

## 1. 叙事结构设计
- **Hook (前 3 秒)**：[如何抓住观众眼睛]
- **论点推进**：[逻辑层层递进的方式]
- **Payoff 与 CTA**：[收尾行动号召]

## 2. 视听系统拆解
- **视觉层 (Visuals)**：[实拍 A-roll / 插画 / 动效卡片]
- **字幕层 (Captions)**：[出现位置、配色、是否单字逐词高亮]
- **音频层 (Audio)**：[解说音色风格、BGM 情绪起伏、关键音效点]
```

### 3.2 `TIMELINE.md` 模板
```markdown
# 逐秒时空对位记录

## 0.00s – 2.00s · 黄金钩子破局
- **台词发音**："..." (起止: 0.00s - 1.85s)
- **视觉分镜**：特写机位迅速推入，背景高饱和色反差，第 2.0s 发生场景硬切（突变得分 0.66）。
- **视听配合**：台词念到关键词时，屏幕中央弹出大字，声画严格同拍。
- **参考物料**：`evidence/opening_grid.jpg`
```
