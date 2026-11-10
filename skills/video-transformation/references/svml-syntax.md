# SVML (Semantic Video Markup Language) 语法规范手册

> **定位**：多模态视频生产语义标记语言（SVML）权威语法规范  
> **所有权**：本规范定义了大模型在撰写 `.svml` 剧本时必须遵守的全部节点、标记与属性规则。

---

## 1. 核心设计原则

1. **纯语义，零绝对时间戳**：剧本严禁硬编码任何绝对物理秒数（如 `[00:03]`）。绝对时间由底层发音模型（TTS）测得的区间与动态时间线图求值产生。
2. **双重文本（Dual Text）**：解决“屏幕显示标准拼写”与“语音模型发音注音”之间的冲突。
3. **不可分割的对齐单元（Alignment Unit）**：屏幕上的一个词或词组与其对应的发音构成最小对齐原子。

---

## 2. 完整语法要素对照表

| 语法结构 | 标记形式 | 作用与含义 |
| :--- | :--- | :--- |
| **根节点** | `<script id="..."> ... </script>` | 声明剧本文档边界与工程标识 ID |
| **段落 (Segment)** | `<hook>...</hook>`, `<identity>...</identity>` | 叙事功能段落；所有台词必须包裹在段落内 |
| **空镜头段落** | `<pause/>` | 无对白的纯视觉镜头段落（具备时长边界但无词项） |
| **角色标记 (Role)** | `<HOST>`, `<GUEST>`, `<NARRATOR>` | 声明当前段落或对白台词的讲话人角色 |
| **双重文本 (Dual Text)** | `<显示文本 \| 发音注音>` | 核心语法：屏幕展示左侧拼写，TTS 发音使用右侧文本 |
| **等字绑定 (Grouping)** | `<专有名词\|>` | 左右文本一致，作为不可分割的字幕组合显示 |
| **字幕断点 (Caption Cue)**| `\|\|` | 显式声明字幕翻页/换行时机（与段落彻底解耦） |
| **单词属性 (Attribute)** | `word{emphasis}` 或 `word{color=gold}` | 为特定单词附加视觉高亮、强调或样式修饰 |
| **瞬间事件点 (Moment)** | `@{question!}`, `@{compare!}` | 声明零宽度的瞬间事件点，供音效（SFX）和动效对齐 |
| **语义区间 (Selection)** | `@{proof} ... @{/proof}` | 圈定一段台词的跨度区间，供 B-roll 画面或动态遮罩对齐 |

---

## 3. 详细语法规则与范例

### 3.1 根节点与段落 (Script & Segments)
```svml
<script id="my_promo_video">
  <hook>
    <!-- 开头第一阶段台词 -->
  </hook>
  <identity>
    <!-- 品牌或产品展示台词 -->
  </identity>
  <proof>
    <!-- 论证与对比台词 -->
  </proof>
  <outro>
    <!-- 结尾与号召台词 -->
  </outro>
</script>
```
- 段落名称推荐使用小写英文字母、数字与下划线，如 `hook`、`demo`、`solution`、`cta`；
- 所有口播台词必须被包含在某个段落内部，严禁裸露在 `<script>` 顶层。

### 3.2 角色标记 (Role Cues)
```svml
<hook>
  <HOST> 欢迎收看本期视频！
  <GUEST> 听说今天有重大开源发布？
</hook>
```
- `<HOST>`、`<GUEST>` 等大写标签标明发音人身份；
- 换人说话时直接声明新角色，系统自动生成对应的语音轨道切片与对齐气口。

### 3.3 双重文本 (Dual Text) —— 解决发音难题
在多模态视频中，屏幕字幕与配音发音经常产生冲突：
- 屏幕想要正规缩写：`API`、`SQL`、`Rust`、`OLAY`、`5`
- 语音引擎需要注音：`A P I`、`sequel`、`润斯特`、`欧蕾`、`five`

**语法范例**：
```svml
<HOST> 想要拥有 <five | 5> 种核心能力？
<HOST> 试试这款纯 <Rust | rust> 引擎！
<HOST> 它的性能超越了传统 <API | A P I>{emphasis}。
```
- `<five | 5>`：屏幕字幕显示为 `five`（或反之），TTS 发音为 `5`；
- `<Rust | rust>`：字幕规范首字母大写 `Rust`，发音传入全小写音标 `rust`；
- `<API | A P I>`：屏幕显示连续大写 `API`，发音按字母间隔发音 `A P I`。

### 3.4 字幕断点 (`||`)
传统字幕引擎常按标点符号或固定字数机械切行，导致生硬断句。SVML 使用 `||` 由编剧指定字幕气口断点：
```svml
<HOST> 改变的是具体的发音秒数 || 但关键时序关系完全保持不变！
```
- `||` 左侧为 Caption Cue 1，右侧为 Caption Cue 2；两者在屏幕上先后呈现，避免一行字幕过长。

### 3.5 瞬间事件点与区间选择 (`Moment` & `Selection`)
用于在台词流中打下“时空图钉”，供下游画面、特效和音效精准吸附：
```svml
<proof>
  <HOST> @{compare!} 请看这里的左右速度对比！
  <HOST> @{evidence} 测试结果显示性能提升了整整三倍！ @{/evidence}
</proof>
```
- `@{compare!}`：末尾带 `!` 表示瞬时点（Instant），在此毫秒触发转场音效或光圈爆发；
- `@{evidence} ... @{/evidence}`：成对标签表示区间（Selection / Sub-Window），在此期间屏幕上方悬浮高亮性能徽章。

---

## 4. 标准示范模板

```svml
<script id="clone_any_video_promo">
  <hook>
    <HOST> @{question!} Want the power of <five | 5> video tools in one?
  </hook>

  <identity>
    <HOST> <clone-any-video | clone any video>! Pure <Rust | rust> engine.
  </identity>

  <proof>
    <HOST> @{compare!} For visible speed || that <beats | B E A T S>{emphasis} traditional pipelines!
  </proof>

  <outro>
    <HOST> A super workflow for your best AI videos yet. <GitHub | git hub>!
  </outro>
</script>
```

---

## 5. 校验工具

编剧写完 SVML 后，必须运行本项目提供的校验工具进行编译检查：
```bash
# 检查语法、词数预算与各段估算时长
./scripts/clone-any-video script <your_script.svml> --estimate

# 检查纯发音流输出
./scripts/clone-any-video script <your_script.svml> --speech

# 检查字幕切分流
./scripts/clone-any-video script <your_script.svml> --caption
```
