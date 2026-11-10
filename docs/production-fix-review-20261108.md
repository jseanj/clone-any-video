# origin → The Ordinary：生产问题修复与验证

基线：clone-any-video `b8fb378`；Clone Any Video 参照 `2d31e887`。修复集中在第 16 章的集成实现、CLI、Skill 和生产文档，保留前置章节的教学递进。本轮未重新调用收费生图/生视频/TTS；复用上一轮真实素材，实际运行 medium、Chrome、FFmpeg、FlowKit 项目接口。

## 修复矩阵

| 记录 | 本轮处理 | 验证与边界 |
| --- | --- | --- |
| R01 Skill/Mock | 修正本机 audiovisual-craft 的真实调用顺序、当前 CLI 名称与生产参数；同步两个主 Skill。自动 Moment 不再偷偷生成 Mock，支持已准备音效/显式 SFX plan。 | 全局路径是仓库 Skill 的软链接，修改即时生效；3 个 Skill 通过 validator。教学须显式 `--mock-sfx`，来源仍传播。 |
| R02 工程/缓存 | `build --prepared-project` 直接读取并校验精确清单、当前 Spoken 和媒体，不拿缓存替换。合成尊重明确背景源和组件路径。 | 实际五镜头工程经 build 输出 464 帧；CLI 回归验证缓存字节不变、错误 Spoken 被阻断。 |
| R03 网页抽取 | 不修改媒体引擎；采用官方正文/图片交叉核对的资料策略。 | 属于外部抽取错误；本轮没有重新抓取并声称解决上游抽取器。 |
| R04 Python 依赖 | 新增生产 CLI 回归与 daemon 验证使用标准库。 | 非仓库媒体 bug。FlowKit 单测按其 requirements-dev 安装测试依赖，不向生产抓取脚本添加 requests。 |
| R05 商品参考 | image generate/batch 支持重复 reference-image / reference-media-id，实际上传引用、验证图片/MIME/UUID。 | 离线 HTTP 契约验证上传内容、引用列表；不把参考图当成包装准确的自动证明。 |
| R06 配音实测/编辑 | `media prepare-audio` 另存编辑后的音频，记录原始内容指纹、参数、实测时长与对齐失效。支持明确去静音、tempo、响度处理。 | 源文件保留；TTS 仍不能保证按估算表演。剪去静音后不能用全局 tempo 复用旧时间戳。 |
| R07 FlowKit 重复身份 | FlowKit 项目创建先查已存在 ID；无新增实体且元数据一致则返回已有项目，冲突返回 409。 | 5 项 FlowKit 单测通过；重启服务后实测 200/409，确认原项目未变。 |
| R08 ASR/言语证据 | 引擎证据、实际版本/模型与 forced_alignment=false 进入转录；无置信度不补造。`media revise-transcript` 显式校订文本、保留窗口/父证据，纯变速可审计投影。 | 真实 medium 识别本轮 identity；词块内分配仍是估算。未引入 WhisperX 声学强制对齐。缺词仍阻断。 |
| R09 音色身份 | voice batch 增加 voice / language，结果保存实际 preset、语言、时长和预算比较。 | MiMo 单段/批量 HTTP 契约测试检查 Mia/冰糖 preset 和请求内容；无新真实 TTS 请求。 |
| R10 Flow 项目贯通 | 图/视频共享 `.flowkit-project.json`，显式请求与绑定冲突时在提交前失败；初次绑定原子落盘。 | 模拟浏览器切项目后视频仍用原绑定；显式冲突不上传、不生图。维持 Omni Flash 360p。 |
| R11 标点与空格 | Script DisplayToken 保存原 separator；字幕与 spoken 投影正确连接独立标点。 | 实际 7 条字幕变为 `The Ordinary.` / `Multi-Peptide + HA Serum.`；Unit 身份不变。 |
| R12 外部图片回执 | `media import-flowkit-image` 从已完成回执直接下载并正规化 PNG，保留 media/project/URL/回执指纹。 | 本地 HTTP 验证 PNG 魔数和来源身份；显式导入不等于密码学认证。 |
| R13 品牌组件/捕获 | 广告版式由 Agent authored HTML 提供；Unix 捕获复用隔离 Chrome 页面、CDP pipe、字体/图片就绪及显式 renderFrame。 | 原品牌卡 87 帧实测约 6.47 秒；本轮 464 帧字幕也实际捕获。未新增一个宣称适合所有商品的固定模板。 |
| R14 字幕交付 | burn-in / soft / none 显式区分；无 libass 不再默降。新增完整 Chrome 字幕层与 cue 覆盖来源，review 检查实际生产策略。 | 当前无 libass 环境，默认错误和显式 mov_text 软字幕都通过回归；本轮 Chrome 硬字幕抽帧可见，原始 ASS 不重复叠加。 |
| R15 实际闪避 | 按实际幅度活动检测计算 ducking，短间隙的坡道压缩，不再只用整段配音窗口。 | 真实素材 12 个活动窗、31 个包络点；合成声音间隙回归验证恢复增益，旧物理增益测试通过。过短间隙仍可能合并，不声称完成语义/听感审查。 |
| R16 技术/创意 | passed 与 ready_for_delivery 分离；后者要求 production 及绑定成片内容的商品、口播字幕、连续性、水印接受证据。 | 修改视频/审片附件会使证据失效；未审查本片保持 ready=false。 |
| R17 水印接受 | 水印成为必需创意验收项，显式 pass/fail/unverified 和依据。 | 实际水印保留；本轮未去水印，也未代用户确认可投放。 |
| R18 响度 | review 增加真实 R128 LUFS / true peak / LRA；prepare-audio 支持明确母带目标。 | 新成片 -16.0 LUFS、真峰值 -1.4 dBFS、LRA 2.9 LU。当前提示范围不是所有投放平台的统一标准。 |
| R19 剪辑连续性 | 原生 edit plan 明确源/目标帧、zoom、continuity_group 和 cut_intent；每源先规范化，再 1:1 裁剪。导出实际编码切点 ±3 帧。 | 复用五个源，5 镜头、zoom=1；回退/尺度阶跃在声明连续组中被阻断。有意跳切仍需审片，不自动消除所有模型动作畸变。 |
| R20 medium/版本 | daemon 默认 medium / CPU int8；health 返回实际 faster-whisper 版本；加载失败 HTTP 503，生产无模拟转录。 | 当前 8765 实测 medium / 1.2.1；完整词时间戳含 engine 证据。未添加开机自启。 |
| R21 帧精度 | 整数半开裁剪、统一叠层前帧时基、约束总帧数、音频裁补到相同时钟、输出帧数复核。 | 实际 464 帧；像素回归验证 56/71/116 帧片段切点和叠层首尾，较长音频不会拖长容器。 |

