mod artifacts;
#[cfg(unix)]
mod chrome_session;
mod delivery;
mod edit;
mod flowkit;
mod force_alignment;
mod performance;
mod production;
mod production_graph;
mod sampling;
mod semantic_clock;
mod speech_alignment;
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

mod audio;
mod boundaries;
mod cut;
mod image_director;
mod probe;
mod project;
mod script;
mod tiles;
mod timeline;
mod transcribe;
mod video_director;
mod voice_director;

use boundaries::detect_visual_boundaries;
use cut::{cut_media, CutOptions};
use image_director::{
    assemble_prompt, audit_prompt, compute_prompt_hash, ImageGenerationRequest, ImagePromptSpec,
    MockImageGenerator, OpenAiImageGenerator, DEFAULT_PHONE_UGC_CAPTURE,
};
use probe::probe_media;
use project::{
    create_project_manifest, render_project_summary, validate_project, AssetTaskType,
    ProjectManifest,
};
use script::parse_script;
use tiles::{
    create_tile, create_tiles, extract_frames, FramesOptions, TileOptions, TilesOptions,
    DEFAULT_TILE_CELL_WIDTH, DEFAULT_TILE_COLUMNS, DEFAULT_TILE_ROWS,
};
use timeline::{
    project_script_to_timeline, project_script_to_timeline_with_alignments, render_ascii_timeline,
    ResolvedTimeline,
};
use transcribe::{transcribe_media, TranscribeOptions, TranscriptDocument};
use video_director::{
    assemble_video_prompt, audit_video_prompt, compute_video_prompt_hash, CameraMotion,
    MockVideoGenerator, RemoteVideoGenerator, VideoGenRelationship, VideoGenerationRequest,
    VideoPromptSpec,
};
mod sound_mix;
use sound_mix::{
    attach_sfx_to_moments, compile_audio_program_plan, render_master_mix, synthesize_mock_sfx,
    update_manifest_with_mixed_tracks, AudioClip, AudioTrackType, DuckingConfig,
    CANONICAL_SAMPLE_RATE,
};
mod caption_engine;
use caption_engine::{
    apply_head_tracking, build_caption_document_from_script,
    build_caption_timing_from_speech_intervals, build_caption_timing_from_transcript,
    export_to_ass, export_to_html_overlay, export_to_webvtt, query_caption_state_at,
    schedule_caption, CaptionDocument, CaptionRecipe, CaptionSchedule, FaceObservation,
    HeadTrackingConfig,
};
mod component_designer;
use component_designer::{
    auto_design_components_for_project, render_component_to_html,
    update_manifest_with_visual_components, ComponentAnimation, ComponentArchetype, ComponentTheme,
    MotionType, SpatialBounds, VisualComponentSpec,
};
mod browser_capture;
use browser_capture::{
    batch_capture_project_components, capture_frame_sequence, capture_single_frame,
    detect_chrome_executable, render_html_to_mp4, resolve_browser_provider, CaptureConfig,
};
mod compositor;
use compositor::{
    compile_composite_filtergraph, compile_project_to_composite_plan, render_composition,
};
mod video_agent;
use video_agent::{
    review_project_and_master, run_autonomous_video_agent, AgentOptions, ReviewConfig, ReviewStatus,
};
use voice_director::{
    audit_voice_spec, collect_speech_extents, realign_project_timeline, MockVoiceGenerator,
    RemoteVoiceGenerator, VoiceGenerationRequest, VoicePromptSpec,
};

