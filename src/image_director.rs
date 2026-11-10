use anyhow::{bail, Context, Result};
use image::{ImageBuffer, Rgb, RgbImage};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::Instant;

/// 推荐的标准 iPhone 实拍质感摄影 Prompt (Capture 职责段)
pub const DEFAULT_PHONE_UGC_CAPTURE: &str = "\
A photograph captured as a single frame from a video actually shot on an iPhone, \
with the texture of real iPhone footage. The image looks real, without an oily, \
overprocessed finish. The background is clearly visible, with no depth-of-field blur. \
Skin texture is natural and fine, and the lighting is natural. \
The image is coherent and free of visual artifacts.";

/// 商业静物/产品摄影质感 Prompt (Studio Product Capture)
#[allow(dead_code)]
pub const STUDIO_PRODUCT_CAPTURE: &str = "\
A high-end commercial studio product photograph with immaculate lighting, \
ultra-sharp optical fidelity, natural specular highlights, realistic material reflections \
and clean tactile textures. The image looks authentic, premium, coherent, and free of visual artifacts.";

/// 35mm 电影胶片质感 Prompt (Cinematic 35mm Capture)
#[allow(dead_code)]
pub const CINEMATIC_35MM_CAPTURE: &str = "\
A cinematic 35mm film still, organic film grain, rich tonal roll-off, \
natural color grading, nuanced ambient lighting, coherent depth and authentic camera texture. \
Free of synthetic digital smoothing or visual artifacts.";

fn default_aspect_ratio() -> String {
    "9:16".to_string()
}

fn default_resolution() -> String {
    "1K".to_string()
}

/// 四段式分镜视觉提示词规格 (Image Prompt Specification)
/// 严格遵循四段式图像职责规范: Four paragraphs, four responsibilities
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImagePromptSpec {
    /// 第一段：摄影语言与画质基调 (Capture)
    #[serde(default)]
    pub capture: Option<String>,
    /// 第二段：主体人物/物体定义 (Person / Subject)
    pub person: String,
    /// 第三段：景别、机位与交互动作 (Shot / Framing / Encounter)
    pub shot: String,
    /// 第四段：空间场所、环境细节与调色板 (Setting / Palette)
    pub setting: String,
    /// 画面宽高比 ("9:16", "16:9", "1:1", "4:5")
    #[serde(default = "default_aspect_ratio")]
    pub aspect_ratio: String,
    /// 目标分辨率级别 ("1K", "2K", "preview")
    #[serde(default = "default_resolution")]
    pub resolution: String,
    /// 可选负向提示词
    #[serde(default)]
    pub negative_prompt: Option<String>,
}

impl Default for ImagePromptSpec {
    fn default() -> Self {
        Self {
            capture: Some(DEFAULT_PHONE_UGC_CAPTURE.to_string()),
            person: "A tech presenter with a sharp and focused presence.".to_string(),
            shot: "Vertical medium half-body shot, looking directly into the camera.".to_string(),
            setting: "A modern creator studio with subtle warm lighting. Palette: navy blue and pearl lavender.".to_string(),
            aspect_ratio: "9:16".to_string(),
            resolution: "1K".to_string(),
            negative_prompt: None,
        }
    }
}

/// 提示词合规性与质量审计报告
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromptAudit {
    pub is_valid: bool,
    pub word_count: usize,
    pub has_capture: bool,
    pub has_framing_or_shot: bool,
    pub has_gaze_or_encounter: bool,
    pub has_deliberate_palette: bool,
    pub warnings: Vec<String>,
    pub strengths: Vec<String>,
}

/// 图像生成请求对象
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenerationRequest {
    pub spec: ImagePromptSpec,
    pub output_path: PathBuf,
    pub shot_id: Option<String>,
    pub seed: Option<u64>,
}