## 实际生产验证

输出目录：`output/remediation-20261108/`（媒体和运行证据留在本地，Git 忽略）。

- `ordinary-ad.fixed.mp4`：640×360，30 fps，464 帧，约 15.4667 秒；全文件解码通过。
- `project.mixed.json`：实际执行清单，5 个镜头；`edit-plan.json` 保留源映射。
- `caption-schedule.json`：实际转录证据投影的 7 条字幕；`assets/video_components/captions.mp4` 是完整真实 Chrome RGBA 层。
- `ordinary-ad.fixed.review.json` / `technical-review.json`：production，技术通过，91.67 分，ready_for_delivery=false。
- `boundaries/boundaries.json`：与本次成片相同 artifact_digest，真实切点 116、245、302、377 前后各 3 帧。
- `assets/audio/master_mix.unmastered.wav.provenance.json`：实际活动窗和包络；`mastering.log` 保留母带处理参数与实测时长。
- `identity.medium.json` / `whisper-health.json`：实际 ASR 引擎事实，未替换历史转录。
- `flowkit-live-contract.json`：真实项目重复/冲突 HTTP 验证，未生成新媒体。
- `checks/` / `validation-summary.json`：434 项工作区 Rust 测试通过；第 16 章最终 69 项通过；3 组离线 CLI/Provider 契约通过；FlowKit 5 项通过；mdbook build 通过。