#[derive(Parser, Debug)]
#[command(name = "clone-any-video")]
#[command(
    about = "Clone Any Video: Multi-Modal Automated Video Production Agent and Deterministic Media Pipeline",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Measured media preparation, explicit evidence and continuity edits
    Media {
        #[command(subcommand)]
        sub: production::MediaCommands,
    },
    /// 探测并解析音视频媒体元数据
    Probe {
        input: PathBuf,
        #[arg(long)]
        json: bool,
    },

    /// 截取音视频片段
    Cut {
        input: PathBuf,
        #[arg(long)]
        start: f64,
        #[arg(long)]
        end: f64,
        #[arg(long, short)]
        output: PathBuf,
        #[arg(long)]
        accurate: bool,
    },

    /// 语音转录与 ASR 词级时间戳（非声学强制对齐）
    Transcribe {
        input: PathBuf,
        #[arg(long)]
        to: PathBuf,
        #[arg(long, default_value = "en")]
        language: String,
        #[arg(long)]
        api_key: Option<String>,
        #[arg(long)]
        api_url: Option<String>,
        #[arg(long)]
        local: bool,
        #[arg(long)]
        mock: bool,
        #[arg(long)]
        json: bool,
    },

    /// 机械性相邻帧视觉变化检测
    Boundaries {
        input: PathBuf,
        #[arg(long)]
        rate: Option<f64>,
        #[arg(long)]
        threshold: Option<f64>,
        #[arg(long)]
        json: bool,
    },

    /// 提取选定时段或离散时间戳的一组单帧图像
    Frames {
        input: PathBuf,
        #[arg(long)]
        start: Option<f64>,
        #[arg(long)]
        end: Option<f64>,
        #[arg(long)]
        at: Option<String>,
        #[arg(long)]
        every: Option<f64>,
        #[arg(long)]
        frames: Option<usize>,
        #[arg(long)]
        label_time: bool,
        #[arg(long)]
        transcript: Option<PathBuf>,
        #[arg(long)]
        to: PathBuf,
        #[arg(long)]
        json: bool,
    },

    /// 将若干采样帧拼接为单张全景视盘图片 (Contact Sheet Grid)
    Tile {
        input: PathBuf,
        #[arg(long)]
        start: Option<f64>,
        #[arg(long)]
        end: Option<f64>,
        #[arg(long)]
        at: Option<String>,
        #[arg(long)]
        every: Option<f64>,
        #[arg(long)]
        frames: Option<usize>,
        #[arg(long)]
        cell: Option<u32>,
        #[arg(long)]
        columns: Option<u32>,
        #[arg(long)]
        transcript: Option<PathBuf>,
        #[arg(long)]
        to: PathBuf,
        #[arg(long)]
        json: bool,
    },

    /// 多页全景视盘矩阵：长视频分页输出一组视盘
    Tiles {
        input: PathBuf,
        #[arg(long)]
        start: Option<f64>,
        #[arg(long)]
        end: Option<f64>,
        #[arg(long)]
        at: Option<String>,
        #[arg(long)]
        every: Option<f64>,
        #[arg(long)]
        frames: Option<usize>,
        #[arg(long)]
        cell: Option<u32>,
        #[arg(long)]
        columns: Option<u32>,
        #[arg(long)]
        rows: Option<u32>,
        #[arg(long)]
        transcript: Option<PathBuf>,
        #[arg(long)]
        to: PathBuf,
        #[arg(long)]
        json: bool,
    },

    /// SVML 语义剧本解析器与注音转译 (script-parser)
    #[command(alias = "script-parser")]
    Script {
        /// 输入 SVML 剧本文件路径
        input: PathBuf,

        /// 输出纯发音文本流 (用于直通 TTS 合成器)
        #[arg(long)]
        speech: bool,

        /// 输出带角色对白流 (用于演员剧本/LLM 审查)
        #[arg(long)]
        dialogue: bool,

        /// 输出字幕分句 Cue 清单 (用于字幕渲染引擎)
        #[arg(long)]
        caption: bool,

        /// 输出详细时长估算与统计指标
        #[arg(long)]
        estimate: bool,

        /// 将解析结果输出到指定文件
        #[arg(long)]
        to: Option<PathBuf>,

        /// 输出结构化 JSON 语法树 AST
        #[arg(long)]
        json: bool,
    },

    /// 解耦时间与局部投影 —— 动态弹性时间线求解器 (dynamic-timeline)
    #[command(alias = "timeline")]
    DynamicTimeline {
        /// 输入 SVML 剧本文件路径
        input: PathBuf,

        /// 帧率 (FPS, 默认 30.0)
        #[arg(long, default_value = "30.0")]
        fps: f64,

        /// 段落之间的自然呼吸间隔秒数 (Gap, 默认 0.3s)
        #[arg(long, default_value = "0.3")]
        gap: f64,

        /// 传入实际测得的各段落配音时长 JSON 文件 (用于弹性拉伸时间线)
        #[arg(long)]
        speech_extents: Option<PathBuf>,

        /// 传入转录或词级对齐 JSON 文件 (TranscriptDocument)，用于将 Moments/Selections 精准锚定到真实发音时间
        #[arg(long)]
        transcript: Option<PathBuf>,

        /// 将时间线解析成果写入指定文件
        #[arg(long)]
        to: Option<PathBuf>,

        /// 输出结构化 JSON 格式
        #[arg(long)]
        json: bool,

        /// 输出详细摘要表格
        #[arg(long)]
        summary: bool,
    },

    /// 项目声明与工程模型 —— 视频蓝图与资产任务生成器 (project-schema)
    #[command(alias = "project-schema")]
    Project {
        /// 输入 SVML 剧本文件路径
        script: PathBuf,

        /// 输入已解析的动态时间线 JSON 文件 (可选)
        #[arg(long)]
        timeline: Option<PathBuf>,

        /// 画面宽度 (默认 640，对齐参考视频)
        #[arg(long, default_value = "640")]
        width: u32,

        /// 画面高度 (默认 360，对齐参考视频)
        #[arg(long, default_value = "360")]
        height: u32,

        /// 项目标题 (默认使用 script id)
        #[arg(long)]
        title: Option<String>,

        /// Plan native speaking-video audio instead of independent TTS.
        #[arg(long)]
        speaking_video: bool,
        /// Omit the default music track when the brief calls for no added music.
        #[arg(long)]
        no_bgm: bool,

        /// 默认段落自然呼吸间隔秒数 (默认 0.3s)
        #[arg(long, default_value = "0.3")]
        gap: f64,

        /// 传入各段落配音时长 JSON 文件 (可选)
        #[arg(long)]
        speech_extents: Option<PathBuf>,

        /// 帧率 (FPS, 默认 30.0)
        #[arg(long, default_value = "30.0")]
        fps: f64,

        /// 将完整工程项目清单写入指定文件 (如 project.json)
        #[arg(long)]
        to: Option<PathBuf>,

        /// 输出结构化 JSON 格式
        #[arg(long)]
        json: bool,

        /// 执行工程约束校验
        #[arg(long)]
        validate: bool,
    },

    /// 四段式生图与视觉导演 —— 提示词组装、质量审计与生图适配器 (image-director)
    #[command(alias = "image-director")]
    Image {
        #[command(subcommand)]
        sub: ImageSubcommands,
    },

    /// 动态运镜与图生视频 —— 动作意图、运镜控制与视频合成适配器 (video-director)
    #[command(alias = "video-director")]
    Video {
        #[command(subcommand)]
        sub: VideoSubcommands,
    },

    /// A-roll 声音表演与二次对齐 —— 声音铸造、语音合成与动态拉伸 (voice-director)
    #[command(alias = "voice-director")]
    Voice {
        #[command(subcommand)]
        sub: VoiceSubcommands,
    },

    /// 声音工程与多轨混音 —— BGM 智能闪避、音效吸附与母带压制 (sound-mix)
    #[command(alias = "sound-mix")]
    Mix {
        #[command(subcommand)]
        sub: MixSubcommands,
    },

    /// 动态追踪与词级高亮字幕 —— 词级时间轴对齐、动态人头追踪与 Web-as-Video 字幕渲染 (caption-engine)
    #[command(alias = "caption-engine")]
    Caption {
        #[command(subcommand)]
        sub: CaptionSubcommands,
    },

    /// Web-as-Video 视觉组件化 —— HTML/CSS 视觉组件设计、模板实例化与轨道挂载 (component-designer)
    #[command(alias = "component-designer")]
    Component {
        #[command(subcommand)]
        sub: ComponentSubcommands,
    },

    /// 无头浏览器确定性捕获与视频编码 —— 将 HTML/CSS 动态组件转录为高保真 MP4 视频 (browser-capture)
    #[command(alias = "browser-capture")]
    Capture {
        #[command(subcommand)]
        sub: CaptureSubcommands,
    },

    /// Hyperframes 离线多层压制 —— 将背景、动态组件、字幕与母带音频一次性合成为广播级成片 (compositor)
    #[command(alias = "compositor")]
    Compose {
        #[command(subcommand)]
        sub: ComposeSubcommands,
    },

    /// 为经过确认的自备真实媒体登记来源（已知 Mock 不允许改标）
    AdmitMedia {
        #[arg(long)]
        asset: PathBuf,
    },

    /// 审查成片技术指标及媒体来源；语义质量由外部 Agent 验证
    #[command(alias = "quality-review")]
    Review {
        /// 项目蓝图清单 (如 project_with_rendered_components.json)
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        evidence: Option<PathBuf>,
        #[arg(long)]
        require_delivery: bool,

        /// 交付成片视频路径 (如 master.mp4)
        #[arg(long, short)]
        video: PathBuf,

        /// 验收要求；实际模式由文件内容绑定的来源记录决定
        #[arg(long, value_parser = ["production", "mock"])]
        mode: Option<String>,

        /// 以 JSON 格式输出完整质检报告
        #[arg(long)]
        json: bool,
    },

    /// 原片探测与剧本/组件/合成脚手架；真实素材由外部 Agent 和 Skill 预先生产
    Agent {
        /// 参考原片文件路径
        #[arg(long, short)]
        reference: PathBuf,

        /// 工作区工程目录 (默认 output/target_project)
        #[arg(long, default_value = "output/target_project")]
        project_dir: PathBuf,

        /// 最终交付成片导出路径 (默认 output/target_project/master.mp4)
        #[arg(long, default_value = "output/target_project/master.mp4")]
        output: PathBuf,

        /// 允许根据质检报告对归属事实进行自动修复 (Self-Healing)
        #[arg(long, default_value = "true")]
        auto_heal: bool,

        /// 离线 Mock 快速模式
        #[arg(long)]
        mock: bool,
    },

    /// Compile a production Run into an explained plan; no generation or capture
    Plan {
        source: PathBuf,
        #[arg(long)]
        mock: bool,
        #[arg(long)]
        to: Option<PathBuf>,
    },
    /// Inspect the saved facts and outputs of a production attempt
    Result { path: PathBuf },

    /// 执行生产 Run 任务图；旧 SVML 入口编译并合成已准备好的媒体
    #[command(alias = "run")]
    Build {
        /// 生产 Run（JSON/XML）或旧 .svml 剧本
        source: PathBuf,
        /// Select the measured authored project explicitly; bypass estimated blueprint caches
        #[arg(long)]
        prepared_project: Option<PathBuf>,

        /// 工作区工程目录 (默认基于源文件所在目录)
        #[arg(long)]
        project_dir: Option<PathBuf>,

        /// 视频导出路径；图构建默认只保存 Result，旧 SVML 默认 project_dir/master.mp4
        #[arg(long, short)]
        output: Option<PathBuf>,

        /// 允许根据质检报告对归属事实进行自动修复 (Self-Healing)
        #[arg(long, default_value = "true")]
        auto_heal: bool,

        /// 离线 Mock 快速模式
        #[arg(long)]
        mock: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum ImageSubcommands {
    /// 组装并格式化四段式生图提示词 (Capture, Person, Shot, Setting)
    Expand {
        /// 主体人物或物体 (Person / Subject)
        #[arg(long)]
        person: String,

        /// 景别、机位、交互动作与闲置状态 (Shot / Framing / Encounter)
        #[arg(long)]
        shot: String,

        /// 空间场所、细节与调色板 (Setting / Palette)
        #[arg(long)]
        setting: String,

        /// 摄影语言段 (Capture，默认采用 iPhone 实拍 UGC 描述)
        #[arg(long)]
        capture: Option<String>,

        /// 宽高比 (9:16, 16:9, 1:1, 4:5)
        #[arg(long, default_value = "9:16")]
        aspect_ratio: String,

        /// 分辨率档位 (1K, 2K, preview)
        #[arg(long, default_value = "1K")]
        resolution: String,

        /// 将组装好的提示词规格输出为 JSON
        #[arg(long)]
        json: bool,
    },

    /// 审计提示词合规性并排查反模式
    Audit {
        /// 提示词规格 JSON 文件，或使用参数组合
        spec_file: Option<PathBuf>,

        #[arg(long)]
        person: Option<String>,

        #[arg(long)]
        shot: Option<String>,

        #[arg(long)]
        setting: Option<String>,

        #[arg(long)]
        capture: Option<String>,
    },

    /// 执行单图生成 (支持本地 Mock 画布或远程 API)
    Generate {
        /// 提示词规格 JSON 文件 (若未提供则通过 --person/--shot/--setting 组装)
        #[arg(long)]
        spec: Option<PathBuf>,

        #[arg(long)]
        person: Option<String>,

        #[arg(long)]
        shot: Option<String>,

        #[arg(long)]
        setting: Option<String>,

        #[arg(long)]
        capture: Option<String>,

        #[arg(long, default_value = "9:16")]
        aspect_ratio: String,

        #[arg(long, default_value = "1K")]
        resolution: String,

        /// 分镜编号标识 (例如 shot_01)
        #[arg(long)]
        shot_id: Option<String>,

        /// 输出图像路径 (如 assets/visual/shot_01.png)
        #[arg(long, short)]
        output: PathBuf,
        /// Local product/character references; repeat for multiple images (FlowKit)
        #[arg(long = "reference-image")]
        reference_images: Vec<PathBuf>,
        #[arg(long = "reference-media-id")]
        reference_media_ids: Vec<String>,
        #[arg(long)]
        flow_project_id: Option<String>,

        /// 强制使用本地离线模拟生成器 (无需 API Key)
        #[arg(long)]
        mock: bool,

        /// 随机种子
        #[arg(long)]
        seed: Option<u64>,
    },

    /// 根据 project.json 蓝图批量生成分镜视觉素材
    Batch {
        /// project.json 文件路径
        #[arg(long)]
        project: PathBuf,

        /// 素材输出目录 (默认 assets/visual)
        #[arg(long, default_value = "assets/visual")]
        out_dir: PathBuf,
        /// Local product/character references; repeat for multiple images (FlowKit)
        #[arg(long = "reference-image")]
        reference_images: Vec<PathBuf>,
        #[arg(long = "reference-media-id")]
        reference_media_ids: Vec<String>,
        #[arg(long)]
        flow_project_id: Option<String>,

        /// 强制使用本地离线模拟生成器 (无需 API Key)
        #[arg(long)]
        mock: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum VideoSubcommands {
    /// 组装并格式化视频导演提示词与运镜参数
    Expand {
        /// 分镜编号
        #[arg(long, default_value = "shot_01")]
        shot_id: String,

        /// 审计时可选的首帧素材路径
        #[arg(long)]
        first_frame: Option<PathBuf>,

        /// 动作表演与动态意图 (Motion prompt)
        #[arg(long)]
        motion: String,

        /// 摄像机运镜行为 (push_in, pull_out, pan_left, pan_right, tilt_up, tilt_down, orbit, static_tripod, handheld)
        #[arg(long, default_value = "push_in")]
        camera: String,

        /// 视频时长秒数
        #[arg(long, default_value = "3.0")]
        duration: f64,

        /// 帧率 (FPS)
        #[arg(long, default_value = "30.0")]
        fps: f64,

        /// 画面宽度
        #[arg(long, default_value = "640")]
        width: u32,

        /// 画面高度
        #[arg(long, default_value = "360")]
        height: u32,

        /// 输出 JSON 格式
        #[arg(long)]
        json: bool,
    },

    /// 执行单个镜头的图生视频生成
    Generate {
        /// 首帧图片路径 (First frame image)
        #[arg(long)]
        first_frame: Option<PathBuf>,

        /// New speaking performance from Script; model chooses voice, no audio reference.
        #[arg(long, conflicts_with = "source_video")]
        dialogue_script: Option<PathBuf>,
        /// Select a single speaking segment from --dialogue-script.
        #[arg(long, requires = "dialogue_script")]
        segment: Option<String>,

        /// Edit actual source footage (FlowKit only), paired with --source-edit.
        #[arg(long, requires = "source_edit")]
        source_video: Option<PathBuf>,
        /// Source-edit JSON: start_frame/end_frame in Flow 30 fps time, optional upload bindings.
        #[arg(long, requires = "source_video")]
        source_edit: Option<PathBuf>,

        /// Optional last frame; requires first frame (FlowKit)
        #[arg(long)]
        last_frame: Option<PathBuf>,
        /// Ordered reference images; cannot combine with first/last frames (FlowKit)
        #[arg(long)]
        reference_image: Vec<PathBuf>,

        /// 动作表演与动态意图
        #[arg(long)]
        motion: String,

        /// 摄像机运镜行为
        #[arg(long, default_value = "push_in")]
        camera: String,

        /// 视频时长秒数
        #[arg(long, default_value = "3.0")]
        duration: f64,

        /// 帧率
        #[arg(long, default_value = "30.0")]
        fps: f64,

        /// 分镜编号
        #[arg(long, default_value = "shot_01")]
        shot_id: String,

        /// 画面宽度
        #[arg(long, default_value = "640")]
        width: u32,

        /// 画面高度
        #[arg(long, default_value = "360")]
        height: u32,

        /// 输出视频文件路径 (如 assets/video/shot_01.mp4)
        #[arg(long, short)]
        output: PathBuf,
        #[arg(long)]
        flow_project_id: Option<String>,

        /// 强制使用本地 FFmpeg 离线模拟合成器
        #[arg(long)]
        mock: bool,

        /// 随机种子
        #[arg(long)]
        seed: Option<u64>,
    },

    /// 根据 project.json 蓝图与视觉首帧资产，批量生成全片视频片段
    Batch {
        /// project.json 文件路径
        #[arg(long)]
        project: PathBuf,

        /// 首帧图像所在目录 (默认 assets/visual)
        #[arg(long, default_value = "assets/visual")]
        visual_dir: PathBuf,

        /// 视频素材输出目录 (默认 assets/video)
        #[arg(long, default_value = "assets/video")]
        out_dir: PathBuf,
        #[arg(long)]
        flow_project_id: Option<String>,

        /// 强制使用本地 FFmpeg 离线模拟合成器
        #[arg(long)]
        mock: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum VoiceSubcommands {
    /// 组装并格式化声音导演提示词与台词规范
    Expand {
        /// 剧本段落 ID
        #[arg(long)]
        segment_id: String,

        /// 说话角色标识 (Role Cue)
        #[arg(long, default_value = "HOST")]
        role: String,

        /// 发音台词文本 (.speech 投影)
        #[arg(long)]
        spoken: String,

        /// 角色音色与情感语调描述
        #[arg(long)]
        direction: String,

        /// 预估规划时长
        #[arg(long, default_value = "2.5")]
        duration: f64,

        /// 语种
        #[arg(long, default_value = "en")]
        language: String,

        /// 输出 JSON 格式
        #[arg(long)]
        json: bool,
    },

    /// 单个段落语音合成
    Synthesize {
        /// 剧本段落 ID
        #[arg(long)]
        segment_id: String,

        /// 说话角色标识
        #[arg(long, default_value = "HOST")]
        role: String,

        /// 发音台词文本
        #[arg(long)]
        spoken: String,

        /// 角色音色与情感语调描述
        #[arg(long, default_value = "Clear, confident narrator")]
        direction: String,

        /// 预估时长
        #[arg(long, default_value = "2.0")]
        duration: f64,

        /// 输出音频路径 (如 assets/voice_hook.wav)
        #[arg(long, short)]
        output: PathBuf,

        /// 强制使用本地离线合成器 (macOS say / ffmpeg)
        #[arg(long)]
        mock: bool,

        /// 预设音色名称 (如 alloy, echo, shimmer)
        #[arg(long)]
        voice: Option<String>,
    },

    /// 从 project.json 或 target_script.svml 批量生成全片 A-roll 配音
    Batch {
        /// SVML 剧本文件路径
        #[arg(long)]
        script: PathBuf,
        #[arg(long)]
        voice: Option<String>,
        #[arg(long, default_value = "en")]
        language: String,

        /// 声音资产输出目录 (默认 assets/voice)
        #[arg(long, default_value = "assets/voice")]
        out_dir: PathBuf,

        /// 角色音色总体导演描述
        #[arg(
            long,
            default_value = "A confident, clear conversational narrator with crisp pronunciation"
        )]
        direction: String,

        /// 强制使用本地离线合成器
        #[arg(long)]
        mock: bool,
    },

    /// 二次对齐与时间线弹性拉伸 (二次对齐核心闭环)
    Realign {
        /// SVML 剧本文件路径
        #[arg(long)]
        script: PathBuf,

        /// 初始 project.json 路径
        #[arg(long)]
        project: PathBuf,

        /// 存放已合成音频的目录
        #[arg(long, default_value = "assets/voice")]
        audio_dir: PathBuf,

        /// 测得的实测时长清单输出路径 (如 speech_extents.json)
        #[arg(long)]
        out_extents: Option<PathBuf>,

        /// 弹性拉伸后更新的 project_aligned.json 输出路径
        #[arg(long)]
        out_project: Option<PathBuf>,

        /// 帧率 (默认 30.0)
        #[arg(long, default_value = "30.0")]
        fps: f64,

        /// 段落自然停顿呼吸秒数 (默认 0.3)
        #[arg(long, default_value = "0.3")]
        gap: f64,
    },
}

#[derive(Subcommand, Debug)]
pub enum MixSubcommands {
    /// 规划并查看多轨混音工程方案 (Audio Program Plan & Ducking Envelope)
    Plan {
        /// 已完成二次对齐的项目蓝图 project_aligned.json
        #[arg(long)]
        project: PathBuf,

        /// SVML 剧本文件路径
        #[arg(long)]
        script: PathBuf,

        /// 配音音频目录 (默认 assets/voice)
        #[arg(long, default_value = "assets/voice")]
        voice_dir: PathBuf,

        /// 背景音乐 (BGM) 文件路径 (可选)
        #[arg(long)]
        bgm: Option<PathBuf>,

        /// 闪避压制音量增益 (默认 0.18, 约 -15dB)
        #[arg(long, default_value = "0.18")]
        ducked_gain: f64,

        /// 正常背景音乐增益 (默认 0.45, 约 -7dB)
        #[arg(long, default_value = "0.45")]
        normal_gain: f64,
    },

    /// 离线合成单个声效 (whoosh, pop, ding, bell, shutter)
    Sfx {
        /// 音效风格
        #[arg(long, default_value = "whoosh")]
        style: String,

        /// 输出 WAV 路径
        #[arg(long, short)]
        output: PathBuf,

        /// 采样率 (默认 48000)
        #[arg(long, default_value = "48000")]
        sample_rate: u32,
    },

    /// 执行物理级多轨母带混音压制 (Render Master Mix)
    Render {
        /// 已完成二次对齐的 project_aligned.json
        #[arg(long)]
        project: PathBuf,

        /// SVML 剧本文件路径 (用于提取 Moment 标记并自动吸附音效)
        #[arg(long)]
        script: PathBuf,
        #[arg(long)]
        mock_sfx: bool,
        /// Explicit prepared AudioClip list with observed event times
        #[arg(long)]
        sfx_plan: Option<PathBuf>,

        /// 配音音频所在目录 (默认 assets/voice)
        #[arg(long, default_value = "assets/voice")]
        voice_dir: PathBuf,

        /// 背景音乐 (BGM) 文件路径 (可选，未提供时默认使用 fixtures/soundtrack.m4a)
        #[arg(long)]
        bgm: Option<PathBuf>,

        /// 音效输出目录 (默认 assets/audio/sfx)
        #[arg(long, default_value = "assets/audio/sfx")]
        sfx_dir: PathBuf,

        /// 输出母带混音音频文件路径 (默认 assets/audio/master_mix.wav)
        #[arg(long, default_value = "assets/audio/master_mix.wav")]
        out_mix: PathBuf,

        /// 输出更新后的项目蓝图清单 (默认 project_mixed.json)
        #[arg(long)]
        out_project: Option<PathBuf>,

        /// 闪避压制音量增益 (默认 0.18)
        #[arg(long, default_value = "0.18")]
        ducked_gain: f64,

        /// 正常背景音乐增益 (默认 0.45)
        #[arg(long, default_value = "0.45")]
        normal_gain: f64,
    },
}

#[derive(Subcommand, Debug)]
pub enum CaptionSubcommands {
    /// 从 SVML 剧本文档解析出规范的 CaptionDocument (词/单元/Cue不变式校验)
    ParseDoc {
        /// 输入 SVML 剧本文件
        #[arg(long)]
        script: PathBuf,

        /// 输出 CaptionDocument JSON 文件
        #[arg(long)]
        to: Option<PathBuf>,

        /// 控制台打印 JSON 格式
        #[arg(long)]
        json: bool,
    },

    /// 核心时间轴调度：结合语音时长生成可见性窗口 (含 lead/tail/cut 算法与可选人头追踪)
    Schedule {
        /// 输入 SVML 剧本文件
        #[arg(long)]
        script: Option<PathBuf>,

        /// 输入已解析的 CaptionDocument JSON (二选一)
        #[arg(long)]
        doc: Option<PathBuf>,

        /// 输入工程项目清单 project.json 或 project_aligned.json (从中提取语音资产时长)
        #[arg(long)]
        project: Option<PathBuf>,

        /// 传入转录或词级对齐 JSON 文件 (TranscriptDocument)，用于字词级精确排期
        #[arg(long)]
        transcript: Option<PathBuf>,

        /// 或直接输入语音时长区间 JSON 文件 [[start, end], ...]
        #[arg(long)]
        intervals: Option<PathBuf>,

        /// 字幕样式配方 JSON 文件 (可选)
        #[arg(long)]
        recipe: Option<PathBuf>,

        /// 人脸/人头区域观测证据 JSON 文件 (可选，用于动态人头追踪)
        #[arg(long)]
        observations: Option<PathBuf>,

        /// 是否启用动态人头追踪
        #[arg(long)]
        track_head: bool,

        /// 追踪目标角色 (默认 HOST)
        #[arg(long, default_value = "HOST")]
        target_role: String,

        /// 输出 CaptionSchedule JSON 文件
        #[arg(long)]
        to: Option<PathBuf>,

        /// 直接导出为 WebVTT 字幕文件路径 (如 subtitles.vtt)
        #[arg(long)]
        to_vtt: Option<PathBuf>,

        /// 直接导出为 ASS 字幕文件路径 (如 subtitles.ass)
        #[arg(long)]
        to_ass: Option<PathBuf>,

        /// 控制台打印 JSON 格式
        #[arg(long)]
        json: bool,
    },

    /// 导出为目标渲染载体格式 (ASS, WebVTT, 动态 Web-as-Video HTML)
    Export {
        /// 输入排期结果 CaptionSchedule JSON
        #[arg(long)]
        schedule: PathBuf,

        /// 导出格式 (ass, vtt, html, json)
        #[arg(long, default_value = "ass")]
        format: String,

        /// 画布宽度 (默认 640，对齐参考工程)
        #[arg(long, default_value = "640")]
        width: u32,

        /// 画布高度 (默认 360，对齐参考工程)
        #[arg(long, default_value = "360")]
        height: u32,

        /// 样式配方 JSON 文件 (可选)
        #[arg(long)]
        recipe: Option<PathBuf>,

        /// 输出文件路径 (可选，未指定时打印到控制台)
        #[arg(long)]
        to: Option<PathBuf>,
    },

    /// 实时时间戳状态查询：获取给定秒数下的字词高亮、活动词元与画布坐标
    Query {
        /// 输入排期结果 CaptionSchedule JSON
        #[arg(long)]
        schedule: PathBuf,

        /// 查询时间戳 (秒)
        #[arg(long)]
        time: f64,

        /// 画布宽度
        #[arg(long, default_value = "640")]
        width: u32,

        /// 画布高度
        #[arg(long, default_value = "360")]
        height: u32,

        /// 样式配方 JSON 文件 (可选)
        #[arg(long)]
        recipe: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ComponentSubcommands {
    /// 查看系统支持的所有视觉组件原型 (Archetypes) 与默认属性
    ListArchetypes,

    /// 实例化设计单个视觉组件并编译为自包含 HTML 文件
    Design {
        /// 原型名称 (task_engine, timeline_compare, code_artifact, feature_metric, editorial_banner)
        #[arg(long)]
        archetype: String,

        /// 组件标题
        #[arg(long, default_value = "Visual Component")]
        title: String,

        /// 自定义属性 JSON 字符串或文件路径
        #[arg(long)]
        props: Option<String>,

        /// 画布宽度 (默认 640)
        #[arg(long, default_value = "640")]
        width: u32,

        /// 画布高度 (默认 360)
        #[arg(long, default_value = "360")]
        height: u32,

        /// 动画总帧数 (默认 60 帧)
        #[arg(long, default_value = "60")]
        duration_frames: u64,

        /// 帧率 (默认 30.0)
        #[arg(long, default_value = "30.0")]
        fps: f64,

        /// 输出 HTML 文件路径 (若未指定则打印到控制台)
        #[arg(long, short)]
        to: Option<PathBuf>,
    },

    /// 扫描工程清单与剧本，批量自动设计并生成全镜头对应的 HTML 组件
    #[command(name = "auto-design", alias = "auto")]
    AutoDesign {
        /// 项目蓝图清单 project.json
        #[arg(long)]
        project: PathBuf,

        /// SVML 剧本文件
        #[arg(long)]
        script: PathBuf,

        /// 组件输出目录 (默认 assets/components)
        #[arg(long, default_value = "assets/components")]
        out_dir: PathBuf,

        /// 更新并写回项目清单路径 (可选，如 project_with_components.json)
        #[arg(long)]
        update_project: Option<PathBuf>,
    },

    /// 检查并验证 HTML 视觉组件的规范合规性与运行时钩子
    Inspect {
        /// HTML 组件文件路径
        input: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
pub enum CaptureSubcommands {
    /// 探测本地可用的 Google Chrome / Chromium 无头浏览器环境
    Detect,

    /// 截取 HTML 视听组件在特定帧索引处的单帧 PNG 图像
    Screenshot {
        /// 输入 HTML 组件路径
        #[arg(long, short)]
        input: PathBuf,

        /// 目标帧索引 (默认 0)
        #[arg(long, default_value = "0")]
        frame: u64,

        /// 总帧数基数 (用于计算 progress，默认 60)
        #[arg(long, default_value = "60")]
        total_frames: u64,

        /// 画布宽度 (默认 640)
        #[arg(long, default_value = "640")]
        width: u32,

        /// 画布高度 (默认 360)
        #[arg(long, default_value = "360")]
        height: u32,

        /// 输出 PNG 图像文件路径
        #[arg(long, short)]
        to: PathBuf,

        /// 强制使用纯 Rust Mock 离线兜底 (无需 Chrome)
        #[arg(long)]
        mock: bool,
    },

    /// 捕获指定帧区间的完整 PNG 图像序列
    Sequence {
        /// 输入 HTML 组件路径
        #[arg(long, short)]
        input: PathBuf,

        /// 起始帧 (包含，默认 0)
        #[arg(long, default_value = "0")]
        start_frame: u64,

        /// 结束帧 (不包含，默认 30)
        #[arg(long, default_value = "30")]
        end_frame: u64,

        /// 画布宽度 (默认 640)
        #[arg(long, default_value = "640")]
        width: u32,

        /// 画布高度 (默认 360)
        #[arg(long, default_value = "360")]
        height: u32,

        /// 输出目录 (序列图像存储目录)
        #[arg(long, short)]
        out_dir: PathBuf,

        /// 强制使用纯 Rust Mock 离线兜底
        #[arg(long)]
        mock: bool,
    },

    /// 将 HTML 组件逐帧收割并编码为 BT.709 yuv420p 广播级 MP4 视频
    Record {
        /// 输入 HTML 组件路径
        #[arg(long, short)]
        input: PathBuf,

        /// 总帧数 (默认 60)
        #[arg(long, default_value = "60")]
        total_frames: u64,

        /// 帧率 (默认 30.0)
        #[arg(long, default_value = "30.0")]
        fps: f64,

        /// 画布宽度 (默认 640)
        #[arg(long, default_value = "640")]
        width: u32,

        /// 画布高度 (默认 360)
        #[arg(long, default_value = "360")]
        height: u32,

        /// x264 CRF 质量因子 (默认 23)
        #[arg(long, default_value = "23")]
        crf: u32,

        /// 输出 MP4 视频文件路径
        #[arg(long, short)]
        to: PathBuf,

        /// 保持透明通道 (默认 true，适用于叠加组件)
        #[arg(long, default_value = "true")]
        transparent: bool,

        /// 强制使用纯 Rust Mock 离线兜底
        #[arg(long)]
        mock: bool,
    },

    /// 扫描项目清单中的 visual_components 轨道项，批量收割并写回带 video_path 的新清单
    Batch {
        /// 项目蓝图清单 project_with_components.json
        #[arg(long)]
        project: PathBuf,

        /// 可选指定组件 HTML 所在目录（若指定则优先在此目录下查找组件 HTML）
        #[arg(long)]
        components_dir: Option<PathBuf>,

        /// 视频片段输出目录 (默认 assets/video)
        #[arg(long, default_value = "assets/video")]
        out_dir: PathBuf,

        /// 帧率 (默认 30.0)
        #[arg(long, default_value = "30.0")]
        fps: f64,

        /// 画布宽度 (默认 640)
        #[arg(long, default_value = "640")]
        width: u32,

        /// 画布高度 (默认 360)
        #[arg(long, default_value = "360")]
        height: u32,

        /// 更新并写回项目清单路径 (如 project_with_rendered_components.json)
        #[arg(long)]
        update_project: Option<PathBuf>,

        /// 保持透明通道 (默认 true，适用于叠加组件)
        #[arg(long, default_value = "true")]
        transparent: bool,

        /// 强制使用纯 Rust Mock 离线兜底
        #[arg(long)]
        mock: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum ComposeSubcommands {
    /// 编译并检查项目的多层合成方案 (CompositePlan) 与 FFmpeg 复杂滤镜链
    Plan {
        /// 项目蓝图清单 (如 project_with_rendered_components.json)
        #[arg(long)]
        project: PathBuf,
        #[arg(long, default_value="burn-in",value_parser=["burn-in","soft","none"])]
        subtitle_delivery: String,

        /// 项目根资产目录 (默认项目所在目录)
        #[arg(long)]
        project_dir: Option<PathBuf>,

        /// 目标输出视频路径 (默认 master.mp4)
        #[arg(long, default_value = "master.mp4")]
        output: PathBuf,
    },

    /// 执行完整多层压制合成，渲染最终广播级成片视频
    Render {
        /// 项目蓝图清单 (如 project_with_rendered_components.json)
        #[arg(long)]
        project: PathBuf,
        #[arg(long, default_value="burn-in",value_parser=["burn-in","soft","none"])]
        subtitle_delivery: String,

        /// 项目根资产目录 (默认项目所在目录)
        #[arg(long)]
        project_dir: Option<PathBuf>,

        /// 目标输出视频路径 (默认 master.mp4)
        #[arg(long, short, default_value = "master.mp4")]
        output: PathBuf,

        /// 强制使用纯 Rust Mock 离线兜底
        #[arg(long)]
        mock: bool,
    },

    /// 独立字幕烧录工具：将 ASS/WebVTT 字幕直接压制到已有视频中
    BurnSubtitles {
        /// 输入视频文件
        #[arg(long, short)]
        input: PathBuf,

        /// 字幕文件 (ASS 或 WebVTT)
        #[arg(long, short)]
        subtitles: PathBuf,
        #[arg(long, default_value="burn-in",value_parser=["burn-in","soft"])]
        subtitle_delivery: String,

        /// 输出视频文件
        #[arg(long, short)]
        output: PathBuf,
    },
}

async fn resolve_sample_times(
    input: &PathBuf,
    at: Option<String>,
    start: Option<f64>,
    end: Option<f64>,
    every: Option<f64>,
    frames: Option<usize>,
) -> Result<Vec<f64>> {
    if let Some(at_str) = at {
        let mut times = Vec::new();
        for s in at_str.split(',') {
            let t: f64 = s
                .trim()
                .parse()
                .with_context(|| format!("Invalid timestamp in --at: '{}'", s))?;
            times.push(t);
        }
        if times.is_empty() {
            bail!("--at specified but no valid timestamps parsed");
        }
        return Ok(times);
    }

    let meta = probe_media(input).await?;
    let start_sec = start.unwrap_or(0.0).clamp(0.0, meta.duration_seconds);
    let end_sec = end
        .unwrap_or(meta.duration_seconds)
        .clamp(start_sec, meta.duration_seconds);

    if (end_sec - start_sec) <= 0.001 {
        bail!(
            "Time range too small or invalid: {:.3}s to {:.3}s",
            start_sec,
            end_sec
        );
    }

    let mut times = Vec::new();
    if let Some(interval) = every {
        if interval < 0.01 {
            bail!("--every must be >= 0.01s");
        }
        let mut t = start_sec;
        while t < end_sec {
            times.push(((t * 1000.0).round()) / 1000.0);
            t += interval;
        }
    } else {
        let count = frames.unwrap_or_else(|| {
            let span = end_sec - start_sec;
            (span * 1.5).round().clamp(4.0, 9.0) as usize
        });
        let count = count.max(2);
        let step = (end_sec - start_sec) / (count as f64);
        for i in 0..count {
            let t = start_sec + (i as f64 + 0.5) * step;
            times.push(((t * 1000.0).round()) / 1000.0);
        }
    }

    Ok(times)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Probe { input, json } => {
            let info = probe_media(&input).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&info)?);
            } else {
                println!("┌────────────────────────────────────────────────────────┐");
                println!("│ Media Probe Results                                    │");
                println!("├────────────────────────────────────────────────────────┤");
                println!("│ File      : {:<42} │", info.path);
                println!(
                    "│ Duration  : {:<42} │",
                    format!("{:.3} s", info.duration_seconds)
                );
                if let (Some(w), Some(h)) = (info.width, info.height) {
                    println!("│ Resolution: {:<42} │", format!("{}x{}", w, h));
                }
                if let Some(fps) = info.fps {
                    println!("│ Frame Rate: {:<42} │", format!("{:.2} fps", fps));
                }
                if let Some(v_codec) = info.video_codec {
                    println!("│ Video Codec: {:<41} │", v_codec);
                }
                println!(
                    "│ Audio     : {:<42} │",
                    if info.has_audio { "Yes" } else { "No" }
                );
                if let Some(a_codec) = info.audio_codec {
                    println!("│ Audio Codec: {:<41} │", a_codec);
                }
                if let Some(ch) = info.audio_channels {
                    println!("│ Audio Chan: {:<42} │", format!("{} channels", ch));
                }
                if let Some(sr) = info.audio_sample_rate {
                    println!("│ Sample Rate: {:<41} │", format!("{} Hz", sr));
                }
                println!("└────────────────────────────────────────────────────────┘");
            }
        }
        Commands::Cut {
            input,
            start,
            end,
            output,
            accurate,
        } => {
            println!(
                "✂️  Cutting media: {} from {:.3}s to {:.3}s (mode: {})...",
                input.display(),
                start,
                end,
                if accurate {
                    "re-encode accurate"
                } else {
                    "fast copy"
                }
            );

            cut_media(CutOptions {
                input_path: &input,
                output_path: &output,
                start_seconds: start,
                end_seconds: end,
                accurate,
            })
            .await?;

            println!("✅ Successfully saved slice to: {}", output.display());
        }
        Commands::Transcribe {
            input,
            to,
            language,
            api_key,
            api_url,
            local,
            mock,
            json,
        } => {
            if !json {
                println!(
                    "🎙️  Transcribing audio from: {} (language: {})...",
                    input.display(),
                    language
                );
            } else {
                eprintln!(
                    "🎙️  Transcribing audio from: {} (language: {})...",
                    input.display(),
                    language
                );
            }

            let doc = transcribe_media(TranscribeOptions {
                source_path: &input,
                language: &language,
                api_key,
                api_url,
                local,
                mock,
            })
            .await?;

            if let Some(parent) = to.parent() {
                if !parent.exists() {
                    tokio::fs::create_dir_all(parent).await?;
                }
            }

            let json_str = serde_json::to_string_pretty(&doc)?;
            tokio::fs::write(&to, &json_str).await?;

            if json {
                println!("{}", json_str);
            } else {
                println!("✅ Successfully generated transcript: {}", to.display());
                println!("   - Format   : {}", doc.format);
                println!("   - Duration : {:.3} s", doc.audio_seconds);
                println!("   - Passages : {}", doc.passages.len());
                let total_words: usize = doc.passages.iter().map(|p| p.words.len()).sum();
                println!("   - Words    : {}", total_words);
            }
        }
        Commands::Boundaries {
            input,
            rate,
            threshold,
            json,
        } => {
            let res = detect_visual_boundaries(&input, rate, threshold).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&res)?);
            } else {
                let count = res.candidates.len();
                let noun = if count == 1 {
                    "candidate"
                } else {
                    "candidates"
                };
                println!("{}", res.path);
                println!(
                    "  {} visual-change {} at {} samples/s; scores are not shot labels",
                    count, noun, res.sample_rate
                );
                let shown = res.candidates.iter().take(40);
                for c in shown {
                    println!("  {:>6.3} s  score {:.3}", c.at, c.score);
                }
                if count > 40 {
                    println!("  … {} more; --json returns all candidates", count - 40);
                }
            }
        }
        Commands::Frames {
            input,
            start,
            end,
            at,
            every,
            frames,
            label_time,
            transcript,
            to,
            json,
        } => {
            let times = resolve_sample_times(&input, at, start, end, every, frames).await?;
            let transcript_doc = if let Some(t_path) = transcript {
                let content = tokio::fs::read_to_string(&t_path).await?;
                Some(serde_json::from_str::<TranscriptDocument>(&content)?)
            } else {
                None
            };

            let res = extract_frames(FramesOptions {
                source: &input,
                target_dir: &to,
                times,
                label_time,
                transcript: transcript_doc.as_ref(),
            })
            .await?;

            if json {
                println!("{}", serde_json::to_string_pretty(&res)?);
            } else {
                println!("{}", res.directory);
                println!(
                    "  {} {} from {:.3} s to {:.3} s",
                    res.frames.len(),
                    if res.labeled {
                        "time-labeled frames"
                    } else {
                        "frames"
                    },
                    res.frames.first().map(|f| f.at).unwrap_or(0.0),
                    res.frames.last().map(|f| f.at).unwrap_or(0.0)
                );
            }
        }
        Commands::Tile {
            input,
            start,
            end,
            at,
            every,
            frames,
            cell,
            columns,
            transcript,
            to,
            json,
        } => {
            let times = resolve_sample_times(&input, at, start, end, every, frames).await?;
            let transcript_doc = if let Some(t_path) = transcript {
                let content = tokio::fs::read_to_string(&t_path).await?;
                Some(serde_json::from_str::<TranscriptDocument>(&content)?)
            } else {
                None
            };

            let cell_width = cell.unwrap_or(DEFAULT_TILE_CELL_WIDTH);
            let cols = columns.unwrap_or(DEFAULT_TILE_COLUMNS);

            let res = create_tile(TileOptions {
                source: &input,
                target_file: &to,
                times,
                cell_width,
                columns: cols,
                transcript: transcript_doc.as_ref(),
            })
            .await?;

            if json {
                println!("{}", serde_json::to_string_pretty(&res)?);
            } else {
                println!("{}", res.path);
                println!(
                    "  {} labeled frames, {}×{}, {} px cells",
                    res.samples.len(),
                    res.columns,
                    res.rows,
                    res.cell_width
                );
                let time_list = res
                    .samples
                    .iter()
                    .map(|t| format!("{:.3}s", t))
                    .collect::<Vec<_>>()
                    .join("  ");
                println!("  {}", time_list);
            }
        }
        Commands::Tiles {
            input,
            start,
            end,
            at,
            every,
            frames,
            cell,
            columns,
            rows,
            transcript,
            to,
            json,
        } => {
            let times = resolve_sample_times(&input, at, start, end, every, frames).await?;
            let transcript_doc = if let Some(t_path) = transcript {
                let content = tokio::fs::read_to_string(&t_path).await?;
                Some(serde_json::from_str::<TranscriptDocument>(&content)?)
            } else {
                None
            };

            let cell_width = cell.unwrap_or(DEFAULT_TILE_CELL_WIDTH);
            let cols = columns.unwrap_or(DEFAULT_TILE_COLUMNS);
            let rws = rows.unwrap_or(DEFAULT_TILE_ROWS);

            let res = create_tiles(TilesOptions {
                source: &input,
                target_dir: &to,
                times,
                cell_width,
                columns: cols,
                rows: rws,
                transcript: transcript_doc.as_ref(),
            })
            .await?;

            if json {
                println!("{}", serde_json::to_string_pretty(&res)?);
            } else {
                println!("{}", res.directory);
                println!(
                    "  {} time-labeled {}",
                    res.grids.len(),
                    if res.grids.len() == 1 {
                        "grid"
                    } else {
                        "grids"
                    }
                );
            }
        }
        Commands::Script {
            input,
            speech,
            dialogue,
            caption,
            estimate,
            to,
            json,
        } => {
            let raw_content = tokio::fs::read_to_string(&input)
                .await
                .with_context(|| format!("Failed to read SVML file {:?}", input))?;

            let doc = parse_script(&raw_content)?;

            let output_text = if json {
                serde_json::to_string_pretty(&doc)?
            } else if speech {
                doc.to_spoken_stream()
            } else if dialogue {
                doc.to_dialogue_stream()
            } else if caption {
                let cues = doc.to_caption_cues();
                cues.iter()
                    .map(|c| format!("[Cue {:02}] {}", c.index, c.text))
                    .collect::<Vec<_>>()
                    .join("\n")
            } else if estimate {
                let mut report = format!("📜 Script ID: {}\n", doc.id);
                report.push_str(&format!("  Total Segments : {}\n", doc.segments.len()));
                report.push_str(&format!("  Total Units    : {}\n", doc.total_words));
                report.push_str(&format!(
                    "  Est. Duration  : {:.1} s\n\n",
                    doc.total_estimated_seconds
                ));
                report.push_str("  Segments Breakdown:\n");
                for s in &doc.segments {
                    report.push_str(&format!(
                        "    - [{:<12}] : {:.1}s  ({} turns, {} words)\n",
                        s.id,
                        s.estimated_seconds,
                        s.turns.len(),
                        s.turns.iter().map(|t| t.units.len()).sum::<usize>()
                    ));
                }
                report
            } else {
                // 默认综述模式
                let mut out = format!("📜 SVML Script Parsed: {}\n", input.display());
                out.push_str(&format!("  ID             : {}\n", doc.id));
                out.push_str(&format!("  Segments       : {}\n", doc.segments.len()));
                out.push_str(&format!(
                    "  Est. Duration  : {:.1} s\n",
                    doc.total_estimated_seconds
                ));
                out.push_str("\n--- Spoken Stream Preview ---\n");
                out.push_str(&doc.to_spoken_stream());
                out
            };

            if let Some(out_path) = to {
                if let Some(parent) = out_path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::write(&out_path, &output_text).await?;
                println!("✅ Successfully exported script view to: {:?}", out_path);
            } else {
                println!("{}", output_text);
            }
        }
        Commands::DynamicTimeline {
            input,
            fps,
            gap,
            speech_extents,
            transcript,
            to,
            json,
            summary,
        } => {
            let content = std::fs::read_to_string(&input)
                .with_context(|| format!("Failed to read script or timeline file: {:?}", input))?;

            let doc = parse_script(&content)?;

            let mut extents_map: Option<HashMap<String, f64>> =
                if let Some(ref ext_path) = speech_extents {
                    let ext_content = std::fs::read_to_string(ext_path).with_context(|| {
                        format!("Failed to read speech extents from {:?}", ext_path)
                    })?;
                    let map: HashMap<String, f64> = serde_json::from_str(&ext_content)
                        .with_context(|| {
                            format!("Failed to parse speech extents JSON in {:?}", ext_path)
                        })?;
                    Some(map)
                } else {
                    None
                };

            let mut unit_timings_map: Option<HashMap<String, Vec<(f64, f64)>>> = None;

            if let Some(ref trans_path) = transcript {
                let trans_content = std::fs::read_to_string(trans_path)
                    .with_context(|| format!("Failed to read transcript from {:?}", trans_path))?;
                let trans_doc = serde_json::from_str::<TranscriptDocument>(&trans_content)
                    .with_context(|| {
                        format!("Failed to parse transcript JSON from {:?}", trans_path)
                    })?;
                if trans_doc.passages.is_empty() {
                    bail!("Transcript at {:?} contains no passages", trans_path);
                }

                let (measured, timings) = speech_alignment::align_script(&doc, &trans_doc)?;
                let mut extents = extents_map.unwrap_or_default();
                for (id, duration) in measured {
                    if let Some(explicit) = extents.get(&id) {
                        if (explicit - duration).abs() > 1.0 / fps {
                            bail!("Speech extent conflicts with evidence for '{}'", id);
                        }
                    }
                    extents.insert(id, duration);
                }
                extents_map = Some(extents);
                unit_timings_map = Some(timings);
            }

            let resolved: ResolvedTimeline = if let Some(ref timings) = unit_timings_map {
                project_script_to_timeline_with_alignments(
                    &doc,
                    fps,
                    gap,
                    extents_map.as_ref(),
                    Some(timings),
                )?
            } else {
                project_script_to_timeline(&doc, fps, gap, extents_map.as_ref())?
            };

            if json {
                let serialized = serde_json::to_string_pretty(&resolved)?;
                if let Some(ref out_path) = to {
                    if let Some(parent) = out_path.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    tokio::fs::write(out_path, &serialized).await?;
                    println!("Saved resolved timeline JSON to: {:?}", out_path);
                } else {
                    println!("{}", serialized);
                }
            } else {
                let ascii_chart = render_ascii_timeline(&resolved);
                if let Some(ref out_path) = to {
                    if let Some(parent) = out_path.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    tokio::fs::write(out_path, &ascii_chart).await?;
                    println!("Saved resolved timeline visualization to: {:?}", out_path);
                } else {
                    print!("{}", ascii_chart);
                    if !summary {
                        println!("Tip: use --json to output machine-readable timeline JSON for downstream production.");
                    }
                }
            }
        }
        Commands::Project {
            script,
            timeline,
            width,
            height,
            title,
            speaking_video,
            no_bgm,
            gap,
            speech_extents,
            fps,
            to,
            json,
            validate,
        } => {
            let script_content = std::fs::read_to_string(&script)
                .with_context(|| format!("Failed to read script file: {:?}", script))?;
            let doc = parse_script(&script_content)?;

            let resolved_tl: ResolvedTimeline = if let Some(ref tl_path) = timeline {
                let tl_content = std::fs::read_to_string(tl_path)
                    .with_context(|| format!("Failed to read timeline file: {:?}", tl_path))?;
                serde_json::from_str(&tl_content)
                    .with_context(|| format!("Failed to parse timeline JSON: {:?}", tl_path))?
            } else {
                let extents_map: Option<HashMap<String, f64>> =
                    if let Some(ref ext_path) = speech_extents {
                        let ext_content = std::fs::read_to_string(ext_path).with_context(|| {
                            format!("Failed to read speech extents: {:?}", ext_path)
                        })?;
                        Some(serde_json::from_str(&ext_content)?)
                    } else {
                        None
                    };
                project_script_to_timeline(&doc, fps, gap, extents_map.as_ref())?
            };

            let mut manifest: ProjectManifest =
                create_project_manifest(&doc, &resolved_tl, title.as_deref(), width, height)?;

            if speaking_video {
                project::use_speaking_video(&mut manifest, &doc)?;
            }
            if no_bgm {
                manifest.tracks.bgm.clear();
            }

            if validate {
                let notices = validate_project(&manifest)?;
                if notices.is_empty() {
                    if !json {
                        println!("✅ Project verification passed: 0 errors, 0 warnings.");
                    } else {
                        eprintln!("✅ Project verification passed: 0 errors, 0 warnings.");
                    }
                } else {
                    for n in notices {
                        if !json {
                            println!("⚠️ {}", n);
                        } else {
                            eprintln!("⚠️ {}", n);
                        }
                    }
                }
            }

            if json {
                let serialized = serde_json::to_string_pretty(&manifest)?;
                if let Some(ref out_path) = to {
                    if let Some(parent) = out_path.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    tokio::fs::write(out_path, &serialized).await?;
                }
                println!("{}", serialized);
            } else {
                let summary_text = render_project_summary(&manifest);
                if let Some(ref out_path) = to {
                    if let Some(parent) = out_path.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    tokio::fs::write(out_path, &summary_text).await?;
                    println!("Saved project summary to: {:?}", out_path);
                } else {
                    print!("{}", summary_text);
                    println!("\nTip: use --json to save machine-readable 'project.json' blueprint for downstream production.");
                }
            }
        }

        Commands::Image { sub } => match sub {
            ImageSubcommands::Expand {
                person,
                shot,
                setting,
                capture,
                aspect_ratio,
                resolution,
                json,
            } => {
                let spec = ImagePromptSpec {
                    capture,
                    person,
                    shot,
                    setting,
                    aspect_ratio,
                    resolution,
                    negative_prompt: None,
                };
                let full_prompt = assemble_prompt(&spec);
                let audit = audit_prompt(&spec);

                if json {
                    let out_json = serde_json::json!({
                        "spec": spec,
                        "full_prompt": full_prompt,
                        "audit": audit,
                        "prompt_hash": compute_prompt_hash(&full_prompt),
                    });
                    println!("{}", serde_json::to_string_pretty(&out_json)?);
                } else {
                    println!("============================================================");
                    println!("🎬 Four-Paragraph Visual Prompt (4 Responsibilities)");
                    println!("============================================================");
                    println!("{}", full_prompt);
                    println!("------------------------------------------------------------");
                    println!(
                        "📊 Prompt Audit: {} words | Valid: {}",
                        audit.word_count, audit.is_valid
                    );
                    for s in audit.strengths {
                        println!("  ✓ {}", s);
                    }
                    for w in audit.warnings {
                        println!("  ⚠️ {}", w);
                    }
                }
            }

            ImageSubcommands::Audit {
                spec_file,
                person,
                shot,
                setting,
                capture,
            } => {
                let spec = if let Some(path) = spec_file {
                    let content = std::fs::read_to_string(&path)?;
                    serde_json::from_str(&content)?
                } else {
                    ImagePromptSpec {
                        capture,
                        person: person.unwrap_or_default(),
                        shot: shot.unwrap_or_default(),
                        setting: setting.unwrap_or_default(),
                        aspect_ratio: "9:16".to_string(),
                        resolution: "1K".to_string(),
                        negative_prompt: None,
                    }
                };

                let audit = audit_prompt(&spec);
                println!("Prompt Audit Result:");
                println!("  Word count: {}", audit.word_count);
                println!("  Has capture: {}", audit.has_capture);
                println!("  Has framing/shot: {}", audit.has_framing_or_shot);
                println!("  Has gaze/encounter: {}", audit.has_gaze_or_encounter);
                println!("  Has deliberate palette: {}", audit.has_deliberate_palette);
                println!("  Is valid: {}", audit.is_valid);
                if !audit.warnings.is_empty() {
                    println!("\nWarnings:");
                    for w in audit.warnings {
                        println!("  - {}", w);
                    }
                }
            }

            ImageSubcommands::Generate {
                reference_images,
                reference_media_ids,
                flow_project_id,
                spec,
                person,
                shot,
                setting,
                capture,
                aspect_ratio,
                resolution,
                shot_id,
                output,
                mock,
                seed,
            } => {
                let prompt_spec = if let Some(p) = spec {
                    let text = std::fs::read_to_string(&p)?;
                    serde_json::from_str(&text)?
                } else {
                    let person_str = person.context("Must provide --person or --spec")?;
                    let shot_str = shot.context("Must provide --shot or --spec")?;
                    let setting_str = setting.context("Must provide --setting or --spec")?;
                    ImagePromptSpec {
                        capture,
                        person: person_str,
                        shot: shot_str,
                        setting: setting_str,
                        aspect_ratio,
                        resolution,
                        negative_prompt: None,
                    }
                };

                let req = ImageGenerationRequest {
                    spec: prompt_spec,
                    output_path: output,
                    shot_id,
                    seed,
                };

                if !reference_images.is_empty() || !reference_media_ids.is_empty() {
                    if mock || !flowkit::selected("IMAGE")? {
                        bail!("Reference images require the FlowKit image provider; inputs cannot be ignored");
                    }
                }
                let api_key_opt = std::env::var("OPENAI_API_KEY").ok();
                if !mock && !flowkit::selected("IMAGE")? && api_key_opt.is_none() {
                    anyhow::bail!(
                        "Missing OPENAI_API_KEY for image generation. Production mode prohibits silent mock degradation.\n\
                         Please set OPENAI_API_KEY or pass --mock explicitly for offline testing."
                    );
                }
                let use_mock = mock;

                let res = if use_mock {
                    let gen = MockImageGenerator::new();
                    gen.generate(&req)?
                } else if flowkit::selected("IMAGE")? {
                    flowkit::image_with_inputs(
                        &req,
                        &reference_images,
                        &reference_media_ids,
                        flow_project_id.as_deref(),
                    )
                    .await?
                } else {
                    let gen = OpenAiImageGenerator::new(
                        api_key_opt.unwrap_or_default(),
                        std::env::var("OPENAI_BASE_URL").ok(),
                        None,
                    );
                    gen.generate(&req).await?
                };

                println!("✅ Image generated successfully!");
                println!("  Shot ID:        {}", res.shot_id);
                println!("  Output:         {}", res.output_path);
                println!("  Metadata:       {}", res.metadata_path);
                println!(
                    "  Dimensions:     {}x{} ({})",
                    res.width, res.height, res.aspect_ratio
                );
                println!("  Provider/Model: {} / {}", res.provider, res.model);
                println!("  Prompt Hash:    {}", res.prompt_hash);
                println!("  Elapsed:        {} ms", res.elapsed_ms);
            }

            ImageSubcommands::Batch {
                reference_images,
                reference_media_ids,
                flow_project_id,
                project,
                out_dir,
                mock,
            } => {
                let content = std::fs::read_to_string(&project).with_context(|| {
                    format!("Failed to read project manifest from {:?}", project)
                })?;
                let manifest: ProjectManifest = serde_json::from_str(&content)?;

                println!("============================================================");
                println!(
                    "🎨 Batch Generating Visual Assets for Project: {}",
                    manifest.meta.title
                );
                println!("   Shots: {} | Output: {:?}", manifest.shots.len(), out_dir);
                println!("============================================================");

                if !reference_images.is_empty() || !reference_media_ids.is_empty() {
                    if mock || !flowkit::selected("IMAGE")? {
                        bail!("Reference images require the FlowKit image provider; inputs cannot be ignored");
                    }
                }
                let api_key_opt = std::env::var("OPENAI_API_KEY").ok();
                if !mock && !flowkit::selected("IMAGE")? && api_key_opt.is_none() {
                    anyhow::bail!(
                        "Missing OPENAI_API_KEY for image generation. Production mode prohibits silent mock degradation.\n\
                         Please set OPENAI_API_KEY or pass --mock explicitly for offline testing."
                    );
                }
                let use_mock = mock;
                if use_mock {
                    println!(
                        "Mode: Local Offline Mock Generator (Zero Cost, Deterministic Canvas)"
                    );
                } else {
                    println!("Mode: Configured image provider");
                }

                let (aspect_ratio, resolution) = if manifest.meta.width >= manifest.meta.height {
                    ("16:9", "preview")
                } else {
                    ("9:16", "preview")
                };

                let mut generated_count = 0;
                for shot in &manifest.shots {
                    let shot_output = out_dir.join(format!("{}.png", shot.id));
                    let person_desc = match &shot.prompt_hint {
                        Some(hint) if !hint.trim().is_empty() => hint.clone(),
                        _ => shot.description.clone(),
                    };

                    let default_spec = ImagePromptSpec {
                        capture: Some(DEFAULT_PHONE_UGC_CAPTURE.to_string()),
                        person: person_desc,
                        shot: format!(
                            "Clean visual composition for shot {} in segment {}. Aspect ratio {} with balanced foreground.",
                            shot.shot_index, shot.segment_id, aspect_ratio
                        ),
                        setting: "Modern high-tech production canvas. Palette: dark navy, pearl lavender and vibrant gold.".to_string(),
                        aspect_ratio: aspect_ratio.to_string(),
                        resolution: resolution.to_string(),
                        negative_prompt: None,
                    };

                    let spec = if let Some(task) = manifest.asset_tasks.iter().find(|task| {
                        task.task_type == AssetTaskType::AiImage
                            && task.target == shot.id
                            && task.spec.get("image_prompt").is_some()
                    }) {
                        serde_json::from_value(task.spec["image_prompt"].clone())?
                    } else {
                        default_spec
                    };
                    let req = ImageGenerationRequest {
                        spec,
                        output_path: shot_output,
                        shot_id: Some(shot.id.clone()),
                        seed: Some(shot.shot_index as u64),
                    };

                    let res = if use_mock {
                        let gen = MockImageGenerator::new();
                        gen.generate(&req)?
                    } else if flowkit::selected("IMAGE")? {
                        flowkit::image_with_inputs(
                            &req,
                            &reference_images,
                            &reference_media_ids,
                            flow_project_id.as_deref(),
                        )
                        .await?
                    } else {
                        let gen = OpenAiImageGenerator::new(
                            api_key_opt.clone().unwrap_or_default(),
                            std::env::var("OPENAI_BASE_URL").ok(),
                            None,
                        );
                        gen.generate(&req).await?
                    };

                    println!(
                        "  [{}/{}] Generated {} -> {} ({}x{}, {} ms)",
                        generated_count + 1,
                        manifest.shots.len(),
                        shot.id,
                        res.output_path,
                        res.width,
                        res.height,
                        res.elapsed_ms
                    );
                    generated_count += 1;
                }

                println!("------------------------------------------------------------");
                println!(
                    "✅ All {} shot visuals generated and indexed.",
                    generated_count
                );
            }
        },

        Commands::Video { sub } => match sub {
            VideoSubcommands::Expand {
                shot_id,
                first_frame,
                motion,
                camera,
                duration,
                fps,
                width,
                height,
                json,
            } => {
                let camera_motion = CameraMotion::from_str(&camera);
                let spec = VideoPromptSpec {
                    shot_id,
                    first_frame_path: first_frame.clone(),
                    motion_prompt: motion,
                    camera_motion,
                    relationship: if first_frame.is_some() {
                        VideoGenRelationship::ImageToVideo
                    } else {
                        VideoGenRelationship::TextToVideo
                    },
                    duration_seconds: duration,
                    fps,
                    width,
                    height,
                };

                let full_prompt = assemble_video_prompt(&spec);
                let audit = audit_video_prompt(&spec);

                if json {
                    let out_json = serde_json::json!({
                        "spec": spec,
                        "full_prompt": full_prompt,
                        "audit": audit,
                        "prompt_hash": compute_video_prompt_hash(&full_prompt),
                    });
                    println!("{}", serde_json::to_string_pretty(&out_json)?);
                } else {
                    println!("============================================================");
                    println!("🎬 Video Director Motion Specification");
                    println!("============================================================");
                    println!("{}", full_prompt);
                    println!("------------------------------------------------------------");
                    println!(
                        "📊 Motion Audit: {:.2}s ({} frames @ {:.1}fps) | Valid: {}",
                        audit.duration_seconds, audit.total_frames, spec.fps, audit.is_valid
                    );
                    for s in audit.strengths {
                        println!("  ✓ {}", s);
                    }
                    for w in audit.warnings {
                        println!("  ⚠️ {}", w);
                    }
                }
            }

            VideoSubcommands::Generate {
                flow_project_id,
                first_frame,
                dialogue_script,
                segment,
                source_video,
                source_edit,
                last_frame,
                reference_image,
                motion,
                camera,
                duration,
                fps,
                shot_id,
                width,
                height,
                output,
                mock,
                seed,
            } => {
                let camera_motion = CameraMotion::from_str(&camera);
                let mut spec = VideoPromptSpec {
                    shot_id,
                    first_frame_path: first_frame.clone(),
                    motion_prompt: motion,
                    camera_motion,
                    relationship: if source_video.is_some() {
                        VideoGenRelationship::SourceVideoEdit
                    } else if !reference_image.is_empty() {
                        VideoGenRelationship::ReferenceImages
                    } else if last_frame.is_some() {
                        VideoGenRelationship::FirstLastFrame
                    } else if first_frame.is_some() {
                        VideoGenRelationship::ImageToVideo
                    } else {
                        VideoGenRelationship::TextToVideo
                    },
                    duration_seconds: duration,
                    fps,
                    width,
                    height,
                };

                if let Some(script) = dialogue_script.as_ref() {
                    if mock {
                        bail!("Speaking video needs a real provider; mock cannot fabricate speech");
                    }
                    let mut doc = parse_script(&std::fs::read_to_string(script)?)?;
                    if let Some(id) = segment {
                        doc.segments.retain(|s| s.id == id);
                    }
                    video_director::attach_dialogue(&mut spec, &doc)?;
                }

                let req = VideoGenerationRequest {
                    spec,
                    output_path: output,
                    seed,
                };

                let extra = video_director::VideoInputs {
                    last_frame,
                    reference_images: reference_image,
                    source_video,
                    source_edit: source_edit
                        .map(|path| -> anyhow::Result<_> {
                            Ok(serde_json::from_slice::<video_director::SourceVideoEdit>(
                                &std::fs::read(path)?,
                            )?)
                        })
                        .transpose()?,
                };
                video_director::validate_inputs(&req.spec, &extra)?;
                if (mock || !flowkit::selected("VIDEO")?)
                    && (extra.last_frame.is_some()
                        || !extra.reference_images.is_empty()
                        || extra.source_video.is_some())
                {
                    bail!("Advanced references require the FlowKit adapter");
                }
                let api_key_opt = std::env::var("OPENAI_API_KEY").ok();
                if !mock && !flowkit::selected("VIDEO")? && api_key_opt.is_none() {
                    anyhow::bail!(
                        "Missing OPENAI_API_KEY for video generation. Production mode prohibits silent mock degradation.\n\
                         Please set OPENAI_API_KEY or pass --mock explicitly for offline testing."
                    );
                }
                let use_mock = mock;

                let res = if use_mock {
                    let gen = MockVideoGenerator::new();
                    gen.generate(&req)?
                } else if flowkit::selected("VIDEO")? {
                    flowkit::video_with_inputs(&req, flow_project_id.as_deref(), &extra).await?
                } else {
                    let gen = RemoteVideoGenerator::new(
                        api_key_opt.unwrap_or_default(),
                        std::env::var("OPENAI_BASE_URL").ok(),
                        None,
                    );
                    gen.generate(&req).await?
                };

                if dialogue_script.is_some() && !probe_media(&req.output_path).await?.has_audio {
                    bail!("Speaking video returned no audio; inspect the candidate before adoption");
                }

                println!("✅ Video clip generated successfully!");
                println!("  Shot ID:        {}", res.shot_id);
                println!("  Output:         {}", res.output_path);
                println!("  Metadata:       {}", res.metadata_path);
                println!(
                    "  Duration:       {:.3} s ({} frames @ {:.1} fps)",
                    res.duration_seconds, res.frames_count, res.fps
                );
                println!("  Dimensions:     {}x{}", res.width, res.height);
                println!("  Camera Motion:  {}", res.camera_motion);
                println!("  Provider/Model: {} / {}", res.provider, res.model);
                println!("  Prompt Hash:    {}", res.prompt_hash);
                println!("  Elapsed:        {} ms", res.elapsed_ms);
            }

            VideoSubcommands::Batch {
                flow_project_id,
                project,
                visual_dir,
                out_dir,
                mock,
            } => {
                let content = std::fs::read_to_string(&project).with_context(|| {
                    format!("Failed to read project manifest from {:?}", project)
                })?;
                let manifest: ProjectManifest = serde_json::from_str(&content)?;

                println!("============================================================");
                println!(
                    "🎬 Batch Synthesizing Video Clips for Project: {}",
                    manifest.meta.title
                );
                println!(
                    "   Shots: {} | Visual Dir: {:?} | Video Output: {:?}",
                    manifest.shots.len(),
                    visual_dir,
                    out_dir
                );
                println!("============================================================");

                let api_key_opt = std::env::var("OPENAI_API_KEY").ok();
                if !mock && !flowkit::selected("VIDEO")? && api_key_opt.is_none() {
                    anyhow::bail!(
                        "Missing OPENAI_API_KEY for video generation. Production mode prohibits silent mock degradation.\n\
                         Please set OPENAI_API_KEY or pass --mock explicitly for offline testing."
                    );
                }
                let use_mock = mock;
                if use_mock {
                    println!(
                        "Mode: Local Offline FFmpeg Motion Synthesizer (Real H.264 MP4, Zero Cost)"
                    );
                } else {
                    println!("Mode: Configured video provider");
                }

                let mut generated_count = 0;
                for shot in &manifest.shots {
                    let shot_output = out_dir.join(format!("{}.mp4", shot.id));

                    // 优先检查是否有特定命名的首帧图片（如 shot_01_codex.png 或 shot_01.png）
                    let first_frame_candidate1 = visual_dir.join(format!("{}.png", shot.id));
                    let first_frame_candidate2 = visual_dir.join(format!("{}_codex.png", shot.id));
                    let first_frame_path = if first_frame_candidate1.exists() {
                        first_frame_candidate1
                    } else if first_frame_candidate2.exists() {
                        first_frame_candidate2
                    } else {
                        bail!(
                            "No valid first frame image found in {:?} for {}",
                            visual_dir,
                            shot.id
                        );
                    };

                    // 根据镜头类型与顺序选择摄影机运镜
                    let default_camera_motion = match shot.shot_index {
                        1 => CameraMotion::PushIn,
                        2 => CameraMotion::PanRight,
                        3 => CameraMotion::TiltUp,
                        4 => CameraMotion::PullOut,
                        5 => CameraMotion::PanLeft,
                        6 => CameraMotion::TiltDown,
                        7 => CameraMotion::Orbit,
                        _ => CameraMotion::PushIn,
                    };

                    let directed = manifest.asset_tasks.iter().find(|task| {
                        task.task_type == AssetTaskType::AiVideo && task.target == shot.id
                    });
                    let camera_motion = directed
                        .and_then(|task| task.spec["camera"].as_str())
                        .map(CameraMotion::from_str)
                        .unwrap_or(default_camera_motion);
                    let motion_desc = match &shot.prompt_hint {
                        Some(hint) if !hint.trim().is_empty() => hint.clone(),
                        _ => format!(
                            "Continuous camera motion with gentle dynamic presence: {}",
                            shot.description
                        ),
                    };

                    let spec = VideoPromptSpec {
                        shot_id: shot.id.clone(),
                        first_frame_path: Some(first_frame_path),
                        motion_prompt: motion_desc,
                        camera_motion,
                        relationship: VideoGenRelationship::ImageToVideo,
                        duration_seconds: shot.duration_seconds,
                        fps: manifest.meta.fps,
                        width: manifest.meta.width,
                        height: manifest.meta.height,
                    };

                    let req = VideoGenerationRequest {
                        spec,
                        output_path: shot_output,
                        seed: Some(shot.shot_index as u64),
                    };

                    let res = if use_mock {
                        let gen = MockVideoGenerator::new();
                        gen.generate(&req)?
                    } else if flowkit::selected("VIDEO")? {
                        flowkit::video_with_project(&req, flow_project_id.as_deref()).await?
                    } else {
                        let gen = RemoteVideoGenerator::new(
                            api_key_opt.clone().unwrap_or_default(),
                            std::env::var("OPENAI_BASE_URL").ok(),
                            None,
                        );
                        gen.generate(&req).await?
                    };

                    println!(
                        "  [{}/{}] Synthesized {} -> {} ({:.2}s, {} frames, {} ms)",
                        generated_count + 1,
                        manifest.shots.len(),
                        shot.id,
                        res.output_path,
                        res.duration_seconds,
                        res.frames_count,
                        res.elapsed_ms
                    );
                    generated_count += 1;
                }

                println!("------------------------------------------------------------");
                println!(
                    "✅ All {} video clips synthesized and indexed.",
                    generated_count
                );
            }
        },

        Commands::Voice { sub } => match sub {
            VoiceSubcommands::Expand {
                segment_id,
                role,
                spoken,
                direction,
                duration,
                language,
                json,
            } => {
                let spec = VoicePromptSpec {
                    segment_id,
                    role,
                    spoken_text: spoken,
                    voice_direction: direction,
                    language,
                    target_duration_seconds: duration,
                };

                let warnings = audit_voice_spec(&spec)?;

                if json {
                    println!("{}", serde_json::to_string_pretty(&spec)?);
                } else {
                    println!("============================================================");
                    println!(
                        "🎙️ Voice Director Specification for Segment: {}",
                        spec.segment_id
                    );
                    println!("============================================================");
                    println!("Role:       <{}>", spec.role);
                    println!("Spoken:     \"{}\"", spec.spoken_text);
                    println!("Direction:  {}", spec.voice_direction);
                    println!(
                        "Duration:   {:.2}s (Estimated)",
                        spec.target_duration_seconds
                    );
                    println!("Language:   {}", spec.language);
                    println!("------------------------------------------------------------");
                    if warnings.is_empty() {
                        println!("✓ Voice specification audit passed: 0 warnings.");
                    } else {
                        for w in warnings {
                            println!("⚠️ {}", w);
                        }
                    }
                }
            }

            VoiceSubcommands::Synthesize {
                segment_id,
                role,
                spoken,
                direction,
                duration,
                output,
                mock,
                voice,
            } => {
                let spec = VoicePromptSpec {
                    segment_id,
                    role,
                    spoken_text: spoken,
                    voice_direction: direction,
                    language: "en".to_string(),
                    target_duration_seconds: duration,
                };

                let req = VoiceGenerationRequest {
                    spec,
                    output_path: output,
                    voice_preset: voice,
                };

                let res = if mock {
                    MockVoiceGenerator::new().generate(&req).await?
                } else {
                    RemoteVoiceGenerator::from_environment()?
                        .generate(&req)
                        .await?
                };

                println!("✅ Voice speech synthesized successfully!");
                println!("  Segment ID:     {}", res.segment_id);
                println!("  Output:         {}", res.output_path);
                println!("  Metadata:       {}", res.metadata_path);
                println!(
                    "  Duration:       {:.3} s (Physical Measured Extent)",
                    res.duration_seconds
                );
                println!(
                    "  Audio Format:   {} Hz, {} channel(s)",
                    res.sample_rate, res.channels
                );
                println!("  Provider/Model: {} / {}", res.provider, res.model);
                println!("  Elapsed:        {} ms", res.elapsed_ms);
            }

            VoiceSubcommands::Batch {
                voice,
                language,
                script,
                out_dir,
                direction,
                mock,
            } => {
                let svml_content = std::fs::read_to_string(&script).with_context(|| {
                    format!("Failed to read SVML script file from {:?}", script)
                })?;
                let doc = parse_script(&svml_content)?;

                println!("============================================================");
                println!("🎙️ Batch Synthesizing A-roll Voices for Script: {}", doc.id);
                println!(
                    "   Segments: {} | Output: {:?}",
                    doc.segments.len(),
                    out_dir
                );
                println!("============================================================");

                let use_mock = mock;
                let remote = if mock {
                    None
                } else {
                    Some(RemoteVoiceGenerator::from_environment()?)
                };
                println!(
                    "Mode: {}",
                    if mock {
                        "Mock"
                    } else {
                        "Configured TTS provider"
                    }
                );

                let mut count = 0;
                for seg in &doc.segments {
                    if seg.spoken_text.trim().is_empty() {
                        continue;
                    }
                    let out_path = out_dir.join(format!("voice_{}.wav", seg.id));
                    let role = seg
                        .turns
                        .first()
                        .and_then(|t| t.role.clone())
                        .unwrap_or_else(|| "HOST".to_string());

                    let spec = VoicePromptSpec {
                        segment_id: seg.id.clone(),
                        role,
                        spoken_text: seg.spoken_text.clone(),
                        voice_direction: direction.clone(),
                        language: language.clone(),
                        target_duration_seconds: seg.estimated_seconds,
                    };

                    let req = VoiceGenerationRequest {
                        spec,
                        output_path: out_path,
                        voice_preset: voice.clone(),
                    };

                    let res = if use_mock {
                        let gen = MockVoiceGenerator::new();
                        gen.generate(&req).await?
                    } else {
                        remote.as_ref().unwrap().generate(&req).await?
                    };

                    println!(
                        "  [{}/{}] Synthesized {} -> {} (Measured: {:.3}s, Target: {:.2}s, {} ms)",
                        count + 1,
                        doc.segments.len(),
                        seg.id,
                        res.output_path,
                        res.duration_seconds,
                        seg.estimated_seconds,
                        res.elapsed_ms
                    );
                    count += 1;
                }

                println!("------------------------------------------------------------");
                println!("✅ All {} spoken segments synthesized and measured.", count);
            }

            VoiceSubcommands::Realign {
                script,
                project,
                audio_dir,
                out_extents,
                out_project,
                fps,
                gap,
            } => {
                let svml_content = std::fs::read_to_string(&script).with_context(|| {
                    format!("Failed to read SVML script file from {:?}", script)
                })?;
                let doc = parse_script(&svml_content)?;

                let proj_content = std::fs::read_to_string(&project).with_context(|| {
                    format!("Failed to read project manifest from {:?}", project)
                })?;
                let init_manifest: ProjectManifest = serde_json::from_str(&proj_content)?;

                println!("============================================================");
                println!("🔄 Secondary Timeline Re-Alignment (Speech Extents Feedback)");
                println!("============================================================");

                let extents = collect_speech_extents(&audio_dir, &doc).await?;
                println!(
                    "Measured Physical Speech Extents ({} segments found):",
                    extents.len()
                );
                for (seg_id, dur) in &extents {
                    println!("  - {:<12} : {:.3} s (measured)", seg_id, dur);
                }

                if let Some(ref ext_path) = out_extents {
                    if let Some(p) = ext_path.parent() {
                        tokio::fs::create_dir_all(p).await?;
                    }
                    let ext_json = serde_json::to_string_pretty(&extents)?;
                    tokio::fs::write(ext_path, ext_json).await?;
                    println!("Saved measured extents dictionary to: {:?}", ext_path);
                }

                let (_aligned_tl, aligned_manifest) =
                    realign_project_timeline(&doc, &init_manifest, &extents, fps, gap)?;

                println!("------------------------------------------------------------");
                println!("📊 Timeline Elastic Re-Alignment Comparison:");
                println!(
                    "  Total Duration: {:.3}s -> {:.3}s (Delta: {:+.3}s)",
                    init_manifest.meta.total_seconds,
                    aligned_manifest.meta.total_seconds,
                    aligned_manifest.meta.total_seconds - init_manifest.meta.total_seconds
                );
                println!(
                    "  Total Frames:   {} -> {} (at {:.1} fps)",
                    init_manifest.meta.total_frames, aligned_manifest.meta.total_frames, fps
                );

                if let Some(ref out_proj_path) = out_project {
                    if let Some(p) = out_proj_path.parent() {
                        tokio::fs::create_dir_all(p).await?;
                    }
                    let proj_json = serde_json::to_string_pretty(&aligned_manifest)?;
                    tokio::fs::write(out_proj_path, proj_json).await?;
                    println!("Saved realigned project blueprint to: {:?}", out_proj_path);
                }
                println!("✅ Secondary timeline re-alignment completed successfully.");
            }
        },

        Commands::Mix { sub } => {
            match sub {
                MixSubcommands::Plan {
                    project,
                    script,
                    voice_dir,
                    bgm,
                    ducked_gain,
                    normal_gain,
                } => {
                    let proj_content = std::fs::read_to_string(&project).with_context(|| {
                        format!("Failed to read project manifest from {:?}", project)
                    })?;
                    let aligned_manifest: ProjectManifest = serde_json::from_str(&proj_content)?;

                    let script_content = std::fs::read_to_string(&script)
                        .with_context(|| format!("Failed to read SVML script from {:?}", script))?;
                    let doc = parse_script(&script_content)?;

                    println!("============================================================");
                    println!("🎧 Audio Mix Program Plan & Ducking Inspection");
                    println!("============================================================");
                    println!(
                        "Project:  {} ({:.3}s, {} frames at {:.1} fps)",
                        aligned_manifest.meta.title,
                        aligned_manifest.meta.total_seconds,
                        aligned_manifest.meta.total_frames,
                        aligned_manifest.meta.fps,
                    );

                    // 收集 Voice Clips
                    let voice_clips =
                        sound_mix::prepared_voice_clips(&aligned_manifest, &doc, &voice_dir)
                            .await?;
                    println!("Voice Clips Found: {}", voice_clips.len());
                    for vc in &voice_clips {
                        println!(
                            "  - {:<18} : [{:.3}s -> {:.3}s] ({:.3}s)",
                            vc.id,
                            vc.target_start_seconds,
                            vc.target_end_seconds,
                            vc.target_end_seconds - vc.target_start_seconds
                        );
                    }

                    // 收集或规划 SFX
                    let sfx_dir = voice_dir.parent().unwrap_or(Path::new(".")).join("sfx");
                    let sfx_clips = attach_sfx_to_moments(
                        &doc,
                        &aligned_manifest,
                        &sfx_dir,
                        CANONICAL_SAMPLE_RATE,
                    )?;
                    println!("SFX Moment Cues Attached: {}", sfx_clips.len());
                    for sfx in &sfx_clips {
                        println!(
                            "  - {:<28} : @ {:.3}s (gain: {:.2})",
                            sfx.id, sfx.target_start_seconds, sfx.gain
                        );
                    }

                    // 计算 BGM 闪避
                    let duck_config = DuckingConfig {
                        ducked_gain,
                        normal_gain,
                        ..Default::default()
                    };
                    let mut plan = compile_audio_program_plan(
                        &aligned_manifest,
                        voice_clips,
                        bgm,
                        sfx_clips,
                        &duck_config,
                    )?;
                    sound_mix::apply_observed_ducking(&mut plan, &duck_config)?;

                    if let Some(ref bgm_clip) = plan.bgm_clip {
                        println!("Background Music (BGM):");
                        println!("  Source:      {:?}", bgm_clip.source_path);
                        println!(
                            "  Normal Gain: {:.2} | Ducked Gain: {:.2}",
                            duck_config.normal_gain, duck_config.ducked_gain
                        );
                        if let Some(ref env) = bgm_clip.gain_envelope {
                            println!(
                                "  Smart Ducking Envelope Points: {} points calculated",
                                env.len()
                            );
                            for (idx, pt) in env.iter().enumerate().take(8) {
                                println!(
                                    "    Point {:>2}: sample {:>7} ({:.3}s) -> gain {:.2}",
                                    idx,
                                    pt.sample,
                                    pt.sample as f64 / CANONICAL_SAMPLE_RATE as f64,
                                    pt.gain
                                );
                            }
                            if env.len() > 8 {
                                println!("    ... [{} more points]", env.len() - 8);
                            }
                        }
                    }
                }

                MixSubcommands::Sfx {
                    style,
                    output,
                    sample_rate,
                } => {
                    let duration = synthesize_mock_sfx(&style, &output, sample_rate)?;
                    println!("✅ Synthesized mock SFX:");
                    println!("  Style:       {}", style);
                    println!("  Duration:    {:.3} s", duration);
                    println!("  Sample Rate: {} Hz", sample_rate);
                    println!("  Output:      {:?}", output);
                }

                MixSubcommands::Render {
                    mock_sfx,
                    sfx_plan,
                    project,
                    script,
                    voice_dir,
                    bgm,
                    sfx_dir,
                    out_mix,
                    out_project,
                    ducked_gain,
                    normal_gain,
                } => {
                    let proj_content = std::fs::read_to_string(&project).with_context(|| {
                        format!("Failed to read project manifest from {:?}", project)
                    })?;
                    let aligned_manifest: ProjectManifest = serde_json::from_str(&proj_content)?;

                    let script_content = std::fs::read_to_string(&script)
                        .with_context(|| format!("Failed to read SVML script from {:?}", script))?;
                    let doc = parse_script(&script_content)?;

                    println!("============================================================");
                    println!("🎛️ Master Audio Mix & Render Engine");
                    println!("============================================================");
                    println!(
                        "Project:  {} ({:.3}s, {} frames)",
                        aligned_manifest.meta.title,
                        aligned_manifest.meta.total_seconds,
                        aligned_manifest.meta.total_frames
                    );

                    // 1. 扫描 Voice Clips
                    let voice_clips =
                        sound_mix::prepared_voice_clips(&aligned_manifest, &doc, &voice_dir)
                            .await?;
                    println!(
                        "Loaded {} voice clips from {:?}",
                        voice_clips.len(),
                        voice_dir
                    );

                    // 2. 自动提取并吸附 Moment 音效
                    let sfx_clips = if let Some(path) = sfx_plan {
                        let mut clips: Vec<AudioClip> =
                            serde_json::from_slice(&std::fs::read(&path)?)?;
                        for clip in &mut clips {
                            if clip.track_type != AudioTrackType::Sfx {
                                bail!("SFX plan must contain only Sfx clips");
                            }
                            if clip.source_path.is_relative() {
                                clip.source_path = path
                                    .parent()
                                    .unwrap_or(Path::new("."))
                                    .join(&clip.source_path);
                            }
                            if !probe_media(&clip.source_path).await?.has_audio {
                                bail!("Prepared SFX has no audio");
                            }
                        }
                        clips
                    } else {
                        sound_mix::attach_sfx_with_policy(
                            &doc,
                            &aligned_manifest,
                            &sfx_dir,
                            CANONICAL_SAMPLE_RATE,
                            mock_sfx,
                        )?
                    };
                    println!(
                        "Synthesized and placed {} SFX moment cues to {:?}",
                        sfx_clips.len(),
                        sfx_dir
                    );

                    // 3. 决定 BGM 来源 (若未指定，优先寻找 fixtures/soundtrack.m4a)
                    let resolved_bgm = match bgm {
                        Some(b) => {
                            if !b.is_file() {
                                bail!("Explicit BGM source does not exist: {:?}", b);
                            }
                            Some(b)
                        }
                        None => {
                            let fallback = PathBuf::from("fixtures/soundtrack.m4a");
                            if fallback.exists() {
                                Some(fallback)
                            } else {
                                None
                            }
                        }
                    };

                    let duck_config = DuckingConfig {
                        ducked_gain,
                        normal_gain,
                        ..Default::default()
                    };

                    // 4. 组装混音方案并生成智能闪避包络线
                    let mut plan = compile_audio_program_plan(
                        &aligned_manifest,
                        voice_clips,
                        resolved_bgm,
                        sfx_clips,
                        &duck_config,
                    )?;
                    sound_mix::apply_observed_ducking(&mut plan, &duck_config)?;
                    if plan.bgm_clip.is_some() {
                        println!("BGM Track Active with Smart Audio Ducking (Normal: {:.2}, Ducked: {:.2})", normal_gain, ducked_gain);
                    }

                    // 5. 调用 FFmpeg 多轨压制
                    println!("Executing FFmpeg master audio rendering pipeline...");
                    render_master_mix(&plan, &out_mix).await?;
                    println!("✅ Master mix WAV rendered: {:?}", out_mix);

                    // 6. 更新并持久化项目蓝图
                    let mixed_manifest =
                        update_manifest_with_mixed_tracks(&aligned_manifest, &plan, &out_mix)?;
                    let out_proj_path = out_project.unwrap_or_else(|| {
                        project
                            .parent()
                            .unwrap_or(Path::new("."))
                            .join("project_mixed.json")
                    });
                    if let Some(p) = out_proj_path.parent() {
                        tokio::fs::create_dir_all(p).await?;
                    }
                    let proj_json = serde_json::to_string_pretty(&mixed_manifest)?;
                    tokio::fs::write(&out_proj_path, proj_json).await?;
                    println!("Saved updated multi-track manifest to: {:?}", out_proj_path);
                }
            }
        }

        Commands::Caption { sub } => {
            match sub {
                CaptionSubcommands::ParseDoc { script, to, json } => {
                    let script_text = tokio::fs::read_to_string(&script)
                        .await
                        .with_context(|| format!("Failed to read script file: {:?}", script))?;
                    let script_doc = parse_script(&script_text)?;
                    let cap_doc = build_caption_document_from_script(&script_doc)?;

                    if !json {
                        println!("✅ Parsed CaptionDocument successfully:");
                        println!("  Document ID:  {}", cap_doc.id);
                        println!("  Total Words:  {}", cap_doc.words.len());
                        println!("  Total Units:  {}", cap_doc.units.len());
                        println!("  Authored Cues: {}", cap_doc.cues.len());
                    }

                    let serialized = serde_json::to_string_pretty(&cap_doc)?;
                    if let Some(target) = to {
                        if let Some(parent) = target.parent() {
                            tokio::fs::create_dir_all(parent).await?;
                        }
                        tokio::fs::write(&target, &serialized).await?;
                        if !json {
                            println!("Saved CaptionDocument JSON to: {:?}", target);
                        }
                    }
                    if json {
                        println!("{}", serialized);
                    }
                }

                CaptionSubcommands::Schedule {
                    script,
                    doc,
                    project,
                    transcript,
                    intervals,
                    recipe,
                    observations,
                    track_head,
                    target_role,
                    to,
                    to_vtt,
                    to_ass,
                    json,
                } => {
                    // 1. 获取 CaptionDocument
                    let cap_doc = if let Some(doc_path) = doc {
                        let text = tokio::fs::read_to_string(&doc_path).await?;
                        serde_json::from_str::<CaptionDocument>(&text)?
                    } else if let Some(script_path) = script {
                        let script_text = tokio::fs::read_to_string(&script_path).await?;
                        let script_doc = parse_script(&script_text)?;
                        build_caption_document_from_script(&script_doc)?
                    } else {
                        bail!("Either --script or --doc must be specified for caption scheduling");
                    };

                    // 2. 获取样式配方
                    let caption_recipe = if let Some(rec_path) = recipe {
                        let text = tokio::fs::read_to_string(&rec_path).await?;
                        serde_json::from_str::<CaptionRecipe>(&text)?
                    } else {
                        CaptionRecipe::default()
                    };

                    // 3. 计算对齐时序 (优先词级转录，次之 speech intervals)
                    let timing = if let Some(trans_path) = transcript {
                        let text = tokio::fs::read_to_string(&trans_path).await?;
                        let trans_doc: TranscriptDocument = serde_json::from_str(&text)?;
                        build_caption_timing_from_transcript(&cap_doc, &trans_doc)?
                    } else {
                        let speech_intervals = if let Some(int_path) = intervals {
                            let text = tokio::fs::read_to_string(&int_path).await?;
                            serde_json::from_str::<Vec<(f64, f64)>>(&text)?
                        } else if let Some(proj_path) = project {
                            let text = tokio::fs::read_to_string(&proj_path).await?;
                            let manifest: ProjectManifest = serde_json::from_str(&text)?;

                            let mut collected = Vec::new();
                            for item in &manifest.tracks.voice {
                                collected.push((item.start_seconds, item.end_seconds));
                            }

                            if collected.is_empty() {
                                for shot in &manifest.shots {
                                    collected.push((shot.start_seconds, shot.end_seconds));
                                }
                            }
                            collected
                        } else {
                            let total = 10.6;
                            let n = cap_doc.cues.len().max(1);
                            let step = total / n as f64;
                            (0..n)
                                .map(|i| (i as f64 * step, (i + 1) as f64 * step))
                                .collect()
                        };

                        build_caption_timing_from_speech_intervals(&cap_doc, &speech_intervals)?
                    };

                    let mut schedule = schedule_caption(&cap_doc, &timing, &caption_recipe)?;

                    // 5. 若启用人头追踪，解析并挂载追踪锚点
                    if track_head {
                        let obs_list = if let Some(obs_path) = observations {
                            let text = tokio::fs::read_to_string(&obs_path).await?;
                            serde_json::from_str::<Vec<FaceObservation>>(&text)?
                        } else {
                            // 若未指定观测文件但开启了人头追踪，基于默认画面中央轻微起伏生成平滑观测证据
                            let mut synthetic = Vec::new();
                            for cue in &schedule.cues {
                                let mid_t = (cue.timed_start + cue.timed_end) / 2.0;
                                synthetic.push(FaceObservation {
                                    timestamp: mid_t,
                                    role: target_role.clone(),
                                    box_normalized: [0.38, 0.22, 0.24, 0.24],
                                });
                            }
                            synthetic
                        };

                        let track_cfg = HeadTrackingConfig {
                            enabled: true,
                            target_role: target_role.clone(),
                            ..Default::default()
                        };
                        apply_head_tracking(&mut schedule, &obs_list, &track_cfg);
                    }

                    if !json {
                        println!("✅ Scheduled Caption successfully:");
                        println!("  Timeline:      {}", schedule.timeline_id);
                        println!("  Recipe:        {}", schedule.recipe_id);
                        println!("  Scheduled Cues: {}", schedule.cues.len());
                        let tracked_count = schedule
                            .cues
                            .iter()
                            .filter(|c| c.tracking_anchor.is_some())
                            .count();
                        println!(
                            "  Head Tracked:  {} / {}",
                            tracked_count,
                            schedule.cues.len()
                        );

                        for (idx, cue) in schedule.cues.iter().enumerate() {
                            let track_str = if let Some((x, y)) = cue.tracking_anchor {
                                format!(" [Head @ ({:.2}, {:.2})]", x, y)
                            } else {
                                "".into()
                            };
                            println!(
                            "  [{:02}] {:<6} timed: [{:5.2}s ~ {:5.2}s] visible: [{:5.2}s ~ {:5.2}s]{} | \"{}\"",
                            idx + 1,
                            cue.role,
                            cue.timed_start,
                            cue.timed_end,
                            cue.visible_start,
                            cue.visible_end,
                            track_str,
                            cue.display_text.trim(),
                        );
                        }
                    }

                    let serialized = serde_json::to_string_pretty(&schedule)?;
                    if let Some(target) = to {
                        if let Some(parent) = target.parent() {
                            tokio::fs::create_dir_all(parent).await?;
                        }
                        tokio::fs::write(&target, &serialized).await?;
                        if !json {
                            println!("Saved CaptionSchedule JSON to: {:?}", target);
                        }
                    }

                    if let Some(vtt_target) = to_vtt {
                        if let Some(parent) = vtt_target.parent() {
                            tokio::fs::create_dir_all(parent).await?;
                        }
                        let vtt_str = export_to_webvtt(&schedule);
                        tokio::fs::write(&vtt_target, vtt_str).await?;
                        if !json {
                            println!("Exported WebVTT subtitles to: {:?}", vtt_target);
                        }
                    }

                    if let Some(ass_target) = to_ass {
                        if let Some(parent) = ass_target.parent() {
                            tokio::fs::create_dir_all(parent).await?;
                        }
                        let ass_str = export_to_ass(&schedule, &caption_recipe, 640, 360);
                        tokio::fs::write(&ass_target, ass_str).await?;
                        if !json {
                            println!("Exported ASS subtitles to: {:?}", ass_target);
                        }
                    }

                    if json {
                        println!("{}", serialized);
                    }
                }

                CaptionSubcommands::Export {
                    schedule,
                    format,
                    width,
                    height,
                    recipe,
                    to,
                } => {
                    let sched_text = tokio::fs::read_to_string(&schedule).await?;
                    let sched: CaptionSchedule = serde_json::from_str(&sched_text)?;

                    let caption_recipe = if let Some(rec_path) = recipe {
                        let text = tokio::fs::read_to_string(&rec_path).await?;
                        serde_json::from_str::<CaptionRecipe>(&text)?
                    } else {
                        CaptionRecipe::default()
                    };

                    let rendered = match format.to_lowercase().as_str() {
                        "ass" => export_to_ass(&sched, &caption_recipe, width, height),
                        "vtt" | "webvtt" => export_to_webvtt(&sched),
                        "html" => export_to_html_overlay(&sched, &caption_recipe, width, height),
                        "json" => serde_json::to_string_pretty(&sched)?,
                        other => bail!(
                            "Unsupported export format: {}. Use ass, vtt, html, or json",
                            other
                        ),
                    };

                    if let Some(target) = to {
                        if let Some(parent) = target.parent() {
                            tokio::fs::create_dir_all(parent).await?;
                        }
                        tokio::fs::write(&target, &rendered).await?;
                        println!(
                            "✅ Exported {} captions to {:?}",
                            format.to_uppercase(),
                            target
                        );
                    } else {
                        println!("{}", rendered);
                    }
                }

                CaptionSubcommands::Query {
                    schedule,
                    time,
                    width,
                    height,
                    recipe,
                } => {
                    let sched_text = tokio::fs::read_to_string(&schedule).await?;
                    let sched: CaptionSchedule = serde_json::from_str(&sched_text)?;

                    let caption_recipe = if let Some(rec_path) = recipe {
                        let text = tokio::fs::read_to_string(&rec_path).await?;
                        serde_json::from_str::<CaptionRecipe>(&text)?
                    } else {
                        CaptionRecipe::default()
                    };

                    match query_caption_state_at(&sched, &caption_recipe, time, width, height) {
                        Some(snapshot) => {
                            println!("=== Caption State Query at {:.3}s ===", time);
                            println!("  Cue ID:       {}", snapshot.cue_id);
                            println!("  Role:         {}", snapshot.role);
                            println!("  Display Text: \"{}\"", snapshot.display_text.trim());
                            println!(
                                "  Active Word:  {}",
                                snapshot.active_word_text.as_deref().unwrap_or("<none>")
                            );
                            println!(
                                "  Anchor Pos:   ({:.1}px, {:.1}px)",
                                snapshot.position_px.0, snapshot.position_px.1
                            );
                            println!("  Head Tracked: {}", snapshot.is_tracking_head);
                        }
                        None => {
                            println!("No subtitle active at timestamp {:.3}s (Screen is silent / in gap)", time);
                        }
                    }
                }
            }
        }

        Commands::Component { sub } => match sub {
            ComponentSubcommands::ListArchetypes => {
                println!("=== Available Web-as-Video Component Archetypes ===");
                println!("1. task_engine:       Five tasks converging into a central rotating engine ring");
                println!("2. timeline_compare:  Dual-lane timeline compare (speech extents vs elastic timeline)");
                println!("3. code_artifact:     IDE/Terminal window with syntax-highlighted code and test badge");
                println!(
                    "4. feature_metric:    High-impact multi-column metric stats with pulse scale"
                );
                println!("5. editorial_banner:  Magazine-style headline banner with action button");
            }

            ComponentSubcommands::Design {
                archetype,
                title,
                props,
                width,
                height,
                duration_frames,
                fps,
                to,
            } => {
                let arch = match archetype.to_lowercase().as_str() {
                    "task_engine" | "task-engine" | "task" => ComponentArchetype::TaskEngine,
                    "timeline_compare" | "timeline-compare" | "timeline" => ComponentArchetype::TimelineCompare,
                    "code_artifact" | "code-artifact" | "code" => ComponentArchetype::CodeArtifact,
                    "feature_metric" | "feature-metric" | "metric" => ComponentArchetype::FeatureMetric,
                    "editorial_banner" | "editorial-banner" | "editorial" | "banner" => ComponentArchetype::EditorialBanner,
                    other => bail!("Unknown archetype: '{}'. Run 'component list-archetypes' to view supported types.", other),
                };

                let parsed_props: serde_json::Value = match props {
                    Some(raw) => {
                        if Path::new(&raw).exists() {
                            let text = tokio::fs::read_to_string(&raw).await?;
                            serde_json::from_str(&text)?
                        } else {
                            serde_json::from_str(&raw)?
                        }
                    }
                    None => serde_json::json!({}),
                };

                let spec = VisualComponentSpec {
                    id: format!("comp_{}", archetype),
                    archetype: arch,
                    title,
                    bounds: SpatialBounds {
                        width_px: width,
                        height_px: height,
                        padding_px: 24,
                    },
                    theme: ComponentTheme::default(),
                    animation: ComponentAnimation {
                        duration_frames,
                        fps,
                        entrance_frames: 10,
                        exit_frames: 8,
                        motion_type: MotionType::Progressive,
                    },
                    props: parsed_props,
                };

                let rendered_html = render_component_to_html(&spec)?;

                if let Some(target) = to {
                    if let Some(parent) = target.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    tokio::fs::write(&target, &rendered_html).await?;
                    println!("✅ Designed Web-as-Video component: {:?}", target);
                    println!("  Archetype: {:?}", spec.archetype);
                    println!(
                        "  Bounds:    {}x{} px",
                        spec.bounds.width_px, spec.bounds.height_px
                    );
                    println!(
                        "  Frames:    {} (fps: {})",
                        spec.animation.duration_frames, spec.animation.fps
                    );
                } else {
                    println!("{}", rendered_html);
                }
            }

            ComponentSubcommands::AutoDesign {
                project,
                script,
                out_dir,
                update_project,
            } => {
                let proj_text = tokio::fs::read_to_string(&project)
                    .await
                    .with_context(|| format!("Failed to read project manifest: {:?}", project))?;
                let manifest: ProjectManifest = serde_json::from_str(&proj_text)?;

                let script_text = tokio::fs::read_to_string(&script)
                    .await
                    .with_context(|| format!("Failed to read script: {:?}", script))?;
                let script_doc = parse_script(&script_text)?;

                println!("Analyzing project shots and script semantics for Web-as-Video component synthesis...");
                let specs = auto_design_components_for_project(&manifest, &script_doc)?;

                tokio::fs::create_dir_all(&out_dir).await?;

                println!(
                    "Synthesizing {} visual components into {:?}:",
                    specs.len(),
                    out_dir
                );
                for spec in &specs {
                    let html = render_component_to_html(spec)?;
                    let file_name = format!("{}.html", spec.id);
                    let target_path = out_dir.join(&file_name);
                    tokio::fs::write(&target_path, html).await?;
                    println!(
                        "  [{}] {:<18} -> {:?}",
                        spec.id,
                        format!("{:?}", spec.archetype),
                        target_path
                    );
                }

                let updated_manifest =
                    update_manifest_with_visual_components(&manifest, &specs, &out_dir)?;

                if let Some(out_p) = update_project {
                    if let Some(parent) = out_p.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    let updated_json = serde_json::to_string_pretty(&updated_manifest)?;
                    tokio::fs::write(&out_p, updated_json).await?;
                    println!(
                        "✅ Saved updated manifest with visual_components track to: {:?}",
                        out_p
                    );
                }
            }

            ComponentSubcommands::Inspect { input } => {
                let content = tokio::fs::read_to_string(&input)
                    .await
                    .with_context(|| format!("Failed to read component file: {:?}", input))?;

                println!("=== Web-as-Video Component Inspection: {:?} ===", input);
                let has_doctype = content.contains("<!doctype html>");
                let has_spec = content.contains("window.__COMPONENT_SPEC__");
                let has_render_frame = content.contains("window.renderFrame");
                let has_setup = content.contains("window.setup");

                println!(
                    "  DOCTYPE HTML:      {}",
                    if has_doctype {
                        "✅ Valid"
                    } else {
                        "❌ Missing"
                    }
                );
                println!(
                    "  Component Spec:    {}",
                    if has_spec { "✅ Found" } else { "❌ Missing" }
                );
                println!(
                    "  renderFrame Hook:  {}",
                    if has_render_frame {
                        "✅ Exported"
                    } else {
                        "❌ Missing"
                    }
                );
                println!(
                    "  setup Hook:        {}",
                    if has_setup {
                        "✅ Exported"
                    } else {
                        "❌ Missing"
                    }
                );
                println!("  Byte Size:         {} bytes", content.len());

                if has_spec && has_render_frame {
                    println!("✅ Component satisfies Web-as-Video deterministic frame capture contract.");
                } else {
                    println!(
                        "⚠️ Component lacks required runtime hooks for deterministic capture."
                    );
                }
            }
        },

        Commands::Capture { sub } => match sub {
            CaptureSubcommands::Detect => {
                println!("=== Detecting Headless Browser Environment ===");
                let detected = detect_chrome_executable();
                match detected {
                    Some(path) => {
                        println!("✅ Detected Chrome/Chromium executable at: {:?}", path);
                        println!("   Provider status: Headless ready");
                    }
                    None => {
                        println!("⚠️ No Google Chrome / Chromium executable detected in standard locations or PATH.");
                        println!(
                            "   Provider status: Falling back to pure Rust Mock image synthesis."
                        );
                    }
                }
            }

            CaptureSubcommands::Screenshot {
                input,
                frame,
                total_frames,
                width,
                height,
                to,
                mock,
            } => {
                let provider = resolve_browser_provider(mock)?;
                let config = CaptureConfig {
                    width,
                    height,
                    ..Default::default()
                };

                println!(
                    "Capturing single frame {}/{} from {:?} using {:?}...",
                    frame, total_frames, input, provider
                );
                capture_single_frame(&input, frame, total_frames, &config, &to, &provider).await?;
                println!("✅ Captured screenshot to: {:?}", to);
            }

            CaptureSubcommands::Sequence {
                input,
                start_frame,
                end_frame,
                width,
                height,
                out_dir,
                mock,
            } => {
                let provider = resolve_browser_provider(mock)?;
                let config = CaptureConfig {
                    width,
                    height,
                    ..Default::default()
                };

                println!(
                    "Capturing frame sequence [{}..{}) from {:?} into {:?} using {:?}...",
                    start_frame, end_frame, input, out_dir, provider
                );
                let paths = capture_frame_sequence(
                    &input,
                    start_frame,
                    end_frame,
                    &config,
                    &out_dir,
                    &provider,
                )
                .await?;
                println!(
                    "✅ Successfully captured {} frames into: {:?}",
                    paths.len(),
                    out_dir
                );
            }

            CaptureSubcommands::Record {
                input,
                total_frames,
                fps,
                width,
                height,
                crf,
                transparent,
                to,
                mock,
            } => {
                let provider = resolve_browser_provider(mock)?;
                let config = CaptureConfig {
                    width,
                    height,
                    fps,
                    crf,
                    transparent,
                    ..Default::default()
                };

                println!(
                    "Recording component {:?} to MP4 ({} frames @ {:.1} fps, CRF {}) using {:?}...",
                    input, total_frames, fps, crf, provider
                );
                render_html_to_mp4(&input, total_frames, &config, &to, &provider).await?;
                println!(
                    "✅ Successfully encoded broadcast-grade MP4 video to: {:?}",
                    to
                );
            }

            CaptureSubcommands::Batch {
                project,
                components_dir,
                out_dir,
                fps,
                width,
                height,
                update_project,
                transparent,
                mock,
            } => {
                let manifest_text = tokio::fs::read_to_string(&project)
                    .await
                    .with_context(|| format!("Failed to read project manifest: {:?}", project))?;
                let mut manifest: ProjectManifest = serde_json::from_str(&manifest_text)
                    .with_context(|| format!("Failed to parse project manifest: {:?}", project))?;

                // 若指定了 components_dir，优先以此目录下的组件 HTML 路径覆盖清单
                if let Some(ref cdir) = components_dir {
                    for item in &mut manifest.tracks.visual_components {
                        if let Some(obj) = item.payload.as_object_mut() {
                            let comp_id = obj
                                .get("component_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or(&item.id);
                            let target_html = cdir.join(format!("{}.html", comp_id));
                            if target_html.exists() {
                                obj.insert(
                                    "file_path".into(),
                                    serde_json::Value::String(
                                        target_html.to_string_lossy().to_string(),
                                    ),
                                );
                            }
                        }
                    }
                }

                let provider = resolve_browser_provider(mock)?;
                let config = CaptureConfig {
                    width,
                    height,
                    fps,
                    transparent,
                    ..Default::default()
                };

                println!(
                    "Batch capturing visual components for project {:?} into {:?} using {:?}...",
                    manifest.meta.id, out_dir, provider
                );
                let updated_manifest =
                    batch_capture_project_components(&manifest, &out_dir, &config, &provider)
                        .await?;

                if let Some(out_p) = update_project {
                    if let Some(parent) = out_p.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    let updated_json = serde_json::to_string_pretty(&updated_manifest)?;
                    tokio::fs::write(&out_p, updated_json).await?;
                    println!(
                        "✅ Saved updated project manifest with captured video paths to: {:?}",
                        out_p
                    );
                }
            }
        },

        Commands::Compose { sub } => match sub {
            ComposeSubcommands::Plan {
                subtitle_delivery,
                project,
                project_dir,
                output,
            } => {
                let manifest_text = tokio::fs::read_to_string(&project)
                    .await
                    .with_context(|| format!("Failed to read project manifest: {:?}", project))?;
                let manifest: ProjectManifest = serde_json::from_str(&manifest_text)
                    .with_context(|| format!("Failed to parse project manifest: {:?}", project))?;

                let root_dir = project_dir.unwrap_or_else(|| {
                    project
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| PathBuf::from("."))
                });

                println!("=== Compiling Hyperframes Composite Plan ===");
                let mut plan = compile_project_to_composite_plan(&manifest, &root_dir, &output)?;
                plan.subtitle_delivery = match subtitle_delivery.as_str() {
                    "soft" => compositor::SubtitleDelivery::Soft,
                    "none" => compositor::SubtitleDelivery::None,
                    _ => compositor::SubtitleDelivery::BurnIn,
                };

                println!(
                    "Canvas:        {}x{} @ {:.1} fps (total {:.2}s / {} frames)",
                    plan.canvas.width,
                    plan.canvas.height,
                    plan.canvas.fps,
                    plan.canvas.total_seconds,
                    plan.canvas.total_frames
                );
                println!(
                    "Background:    {} shot segments (Color: {})",
                    plan.bg_segments.len(),
                    plan.canvas.background_color
                );
                for seg in &plan.bg_segments {
                    println!(
                        "  - [{}] {:.2}s..{:.2}s ({}) -> {:?}",
                        seg.id,
                        seg.start_seconds,
                        seg.start_seconds + seg.duration_seconds,
                        if seg.is_still { "still loop" } else { "video" },
                        seg.source_path
                    );
                }
                println!(
                    "Overlays:      {} visual component layers",
                    plan.overlays.len()
                );
                for ov in &plan.overlays {
                    println!(
                        "  - [{}] {:.2}s..{:.2}s -> {:?}",
                        ov.id, ov.start_seconds, ov.end_seconds, ov.source_path
                    );
                }
                println!(
                    "Subtitles:     {}",
                    if let Some(sub) = &plan.subtitle {
                        format!("{:?} ({:?})", sub.path, sub.format)
                    } else {
                        "None".into()
                    }
                );
                println!(
                    "Audio Master:  {}",
                    if let Some(aud) = &plan.audio {
                        format!(
                            "{:?} ({}Hz / {}ch / {})",
                            aud.path, aud.sample_rate, aud.channels, aud.bitrate
                        )
                    } else {
                        "None".into()
                    }
                );
                println!("Output Target: {:?}", plan.output_path);

                let compiled = compile_composite_filtergraph(&plan)?;
                println!("\n=== Compiled FFmpeg Filter Complex ===");
                println!("{}", compiled.filter_complex);
            }

            ComposeSubcommands::Render {
                subtitle_delivery,
                project,
                project_dir,
                output,
                mock,
            } => {
                let manifest_text = tokio::fs::read_to_string(&project)
                    .await
                    .with_context(|| format!("Failed to read project manifest: {:?}", project))?;
                let manifest: ProjectManifest = serde_json::from_str(&manifest_text)
                    .with_context(|| format!("Failed to parse project manifest: {:?}", project))?;

                let root_dir = project_dir.unwrap_or_else(|| {
                    project
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| PathBuf::from("."))
                });

                println!("Compiling composition plan from {:?}...", project);
                let mut plan = compile_project_to_composite_plan(&manifest, &root_dir, &output)?;
                plan.subtitle_delivery = match subtitle_delivery.as_str() {
                    "soft" => compositor::SubtitleDelivery::Soft,
                    "none" => compositor::SubtitleDelivery::None,
                    _ => compositor::SubtitleDelivery::BurnIn,
                };

                println!(
                    "Executing Hyperframes offline multi-layer rendering into {:?} (mock: {})...",
                    plan.output_path, mock
                );
                render_composition(&plan, mock).await?;
                println!(
                    "✅ Successfully rendered broadcast master video to: {:?}",
                    plan.output_path
                );
            }

            ComposeSubcommands::BurnSubtitles {
                subtitle_delivery,
                input,
                subtitles,
                output,
            } => {
                if let Some(parent) = output.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }

                let has_filter = compositor::check_ffmpeg_has_subtitles_filter();
                let mut cmd = tokio::process::Command::new("ffmpeg");
                cmd.arg("-y").arg("-v").arg("error");

                if subtitle_delivery == "burn-in" && !has_filter {
                    bail!("Burn-in requires libass; use caption render or explicitly choose --subtitle-delivery soft");
                }
                if subtitle_delivery == "burn-in" {
                    println!(
                        "Burning hard subtitles {:?} onto {:?} -> {:?}...",
                        subtitles, input, output
                    );
                    let sub_escaped = compositor::escape_ffmpeg_filter_path(&subtitles);
                    let filter = format!(
                        "subtitles=filename='{}',scale=out_color_matrix=bt709:out_range=tv,format=yuv420p,setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv",
                        sub_escaped
                    );
                    cmd.arg("-i")
                        .arg(input.to_string_lossy().to_string())
                        .arg("-vf")
                        .arg(&filter)
                        .arg("-c:a")
                        .arg("copy")
                        .arg("-c:v")
                        .arg("libx264")
                        .arg("-crf")
                        .arg("21")
                        .arg("-preset")
                        .arg("veryfast");
                } else {
                    println!("FFmpeg lacks libass filter; soft-muxing subtitles {:?} into {:?} -> {:?}...", subtitles, input, output);
                    let vtt_candidate =
                        if subtitles.extension().map(|e| e == "vtt").unwrap_or(false) {
                            subtitles.clone()
                        } else {
                            let vtt_sibling = subtitles.with_extension("vtt");
                            if vtt_sibling.exists() {
                                vtt_sibling
                            } else {
                                subtitles.clone()
                            }
                        };

                    cmd.arg("-i")
                        .arg(input.to_string_lossy().to_string())
                        .arg("-i")
                        .arg(vtt_candidate.to_string_lossy().to_string())
                        .arg("-c:v")
                        .arg("copy")
                        .arg("-c:a")
                        .arg("copy")
                        .arg("-c:s")
                        .arg("mov_text")
                        .arg("-metadata:s:s:0")
                        .arg("language=eng");
                }

                cmd.arg("-movflags")
                    .arg("+faststart")
                    .arg(output.to_string_lossy().to_string());

                let status = cmd
                    .status()
                    .await
                    .context("Failed executing FFmpeg subtitle burn/mux")?;
                if !status.success() || !output.exists() {
                    bail!("Failed to burn/mux subtitles to {:?}", output);
                }
                println!("✅ Subtitles successfully integrated into: {:?}", output);
            }
        },

        Commands::Media { sub } => production::run(sub).await?,
        Commands::AdmitMedia { asset } => {
            let meta = probe_media(&asset).await?;
            if !meta.has_audio && meta.width.is_none() {
                bail!("Not a usable media asset: {:?}", asset);
            }
            artifacts::admit_supplied(&asset)?;
            println!("Registered supplied production media: {:?}", asset);
        }

        Commands::Review {
            project,
            video,
            mode,
            json,
            evidence,
            require_delivery,
        } => {
            let manifest_text = tokio::fs::read_to_string(&project)
                .await
                .with_context(|| format!("Failed to read project manifest: {:?}", project))?;
            let manifest: ProjectManifest = serde_json::from_str(&manifest_text)
                .with_context(|| format!("Failed to parse project manifest: {:?}", project))?;

            let config = ReviewConfig {
                expected_width: manifest.meta.width,
                expected_height: manifest.meta.height,
                expected_fps: manifest.meta.fps,
                mode,
                evidence,
                ..Default::default()
            };

            let report = review_project_and_master(&manifest, &video, &config).await?;

            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("=== Multi-Modal Video Quality Review Report ===");
                println!("Deliverable:   {:?}", video);
                println!("Project:       {:?} ({})", project, manifest.meta.title);
                println!(
                    "Artifact Mode: {}",
                    report.mode.as_deref().unwrap_or("unknown")
                );
                println!("Overall Score: {:.1} / 100.0", report.score);
                println!("Ready for delivery: {}", report.ready_for_delivery);
                println!(
                    "Verdict:       {}",
                    if report.passed {
                        "✅ PASSED (technical checks)"
                    } else {
                        "❌ REVISION REQUIRED"
                    }
                );
                println!("\nDetailed Audit Criteria:");
                for (i, c) in report.criteria.iter().enumerate() {
                    let mark = match c.status {
                        ReviewStatus::Pass => "✅ PASS",
                        ReviewStatus::Warning => "⚠️ WARN",
                        ReviewStatus::Fail => "❌ FAIL",
                        ReviewStatus::Unverified => "❓ UNVERIFIED",
                    };
                    println!("  {:02}. [{}] {:<28} {}", i + 1, mark, c.name, c.message);
                    if let Some(fact) = &c.owning_fact {
                        println!("      ↳ Owning Fact Repair: {}", fact);
                    }
                }

                if !report.recommendations.is_empty() {
                    println!("\nRecommended Action Items:");
                    for (i, rec) in report.recommendations.iter().enumerate() {
                        println!("  {}. {}", i + 1, rec);
                    }
                }
            }

            if !report.passed || (require_delivery && !report.ready_for_delivery) {
                std::process::exit(1);
            }
        }

        Commands::Agent {
            reference,
            project_dir,
            output,
            auto_heal,
            mock,
        } => {
            let options = AgentOptions {
                reference_video: reference,
                project_dir,
                output_video: output,
                auto_heal,
                mock,
            };

            let res = run_autonomous_video_agent(&options).await?;
            if !res.success {
                bail!("Autonomous video agent execution concluded with critical quality failures");
            }
        }

        Commands::Plan { source, mock, to } => {
            let plan = production_graph::compile(&source, mock)?;
            let json = serde_json::to_string_pretty(&plan)?;
            if let Some(path) = to {
                if path.exists() {
                    let destination = path.canonicalize()?;
                    let mut sources = vec![plan.run.clone(), plan.author.clone()];
                    let selection = production_graph::source::read_run(&plan.run)?;
                    sources.extend(selection.reuse);
                    for candidate in selection.candidates.values() {
                        match candidate {
                            production_graph::model::Candidate::File { path, .. } => {
                                sources.push(path.clone())
                            }
                            production_graph::model::Candidate::Result { result, .. } => {
                                sources.push(result.clone())
                            }
                        }
                    }
                    sources.extend(plan.provided.values().map(|a| a.path.clone()));
                    for task in &plan.tasks {
                        for input in task.inputs.values() {
                            if let production_graph::model::PlannedInput::Material { material } =
                                input
                            {
                                sources.push(material.artifact.path.clone());
                            }
                        }
                        if let Some(cached) = &task.cached {
                            sources.extend(cached.outputs.values().map(|a| a.path.clone()));
                        }
                    }
                    if sources
                        .iter()
                        .any(|source| source.canonicalize().is_ok_and(|p| p == destination))
                    {
                        bail!("Plan output cannot overwrite source inputs");
                    }
                }
                if let Some(parent) = path.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent)?;
                    }
                }
                std::fs::write(path, &json)?;
            }
            println!("{}", json);
            if !plan.blocked.is_empty() {
                bail!("Production plan is blocked; inspect blocked reasons");
            }
        }
        Commands::Result { path } => println!(
            "{}",
            serde_json::to_string_pretty(&production_graph::inspect(&path)?)?
        ),
        Commands::Build {
            prepared_project,
            source,
            project_dir,
            output,
            auto_heal,
            mock,
        } => {
            if production_graph::source::graph_run(&source)? {
                if prepared_project.is_some() {
                    bail!("Production Run owns its project dependencies; --prepared-project is for legacy SVML builds");
                }
                let result = production_graph::build(
                    &source,
                    project_dir.as_deref(),
                    output.as_deref(),
                    mock,
                )
                .await?;
                println!("Production Result: {}", result.display());
                return Ok(());
            }
            let p_dir = project_dir.unwrap_or_else(|| {
                prepared_project
                    .as_ref()
                    .unwrap_or(&source)
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| PathBuf::from("output/target_project"))
            });
            let out_vid = output.unwrap_or_else(|| p_dir.join("master.mp4"));
            let options = video_agent::BuildOptions {
                prepared_project,
                source,
                project_dir: p_dir,
                output_video: out_vid,
                auto_heal,
                mock,
            };

            let res = video_agent::build_svml_project(&options).await?;
            if !res.success {
                bail!("Video build concluded with critical quality failures");
            }
        }
    }

    Ok(())
}