/// 图像生成结果元数据清单
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenerationResult {
    pub shot_id: String,
    pub output_path: String,
    pub metadata_path: String,
    pub width: u32,
    pub height: u32,
    pub aspect_ratio: String,
    pub provider: String,
    pub model: String,
    pub prompt_hash: String,
    pub elapsed_ms: u64,
    pub spec: ImagePromptSpec,
    pub full_prompt: String,
}

/// 将四段式规格组装为生产级完整 Prompt
pub fn assemble_prompt(spec: &ImagePromptSpec) -> String {
    let capture_text = match &spec.capture {
        Some(cap) if !cap.trim().is_empty() => cap.trim(),
        _ => DEFAULT_PHONE_UGC_CAPTURE,
    };

    format!(
        "{}\n\n{}\n\n{}\n\n{}",
        capture_text.trim(),
        spec.person.trim(),
        spec.shot.trim(),
        spec.setting.trim()
    )
}

/// 审计四段式提示词，排查画质劣化反模式并校验必要导演维度
pub fn audit_prompt(spec: &ImagePromptSpec) -> PromptAudit {
    let full = assemble_prompt(spec);
    let lower_full = full.to_lowercase();
    let lower_shot = spec.shot.to_lowercase();
    let lower_setting = spec.setting.to_lowercase();

    let word_count = full.split_whitespace().count();
    let mut warnings = Vec::new();
    let mut strengths = Vec::new();

    // 1. 检查 Capture 摄影段
    let has_capture = spec
        .capture
        .as_ref()
        .map(|c| !c.trim().is_empty())
        .unwrap_or(false);
    if has_capture {
        strengths.push("Explicit photographic capture direction specified.".to_string());
    } else {
        warnings.push("No explicit capture specified; defaulting to canonical Phone-UGC.".to_string());
    }

    // 2. 检查 Shot 机位景别
    let framing_keywords = [
        "shot", "half-body", "close-up", "macro", "wide", "medium", "framing", "portrait",
        "angle", "camera", "crop", "head-and-shoulders",
    ];
    let has_framing_or_shot = framing_keywords.iter().any(|k| lower_shot.contains(k));
    if has_framing_or_shot {
        strengths.push("Shot establishes clear camera framing and distance.".to_string());
    } else {
        warnings.push("Shot lacks clear framing keywords (e.g. half-body, close-up, medium).".to_string());
    }

    // 3. 检查交互状态与视线
    let encounter_keywords = [
        "gaze", "looking", "facing", "lens", "camera", "viewer", "gesture", "pose", "idle",
        "hands", "seated", "sits", "stands",
    ];
    let has_gaze_or_encounter = encounter_keywords
        .iter()
        .any(|k| lower_shot.contains(k) || spec.person.to_lowercase().contains(k));
    if has_gaze_or_encounter {
        strengths.push("Encounter or ongoing activity specified.".to_string());
    } else {
        warnings.push("Lacks explicit gaze or encounter direction (e.g. looking straight at lens).".to_string());
    }

    // 4. 检查调色板 (Palette)
    let palette_keywords = [
        "palette", "color", "colors", "blue", "navy", "green", "terracotta", "lavender",
        "gold", "silver", "warm", "cool", "neutral", "pastel", "dark", "light", "crimson",
    ];
    let has_deliberate_palette = palette_keywords.iter().any(|k| lower_setting.contains(k));
    if has_deliberate_palette {
        strengths.push("Setting defines an authored color palette.".to_string());
    } else {
        warnings.push("Setting does not define an explicit palette (e.g. terracotta and green).".to_string());
    }

    // 5. 反模式检测 (避免 generic low-quality prompts)
    let buzzwords = [
        "photorealistic",
        "hyperrealistic",
        "octane render",
        "unreal engine",
        "trending on artstation",
        "masterpiece",
        "8k",
        "4k",
    ];
    for b in buzzwords {
        if lower_full.contains(b) {
            warnings.push(format!(
                "Contains anti-pattern buzzword '{}'; prefer concrete physical/optical descriptions.",
                b
            ));
        }
    }

    let is_valid = has_framing_or_shot && !spec.person.trim().is_empty();

    PromptAudit {
        is_valid,
        word_count,
        has_capture,
        has_framing_or_shot,
        has_gaze_or_encounter,
        has_deliberate_palette,
        warnings,
        strengths,
    }
}

