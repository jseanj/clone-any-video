---
name: reference-video
description: 原片视听理解与逆向工程 Skill。当用户提供参考视频文件，要求分析其时空节奏、台词对齐、分镜跳点、生成全景视盘或复刻视频时使用。
---

# 原片视听理解与逆向工程 (reference-video)

当用户提供一个参考视频文件（或视频片段），要求“分析这条视频”、“复刻这条视频”或“借鉴其节奏与视听形式”时使用本 Skill。

目标是产出时间码级别的视听语义解读报告：`ANALYSIS.md`（宏观叙事与系统模型）与 `TIMELINE.md`（逐秒时空对位记录）。

---

## 1. 可用底层工具箱 (clone-any-video CLI)

执行本 Skill 时，使用当前项目提供的命令行工具：`./scripts/clone-any-video`（先定位当前仓库根目录）：

- **`probe`**：探测时长、尺寸分辨率、帧率与音频流
  ```bash
  ./scripts/clone-any-video probe <video.mp4> --json
  ```
- **`transcribe`**：生成含词级时间戳的对齐转录（基于本地 Whisper 服务）
  ```bash
  ./scripts/clone-any-video transcribe <video.mp4> --local --language en --to <output_dir/transcript.json>
  ```
- **`boundaries`**：基于 32x32 RGB24 L1 算法定位相邻帧视觉突变候选点
  ```bash
  ./scripts/clone-any-video boundaries <video.mp4> --rate 12 --threshold 0.1 --json
  ```
- **`tile`**：将关键帧按网格排版为全景视盘图片，并在底部附带时间码与台词单词
  ```bash
  ./scripts/clone-any-video tile <video.mp4> --transcript <transcript.json> --frames 6 --columns 3 --to <output_dir/overview_grid.jpg>
  ```
- **`frames`**：提取离散或等间距的高清单帧
  ```bash
  ./scripts/clone-any-video frames <video.mp4> --at <s,s,...> --to <output_dir/frames>
  ```
- **`cut`**：截取局部代表性音视频片段
  ```bash
  ./scripts/clone-any-video cut <video.mp4> --start <start_s> --end <end_s> --output <clip.mp4>
  ```

<!-- 待后续扩展工具：
- fetch: 当用户提供网页视频链接 (如 Bilibili/YouTube/TikTok) 时，调用 yt-dlp 自动抓取并保存本地 MP4
- snapshot: 从无头 Chromium 浏览器捕获渲染画布快照，用于与原片进行像素级视觉对比
-->

---

## 2. 核心执行工作流

当用户要求分析某个视频时，请按顺序执行以下 4 个阶段：

### 阶段一：建立物理与听觉骨架
1. 执行 `./scripts/clone-any-video probe <video.mp4> --json` 获取时长、宽高、帧率；
2. 若存在音频，执行 `./scripts/clone-any-video transcribe <video.mp4> --local --to <output_dir/transcript.json>` 获取词级时间戳。

### 阶段二：视觉突变与分镜初筛
1. 执行 `./scripts/clone-any-video boundaries <video.mp4> --rate 12 --threshold 0.1 --json` 扫描分镜候选点；
2. 结合 `transcript.json`，检查视觉跳点是否与台词停顿（气口）重合，寻找“声画对位点”。

### 阶段三：全景视盘观察
1. 执行 `./scripts/clone-any-video tile <video.mp4> --transcript <transcript.json> --frames 6 --columns 3 --to <output_dir/overview_grid.jpg>` 生成全景视盘；
2. 观察视盘画面的机位构图、主体运动轨迹与信息分布。

### 阶段四：沉淀结构化逆向档案
在目标输出目录中生成两份专业报告：
1. **`ANALYSIS.md`**：记录宏观叙事模型、Hook 破局方式、各层视听系统贡献与复刻建议；
2. **`TIMELINE.md`**：按时段切片记录各阶段的台词、画面动作、气口停顿与微观词级时间戳。

---

## 3. 输出文件规范

### 3.1 `ANALYSIS.md` 模板
```markdown
# 原片宏观解读报告

## 1. 物理特征
- 时长：X.X 秒 | 分辨率：WxH | 帧率：XX fps

## 2. 叙事与节奏模型
- 核心叙事目标：[该视频的核心传播意图]
- 言语骨架：[全篇台词摘要]
- 节奏模型：[语速、停顿与转折]

## 3. 视听系统拆解
- 画面层 (Visuals)：[主体表现、机位调度]
- 听觉层 (Audio)：[音色、语调、气口]
- 字幕层 (Captions)：[展现形式与视觉重心]

## 4. 迁移与复刻建议
- [如何保留节奏和功能，重构形式]
```

### 3.2 `TIMELINE.md` 模板
```markdown
# 逐秒时空对位记录

## 时段切片
- `0.00s – X.XXs`：[阶段一主题与声画对位]
- `X.XXs – Y.YYs`：[阶段二主题与声画对位]

## 微观词级证据链
- `[start - end]`：单词与时间戳明细
```