复现入口见 [正式生产指南](production-remediation.md)。运行 `cargo test --workspace --offline`，构建第 16 章后执行 `scripts/test_media_providers.py`、`scripts/test_pipeline_contracts.py`、`scripts/test_production_remediation.py`。本轮最后针对非线性音频编辑新增了 CLI 拒绝测试。

## 本轮修复中额外发现的事实

1. concat 改变时基后，即使背景总帧数正确，叠层还可能晚出现一帧。通过实际编码像素测试发现，现已在叠层前统一时基。
2. 老的准备工程 outro payload 仍含已删除的旧台词。realign 现在同步当前 Spoken；prepared build 会拒绝沿用旧 Spoken，不能仅重写标签掩盖音频身份。
3. FlowKit 旧进程的 SIGTERM 被自定义处理器用于停止 worker，HTTP 进程并未退出，首次替代进程遇到端口占用。确认空闲后停止精确旧进程并重新启动，恢复原默认 Flow 项目，扩展重连成功。未扩展本次修改去重写 FlowKit 整套进程管理。当前服务日志在输出目录 `flowkit-restart.log`，`.agent/server.pid` 已更新。
4. 媒体来源、结构检查和技术分数都不提供生成模型的完美包装/动作保证。本片生成器标记仍可见；正式投放还需完成具体内容验收。

## Clone Any Video 参考的实际机制

参照本地 `2d31e887` 中 prepared voice、参考材料职责、源/目标整数帧映射、Segment ≠ Shot、实际切点前/中/后复核及 repair owning fact。实现位置见 [生产指南的 Clone Any Video 对照](production-remediation.md#clone-any-video-对照)。没有假定 Clone Any Video 提供自动消除全部视觉跳动或自动证明广告商品准确的开关。

## 提交前第二轮复查

追加修复以下实际缺陷，相关测试纳入最终提交：

- 自定义 `mix render --out-mix` 没有传给合成器：新增可选 `ProjectManifest.master_audio`，混音写入明确路径，prepared build 的依赖记录检查实际选择的母带，显式缺失不会改用默认母带。
- 缺失必需配音会被跳过，显式缺失音乐会换成 fixtures：现在在输出前明确失败，并校验实测配音没有超出工程时钟。
- Voice/SFX 目标结束采样与淡入淡出未真正执行：修复实际滤镜，用长源音频放入短窗口的物理 RMS 回归验证无越界声音，BGM 也读取声明的淡入淡出长度。
- 字幕 ID 没变时，旧文字/旧窗口仍可通过技术项：现在绑定文字、实际可见半开帧窗和叠层位置；prepared build 还比对当前 Author 展示文本。浮点边界按播放器 `frame / fps` 的实际可见性判定，避免简单 ceil 多算一帧。
- 同名创意验收项可互相矛盾：重复 name 直接拒绝。
- edit 输出可能碰到已接受的源视频：在任何规范化/写入前阻断覆盖。规范化保持原纵横比并补边，不再无意拉伸人物/商品。
- 工程摘要对中文字节截断可能 panic：改为按 Unicode 字符截断。

最终真实复跑输出为 `ordinary-ad.reviewed.mp4`、`project.reviewed.json`、`ordinary-ad.reviewed.review.json`。上一轮 fixed 版本保留为历史证据。本轮依旧没有重新调用生成模型。

提交前最终验证：工作区 **438** 项 Rust 测试通过，第 16 章 **73** 项通过；3 组 Provider/CLI 契约、FlowKit 5 项测试、mdbook build 和 Skill 校验均通过。真实成片技术通过、464 帧，创意交付状态仍为未就绪。同 cue ID 的过期文字已实际验证在 review 和合成阶段都被阻断。FlowKit 项目身份修复独立提交为 `e175756`；其原有扩展改动未纳入本次提交。