/// 计算提示词哈希指纹
pub fn compute_prompt_hash(prompt: &str) -> String {
    let mut hasher = DefaultHasher::new();
    prompt.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// 根据宽高比与分辨率档位计算像素宽高
pub fn resolve_dimensions(aspect_ratio: &str, resolution: &str) -> (u32, u32) {
    match (aspect_ratio, resolution) {
        ("9:16", "2K") => (1080, 1920),
        ("9:16", "preview") => (360, 640),
        ("9:16", _) => (576, 1024), // 1K vertical

        ("16:9", "2K") => (1920, 1080),
        ("16:9", "preview") => (640, 360),
        ("16:9", _) => (1024, 576), // 1K horizontal

        ("1:1", "2K") => (2048, 2048),
        ("1:1", "preview") => (512, 512),
        ("1:1", _) => (1024, 1024), // 1K square

        ("4:5", "2K") => (1080, 1350),
        ("4:5", "preview") => (360, 450),
        ("4:5", _) => (864, 1080),

        _ => (576, 1024),
    }
}

/// 调色板颜色提取辅助
fn extract_palette_colors(setting: &str) -> (Rgb<u8>, Rgb<u8>) {
    let lower = setting.to_lowercase();
    let mut colors = Vec::new();

    if lower.contains("navy") {
        colors.push(Rgb([15, 23, 42]));
    }
    if lower.contains("lavender") || lower.contains("purple") {
        colors.push(Rgb([147, 112, 219]));
    }
    if lower.contains("terracotta") {
        colors.push(Rgb([195, 88, 64]));
    }
    if lower.contains("green") {
        colors.push(Rgb([46, 125, 50]));
    }
    if lower.contains("gold") || lower.contains("yellow") {
        colors.push(Rgb([218, 165, 32]));
    }
    if lower.contains("blue") && !lower.contains("navy") {
        colors.push(Rgb([30, 64, 175]));
    }
    if lower.contains("pearl") || lower.contains("white") {
        colors.push(Rgb([240, 243, 246]));
    }
    if lower.contains("dark") || lower.contains("black") {
        colors.push(Rgb([20, 24, 30]));
    }

    let c1 = colors.first().cloned().unwrap_or(Rgb([18, 24, 38]));
    let c2 = colors.get(1).cloned().unwrap_or(Rgb([56, 75, 112]));
    (c1, c2)
}

/// 简易嵌入式 5x7 ASCII 字符点阵字模 (无需外部 TTF 字体，100% 离线确定性渲染)
fn draw_char(img: &mut RgbImage, c: char, start_x: u32, start_y: u32, scale: u32, color: Rgb<u8>) {
    let glyph: [u8; 5] = match c.to_ascii_uppercase() {
        'A' => [0x7C, 0x12, 0x11, 0x12, 0x7C],
        'B' => [0x7F, 0x49, 0x49, 0x49, 0x36],
        'C' => [0x3E, 0x41, 0x41, 0x41, 0x22],
        'D' => [0x7F, 0x41, 0x41, 0x22, 0x1C],
        'E' => [0x7F, 0x49, 0x49, 0x49, 0x41],
        'F' => [0x7F, 0x09, 0x09, 0x09, 0x01],
        'G' => [0x3E, 0x41, 0x49, 0x49, 0x7A],
        'H' => [0x7F, 0x08, 0x08, 0x08, 0x7F],
        'I' => [0x00, 0x41, 0x7F, 0x41, 0x00],
        'J' => [0x20, 0x40, 0x41, 0x3F, 0x01],
        'K' => [0x7F, 0x08, 0x14, 0x22, 0x41],
        'L' => [0x7F, 0x40, 0x40, 0x40, 0x40],
        'M' => [0x7F, 0x02, 0x0C, 0x02, 0x7F],
        'N' => [0x7F, 0x04, 0x08, 0x10, 0x7F],
        'O' => [0x3E, 0x41, 0x41, 0x41, 0x3E],
        'P' => [0x7F, 0x09, 0x09, 0x09, 0x06],
        'Q' => [0x3E, 0x41, 0x51, 0x21, 0x5E],
        'R' => [0x7F, 0x09, 0x19, 0x29, 0x46],
        'S' => [0x46, 0x49, 0x49, 0x49, 0x31],
        'T' => [0x01, 0x01, 0x7F, 0x01, 0x01],
        'U' => [0x3F, 0x40, 0x40, 0x40, 0x3F],
        'V' => [0x1F, 0x20, 0x40, 0x20, 0x1F],
        'W' => [0x7F, 0x20, 0x18, 0x20, 0x7F],
        'X' => [0x63, 0x14, 0x08, 0x14, 0x63],
        'Y' => [0x07, 0x08, 0x70, 0x08, 0x07],
        'Z' => [0x61, 0x51, 0x49, 0x45, 0x43],
        '0' => [0x3E, 0x51, 0x49, 0x45, 0x3E],
        '1' => [0x00, 0x42, 0x7F, 0x40, 0x00],
        '2' => [0x42, 0x61, 0x51, 0x49, 0x46],
        '3' => [0x21, 0x41, 0x45, 0x4B, 0x31],
        '4' => [0x18, 0x14, 0x12, 0x7F, 0x10],
        '5' => [0x27, 0x45, 0x45, 0x45, 0x39],
        '6' => [0x3C, 0x4A, 0x49, 0x49, 0x30],
        '7' => [0x01, 0x71, 0x09, 0x05, 0x03],
        '8' => [0x36, 0x49, 0x49, 0x49, 0x36],
        '9' => [0x06, 0x49, 0x49, 0x29, 0x1E],
        ':' => [0x00, 0x36, 0x36, 0x00, 0x00],
        '-' => [0x08, 0x08, 0x08, 0x08, 0x08],
        '_' => [0x80, 0x80, 0x80, 0x80, 0x80],
        '.' => [0x00, 0x60, 0x60, 0x00, 0x00],
        '/' => [0x20, 0x10, 0x08, 0x04, 0x02],
        '#' => [0x14, 0x7F, 0x14, 0x7F, 0x14],
        ' ' => [0x00, 0x00, 0x00, 0x00, 0x00],
        _ => [0x7F, 0x7F, 0x7F, 0x7F, 0x7F],
    };

    let w = img.width();
    let h = img.height();

    for col in 0..5 {
        let col_bits = glyph[col];
        for row in 0..7 {
            if (col_bits & (1 << row)) != 0 {
                for dy in 0..scale {
                    for dx in 0..scale {
                        let px = start_x + (col as u32) * scale + dx;
                        let py = start_y + (row as u32) * scale + dy;
                        if px < w && py < h {
                            img.put_pixel(px, py, color);
                        }
                    }
                }
            }
        }
    }
}

/// 在图像上绘制单行点阵文本
fn draw_text(img: &mut RgbImage, text: &str, x: u32, y: u32, scale: u32, color: Rgb<u8>) {
    let mut cur_x = x;
    for ch in text.chars() {
        draw_char(img, ch, cur_x, y, scale, color);
        cur_x += 6 * scale; // 5 宽 + 1 间距
    }
}

/// 本地模拟/离线生成器 (MockImageGenerator)
/// 保证在无网络、无付费 API Key 环境下，测试套件和离线生产链路 100% 可行、完全闭环
#[derive(Debug, Default)]
pub struct MockImageGenerator;

impl MockImageGenerator {
    pub fn new() -> Self {
        Self
    }

    /// 渲染包含摄影构图参考线、调色板渐变与四段式导演元数据的水印画布
    pub fn render_canvas(&self, req: &ImageGenerationRequest) -> Result<RgbImage> {
        let (width, height) = resolve_dimensions(&req.spec.aspect_ratio, &req.spec.resolution);
        let mut img: RgbImage = ImageBuffer::new(width, height);

        let (c1, c2) = extract_palette_colors(&req.spec.setting);

        // 1. 绘制调色板优雅渐变背景
        for y in 0..height {
            let t = y as f32 / height as f32;
            let r = (c1[0] as f32 * (1.0 - t) + c2[0] as f32 * t) as u8;
            let g = (c1[1] as f32 * (1.0 - t) + c2[1] as f32 * t) as u8;
            let b = (c1[2] as f32 * (1.0 - t) + c2[2] as f32 * t) as u8;
            for x in 0..width {
                img.put_pixel(x, y, Rgb([r, g, b]));
            }
        }

        // 2. 绘制三分法则构图网格 (Rule of Thirds Guidelines)
        let grid_color = Rgb([80, 100, 130]);
        let x_one_third = width / 3;
        let x_two_thirds = (width * 2) / 3;
        let y_one_third = height / 3;
        let y_two_thirds = (height * 2) / 3;

        for y in 0..height {
            if y % 4 == 0 {
                img.put_pixel(x_one_third, y, grid_color);
                img.put_pixel(x_two_thirds, y, grid_color);
            }
        }
        for x in 0..width {
            if x % 4 == 0 {
                img.put_pixel(x, y_one_third, grid_color);
                img.put_pixel(x, y_two_thirds, grid_color);
            }
        }

        // 3. 绘制主体半身/景别线框 (Subject Wireframe Box)
        let lower_shot = req.spec.shot.to_lowercase();
        let is_offset_right = lower_shot.contains("right");
        let is_offset_left = lower_shot.contains("left");

        let box_w = width / 2;
        let box_h = height / 2;
        let box_x = if is_offset_right {
            (width * 5) / 12
        } else if is_offset_left {
            width / 12
        } else {
            width / 4
        };
        let box_y = height / 4;

        let frame_border_color = Rgb([200, 220, 255]);
        for x in box_x..(box_x + box_w).min(width) {
            img.put_pixel(x, box_y, frame_border_color);
            img.put_pixel(x, (box_y + box_h).min(height - 1), frame_border_color);
        }
        for y in box_y..(box_y + box_h).min(height) {
            img.put_pixel(box_x, y, frame_border_color);
            img.put_pixel((box_x + box_w).min(width - 1), y, frame_border_color);
        }

        // 4. 绘制分镜元数据信息铭牌 (Top/Bottom Info Badges)
        let shot_label = req.shot_id.as_deref().unwrap_or("SHOT_DIRECT");
        let hash_label = compute_prompt_hash(&assemble_prompt(&req.spec));
        let scale = if width >= 1000 { 3 } else { 2 };

        let text_color = Rgb([255, 255, 255]);
        let accent_color = Rgb([255, 215, 0]); // Gold

        draw_text(&mut img, "CLONE ANY VIDEO IMAGE DIRECTOR", 20, 20, scale, accent_color);
        draw_text(
            &mut img,
            &format!("SHOT: {}", shot_label.to_uppercase()),
            20,
            20 + 9 * scale,
            scale,
            text_color,
        );
        draw_text(
            &mut img,
            &format!("ASPECT: {} {}", req.spec.aspect_ratio, req.spec.resolution),
            20,
            20 + 18 * scale,
            scale,
            text_color,
        );
        draw_text(
            &mut img,
            &format!("HASH: {}", &hash_label[..8]),
            20,
            20 + 27 * scale,
            scale,
            text_color,
        );

        let bottom_y = height - 20 - 9 * scale;
        draw_text(&mut img, "STILL ASSET - LOCAL MOCK", 20, bottom_y, scale, accent_color);

        Ok(img)
    }

    pub fn generate(&self, req: &ImageGenerationRequest) -> Result<ImageGenerationResult> {
        let start = Instant::now();
        let full_prompt = assemble_prompt(&req.spec);
        let prompt_hash = compute_prompt_hash(&full_prompt);

        if let Some(parent) = req.output_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create output dir: {:?}", parent))?;
        }

        let canvas = self.render_canvas(req)?;
        canvas
            .save(&req.output_path)
            .with_context(|| format!("Failed to save output image to {:?}", req.output_path))?;

        let (width, height) = (canvas.width(), canvas.height());
        let elapsed_ms = start.elapsed().as_millis() as u64;

        let shot_id = req
            .shot_id
            .clone()
            .unwrap_or_else(|| "shot_unnamed".to_string());

        let metadata_path = req.output_path.with_extension("json");
        let result = ImageGenerationResult {
            shot_id,
            output_path: req.output_path.to_string_lossy().to_string(),
            metadata_path: metadata_path.to_string_lossy().to_string(),
            width,
            height,
            aspect_ratio: req.spec.aspect_ratio.clone(),
            provider: "local-mock".to_string(),
            model: "canvas-renderer-v1".to_string(),
            prompt_hash,
            elapsed_ms,
            spec: req.spec.clone(),
            full_prompt,
        };

        let meta_json = serde_json::to_string_pretty(&result)?;
        std::fs::write(&metadata_path, meta_json)?;

        crate::artifacts::record(&req.output_path, crate::artifacts::Mode::Mock, &result.provider)?;
        Ok(result)
    }
}

