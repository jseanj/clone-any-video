---
name: video-transformation
description: 原片视听转译与剧本时间线重构 Skill。当用户提供已分析的原片档案（ANALYSIS.md/TIMELINE.md）并提出新视频创作需求（Brief）时使用，负责提炼叙事核心、重写目标 SVML 语义剧本、求解动态弹性时间线并生成完整的工程项目蓝图 (project.json)。
---

# 原片视听转译与剧本时间线重构 (video-transformation)

当用户提供一条已分析的原片档案（如 `ANALYSIS.md` 与 `TIMELINE.md`），并提出明确的新视频创作需求（例如：“把这个商业广告复刻成宣传开源项目 clone-any-video 的视频”）时，使用本 Skill。

本 Skill 的核心使命是：**严格遵循“保留功能，重构形式（Preserve the role, then recreate the form）”军规，借用原片已被验证的黄金叙事节奏与时序骨架，为用户的新产品编写全新的语义剧本、求解动态时间线并输出完整的工程施工清单（project.json）**。

---

## 1. 核心指导军规 (Core Directing Principles)

1. **保留功能，重构形式 (Preserve the role, then recreate the form)**：
   - 关注原片每个视听动作在时间中为了达到什么心理目的，而不是机械拷贝像素；
   - 原片的“五合一”是在解答复杂度并展现产品实力；新视频必须设计一个同样能解答复杂度的视觉等式；
   - 原片的“左右比较”是在建立可信度；新视频必须提供一个对应的对比论证。
2. **剧本是唯一的言语权威 (Script is sole verbal authority)**：
   - 剧本内严禁硬编码绝对物理秒数，使用纯语义时间与结构标签（`<hook>`, `<identity>`, `<proof>`, `<outro>`）；
   - 必须使用双重文本 `<Display | Spoken>` 解决屏幕展示与 TTS 配音发音的冲突；
   - 使用 `||` 显式标明字幕断点，使用 `@{moment!}` 标注高潮事件；
   - 详细语法规则详见附录手册：[`references/svml-syntax.md`](references/svml-syntax.md)。
3. **字数严格受物理预算约束 (Word Budget Enforcement)**：
   - 新剧本各段落的目标时长必须继承自原片 `TIMELINE.md` 的实测数据；
   - 依据英文 2.6 词/秒、中文 3.8 字/秒公式严格折算字数预算，严禁超长。

---

## 2. 可用底层工具箱 (clone-any-video CLI)

执行本 Skill 时，使用当前项目提供的命令行工具：`./scripts/clone-any-video`（先定位当前仓库根目录）：

- **`script --estimate`**：校验并预估 SVML 剧本时长与各段字数
  ```bash
  ./scripts/clone-any-video script <script.svml> --estimate
  ```
- **`script --speech`**：导出纯净发音文本流（供 TTS 配音使用）
  ```bash
  ./scripts/clone-any-video script <script.svml> --speech
  ```
- **`dynamic-timeline`**：求解基于 DAG 的动态弹性时间线，输出结构化时间数据
  ```bash
  ./scripts/clone-any-video dynamic-timeline <script.svml> [--speech-extents <extents.json>] --gap 0.3 --json --to <timeline.json>
  ```
- **`project`**：绑定剧本与时间线，生成工程总蓝图与资产生产任务清单
  ```bash
  ./scripts/clone-any-video project <script.svml> --timeline <timeline.json> --width <W> --height <H> --validate --json --to <project.json>
  ```

---

## 3. 核心执行工作流 (5 阶段)

### 阶段一：阅读原片档案，确立转译方案 (Treatment)
1. 仔细阅读原片分析报告 `ANALYSIS.md` 与逐秒对位记录 `TIMELINE.md`；
2. 提取原片的 4~8 个核心分镜阶段及其时长、词数与视听功能；
3. 将用户的新产品/新需求逐一映射到原片的功能模块上，撰写转译策划方案 `TREATMENT.md`。按原片实际结构选择阶段数；单个连续镜头不强凑 4–8 个生成请求。

### 阶段二：撰写 SVML 目标剧本 (`target_script.svml`)
1. 依据原片词数预算，为每个段落编写台词；
2. 插入双重文本：如 `<five | 5>`、`<Rust | rust>`、`<beats | B E A T S>{emphasis}`；
3. 插入字幕换行断点 `||` 与关键特效/音效锚点 `@{question!}`、`@{compare!}`。

### 阶段三：编译校验与时序预估
1. 运行 `./scripts/clone-any-video script target_script.svml --estimate`；
2. 检查输出的预估时长是否与原片预算吻合；若有偏差，微调文案直至严密吻合。

### 阶段四：求解动态弹性时间线 (`target_timeline.json`)
1. 运行 `./scripts/clone-any-video dynamic-timeline target_script.svml --gap 0.3 --json --to <output_dir>/target_timeline.json`；
2. 若用户提供了实际配音时长或预设 extents，传入 `--speech-extents` 进行精确贴合。

### 阶段五：生成工程项目总图纸 (`project.json`)
1. 运行 `./scripts/clone-any-video project target_script.svml --timeline <output_dir>/target_timeline.json --width <W> --height <H> --validate --json --to <output_dir>/project.json`；可见同源口播增加 `--speaking-video`，不加音乐增加 `--no-bgm`。
2. 验证输出的 0 errors / 0 warnings，确认分镜清单、多轨分配与资产任务队列均已就绪。

---

## 4. 输出文件交付规范

在目标输出目录中（例如 `output/target_project/`），必须完整交付以下文件：

```text
<output_dir>/
├── TREATMENT.md           # 转译策划书：记录原片与新片的功能映射与导演分镜设计
├── target_script.svml     # 严格符合 SVML 规范的新视频语义剧本
├── target_timeline.json   # 动态弹性时间线求解成果 (ResolvedTimeline)
├── project.json           # 机器可读的工程总蓝图 (clone-any-video.project@1)
└── BLUEPRINT.txt          # 格式化的 ASCII 生产总图纸与任务清单
```

初始 project.json 必须由此原生命令生成，不得手写布局或复制其他案例替代。Author/Run 的 project-timing.layout 必须读取它，按实际表演求解后续工程。真实运行需要遵循主 clone-any-video skill 的 WORKFLOW_RUN.json / WORKFLOW_REVIEW.md / workflow-audit.json 复盘契约。