/// 远程 API 生产级生图适配器 (OpenAI DALL-E / GPT Image 2 / FLUX Proxy)
pub struct OpenAiImageGenerator {
    api_key: String,
    base_url: String,
    model: String,
}

impl OpenAiImageGenerator {
    pub fn new(api_key: String, base_url: Option<String>, model: Option<String>) -> Self {
        Self {
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            model: model.unwrap_or_else(|| "dall-e-3".to_string()),
        }
    }

    pub async fn generate(&self, req: &ImageGenerationRequest) -> Result<ImageGenerationResult> {
        let start = Instant::now();
        let full_prompt = assemble_prompt(&req.spec);
        let prompt_hash = compute_prompt_hash(&full_prompt);

        let (width, height) = resolve_dimensions(&req.spec.aspect_ratio, &req.spec.resolution);
        let size_str = match req.spec.aspect_ratio.as_str() {
            "9:16" => "1024x1792",
            "16:9" => "1792x1024",
            _ => "1024x1024",
        };

        let client = reqwest::Client::new();
        let endpoint = format!("{}/images/generations", self.base_url.trim_end_matches('/'));

        let body = serde_json::json!({
            "model": self.model,
            "prompt": full_prompt,
            "n": 1,
            "size": size_str,
            "response_format": "b64_json"
        });

        let resp = client
            .post(&endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("Failed to send request to image API endpoint: {}", endpoint))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_text = resp.text().await.unwrap_or_default();
            bail!("Image generation API error {}: {}", status, err_text);
        }

        let resp_json: serde_json::Value = resp.json().await?;
        let b64 = resp_json["data"][0]["b64_json"]
            .as_str()
            .context("Missing b64_json in image response")?;

        use std::io::Write;
        let img_bytes = base64_decode(b64)?;

        if let Some(parent) = req.output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::File::create(&req.output_path)?;
        file.write_all(&img_bytes)?;

        let elapsed_ms = start.elapsed().as_millis() as u64;
        let shot_id = req.shot_id.clone().unwrap_or_else(|| "shot_remote".to_string());
        let metadata_path = req.output_path.with_extension("json");

        let result = ImageGenerationResult {
            shot_id,
            output_path: req.output_path.to_string_lossy().to_string(),
            metadata_path: metadata_path.to_string_lossy().to_string(),
            width,
            height,
            aspect_ratio: req.spec.aspect_ratio.clone(),
            provider: "openai".to_string(),
            model: self.model.clone(),
            prompt_hash,
            elapsed_ms,
            spec: req.spec.clone(),
            full_prompt,
        };

        let meta_json = serde_json::to_string_pretty(&result)?;
        std::fs::write(&metadata_path, meta_json)?;

        crate::artifacts::record(&req.output_path, crate::artifacts::Mode::Production, &result.provider)?;
        Ok(result)
    }
}

/// 轻量级 Base64 解码器
fn base64_decode(input: &str) -> Result<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut map = [0xFFu8; 256];
    for (i, &b) in TABLE.iter().enumerate() {
        map[b as usize] = i as u8;
    }

    let clean: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace() && *b != b'=').collect();
    let mut out = Vec::with_capacity((clean.len() * 3) / 4);

    let chunks = clean.chunks_exact(4);
    let remainder = chunks.remainder();

    for chunk in chunks {
        let b0 = map[chunk[0] as usize];
        let b1 = map[chunk[1] as usize];
        let b2 = map[chunk[2] as usize];
        let b3 = map[chunk[3] as usize];
        if b0 == 0xFF || b1 == 0xFF || b2 == 0xFF || b3 == 0xFF {
            bail!("Invalid base64 character in chunk");
        }
        out.push((b0 << 2) | (b1 >> 4));
        out.push((b1 << 4) | (b2 >> 2));
        out.push((b2 << 6) | b3);
    }

    match remainder.len() {
        2 => {
            let b0 = map[remainder[0] as usize];
            let b1 = map[remainder[1] as usize];
            if b0 == 0xFF || b1 == 0xFF {
                bail!("Invalid base64 trailing characters");
            }
            out.push((b0 << 2) | (b1 >> 4));
        }
        3 => {
            let b0 = map[remainder[0] as usize];
            let b1 = map[remainder[1] as usize];
            let b2 = map[remainder[2] as usize];
            if b0 == 0xFF || b1 == 0xFF || b2 == 0xFF {
                bail!("Invalid base64 trailing characters");
            }
            out.push((b0 << 2) | (b1 >> 4));
            out.push((b1 << 4) | (b2 >> 2));
        }
        _ => {}
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_expand_prompt_four_paragraphs() {
        let spec = ImagePromptSpec {
            capture: Some(DEFAULT_PHONE_UGC_CAPTURE.to_string()),
            person: "A Japanese woman in her early twenties, strikingly beautiful.".to_string(),
            shot: "Vertical medium half-body shot, looking directly into the lens.".to_string(),
            setting: "On an apartment balcony. Palette: terracotta and green.".to_string(),
            aspect_ratio: "9:16".to_string(),
            resolution: "2K".to_string(),
            negative_prompt: None,
        };

        let full = assemble_prompt(&spec);
        let paragraphs: Vec<&str> = full.split("\n\n").collect();
        assert_eq!(paragraphs.len(), 4, "Must assemble into exactly 4 distinct paragraphs");
        assert!(paragraphs[0].contains("shot on an iPhone"));
        assert!(paragraphs[1].contains("strikingly beautiful"));
        assert!(paragraphs[2].contains("Vertical medium half-body"));
        assert!(paragraphs[3].contains("terracotta and green"));
    }

    #[test]
    fn test_audit_prompt_validity_and_warnings() {
        // 合规提示词
        let valid_spec = ImagePromptSpec {
            capture: Some(DEFAULT_PHONE_UGC_CAPTURE.to_string()),
            person: "A Chinese presenter in her early thirties, clear and confident.".to_string(),
            shot: "Vertical medium half-body shot, face points straight toward the camera.".to_string(),
            setting: "High-tech studio with soft ambient light. Palette: navy blue and gold.".to_string(),
            aspect_ratio: "9:16".to_string(),
            resolution: "1K".to_string(),
            negative_prompt: None,
        };
        let audit = audit_prompt(&valid_spec);
        assert!(audit.is_valid);
        assert!(audit.has_capture);
        assert!(audit.has_framing_or_shot);
        assert!(audit.has_gaze_or_encounter);
        assert!(audit.has_deliberate_palette);
        assert!(audit.warnings.is_empty() || audit.warnings.iter().all(|w| !w.contains("anti-pattern")));

        // 包含反模式违禁词的提示词
        let buzzword_spec = ImagePromptSpec {
            capture: None,
            person: "A photorealistic anime girl, trending on artstation, masterpiece 8k octane render.".to_string(),
            shot: "standing around".to_string(),
            setting: "white room".to_string(),
            aspect_ratio: "1:1".to_string(),
            resolution: "1K".to_string(),
            negative_prompt: None,
        };
        let audit2 = audit_prompt(&buzzword_spec);
        assert!(audit2.warnings.iter().any(|w| w.contains("photorealistic")));
        assert!(audit2.warnings.iter().any(|w| w.contains("octane render")));
    }

    #[test]
    fn test_resolve_dimensions_and_hash() {
        assert_eq!(resolve_dimensions("9:16", "2K"), (1080, 1920));
        assert_eq!(resolve_dimensions("9:16", "preview"), (360, 640));
        assert_eq!(resolve_dimensions("16:9", "2K"), (1920, 1080));
        assert_eq!(resolve_dimensions("16:9", "preview"), (640, 360));
        assert_eq!(resolve_dimensions("1:1", "1K"), (1024, 1024));

        let hash1 = compute_prompt_hash("Test prompt A");
        let hash2 = compute_prompt_hash("Test prompt A");
        let hash3 = compute_prompt_hash("Test prompt B");
        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_mock_image_generator_render_and_save() {
        let dir = tempdir().unwrap();
        let out_img = dir.path().join("shot_01.png");

        let spec = ImagePromptSpec {
            capture: Some(DEFAULT_PHONE_UGC_CAPTURE.to_string()),
            person: "A presenter introducing a video engine.".to_string(),
            shot: "Vertical medium half-body shot, direct gaze at viewer.".to_string(),
            setting: "Balcony at sunset. Palette: terracotta and green.".to_string(),
            aspect_ratio: "9:16".to_string(),
            resolution: "preview".to_string(),
            negative_prompt: None,
        };

        let req = ImageGenerationRequest {
            spec,
            output_path: out_img.clone(),
            shot_id: Some("shot_01".to_string()),
            seed: Some(42),
        };

        let gen = MockImageGenerator::new();
        let res = gen.generate(&req).expect("Mock generation should succeed");

        assert_eq!(res.width, 360);
        assert_eq!(res.height, 640);
        assert!(out_img.exists(), "Output image PNG must be created");

        let meta_file = dir.path().join("shot_01.json");
        assert!(meta_file.exists(), "Metadata sidecar JSON must be created");

        // 读回图像验证格式与尺寸
        let loaded = image::open(&out_img).expect("Should be valid image file");
        assert_eq!(loaded.width(), 360);
        assert_eq!(loaded.height(), 640);
    }

    #[test]
    fn test_base64_decode() {
        let plain = b"Clone Any Video Image Director Engine";
        // "Clone Any Video Image Director Engine" base64 encoded
        let encoded = "Q2xvbmUgQW55IFZpZGVvIEltYWdlIERpcmVjdG9yIEVuZ2luZQ==";
        let decoded = base64_decode(encoded).expect("Decode should succeed");
        assert_eq!(decoded, plain);
    }
}
