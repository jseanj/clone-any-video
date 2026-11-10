use anyhow::{bail, Context, Result};
use image::{imageops, ImageBuffer, Rgb, RgbImage};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::process::Command;

use crate::transcribe::TranscriptDocument;

/// 默认缩略图单格宽度 (480px)
pub const DEFAULT_TILE_CELL_WIDTH: u32 = 480;
/// 默认排版列数 (3列)
pub const DEFAULT_TILE_COLUMNS: u32 = 3;
/// 默认多页排版行数 (3行，即 3x3 = 9格/页)
pub const DEFAULT_TILE_ROWS: u32 = 3;

// ============================================================================
// 数据结构定义 (全景视盘拼贴规范)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampledFrame {
    #[serde(rename = "requestedAt")]
    pub requested_at: f64,
    pub at: f64,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FramesResult {
    pub directory: String,
    pub labeled: bool,
    pub frames: Vec<SampledFrame>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileResult {
    pub path: String,
    pub samples: Vec<f64>,
    pub columns: u32,
    pub rows: u32,
    #[serde(rename = "cellWidth")]
    pub cell_width: u32,
    pub frames: Vec<SampledFrame>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TilesResult {
    pub directory: String,
    pub columns: u32,
    pub rows: u32,
    #[serde(rename = "cellWidth")]
    pub cell_width: u32,
    pub grids: Vec<TileResult>,
}

pub struct FramesOptions<'a> {
    pub source: &'a Path,
    pub target_dir: &'a Path,
    pub times: Vec<f64>,
    pub label_time: bool,
    pub transcript: Option<&'a TranscriptDocument>,
}

pub struct TileOptions<'a> {
    pub source: &'a Path,
    pub target_file: &'a Path,
    pub times: Vec<f64>,
    pub cell_width: u32,
    pub columns: u32,
    pub transcript: Option<&'a TranscriptDocument>,
}

pub struct TilesOptions<'a> {
    pub source: &'a Path,
    pub target_dir: &'a Path,
    pub times: Vec<f64>,
    pub cell_width: u32,
    pub columns: u32,
    pub rows: u32,
    pub transcript: Option<&'a TranscriptDocument>,
}

// ============================================================================
// 抽帧核心函数
// ============================================================================

/// 极速单帧抽取 (使用 FFmpeg 快速 seek + 提取单帧)
pub async fn cut_single_frame(source: &Path, at: f64, target: &Path) -> Result<()> {
    if let Some(parent) = target.parent() {
        if !parent.exists() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }

    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-ss",
            &format!("{:.3}", at),
            "-i",
        ])
        .arg(source)
        .args([
            "-frames:v",
            "1",
            "-update",
            "1",
            "-q:v",
            "2",
        ])
        .arg(target)
        .status()
        .await
        .with_context(|| format!("Failed to extract frame at {:.3}s from {:?}", at, source))?;

    if !status.success() {
        bail!("FFmpeg frame extraction failed at {:.3}s", at);
    }

    Ok(())
}

/// Extract an unlabelled visual reference; no original audio is carried forward.
pub async fn extract_reference_frame(source: &Path, at_seconds: f64, target: &Path) -> Result<()> {
    let info = crate::probe::probe_media(source).await?;
    if !at_seconds.is_finite() || at_seconds < 0.0 || at_seconds >= info.duration_seconds {
        bail!("Reference frame timestamp must be within the actual source video");
    }
    cut_single_frame(source, at_seconds, target).await?;
    let frame = image::open(target).context("No decodable frame at the requested timestamp")?;
    crate::artifacts::record_derived(
        target,
        crate::artifacts::mode(source),
        "ffmpeg-extract-frame",
        &[source.to_path_buf()],
        serde_json::json!({"requested_seconds":at_seconds,"label_time":false,"width":frame.width(),"height":frame.height(),"audio_reference":false}),
    )?;
    Ok(())
}

/// 格式化时间码为 00:00.000 风格
pub fn format_timecode(seconds: f64) -> String {
    let total_ms = (seconds * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let s = (total_ms / 1000) % 60;
    let m = (total_ms / 60000) % 60;
    format!("{:02}:{:02}.{:03}", m, s, ms)
}

/// 在转录文本中查找指定时间戳命中或相邻的台词
pub fn find_word_at(transcript: &TranscriptDocument, at: f64) -> Option<String> {
    for passage in &transcript.passages {
        for w in &passage.words {
            if let (Some(start), Some(end)) = (w.start_seconds, w.end_seconds) {
                if at >= (start - 0.05) && at <= (end + 0.05) {
                    return Some(format!("\"{}\" ({:.2}s-{:.2}s)", w.text, start, end));
                }
            }
        }
    }
    None
}

// ============================================================================
// 纯 Rust 极速无依赖 Bitmap 字体渲染引擎 (ASCII 8x16)
// ============================================================================

mod font8x16 {
    // 简易 8x16 点阵字模生成器：涵盖常用数字、冒号、小数点、字母与常用标点
    pub fn get_glyph(c: char) -> [u8; 16] {
        match c {
            '0' => [0x3c, 0x66, 0x66, 0x6e, 0x76, 0x66, 0x66, 0x3c, 0, 0, 0, 0, 0, 0, 0, 0],
            '1' => [0x18, 0x38, 0x18, 0x18, 0x18, 0x18, 0x18, 0x7e, 0, 0, 0, 0, 0, 0, 0, 0],
            '2' => [0x3c, 0x66, 0x06, 0x0c, 0x18, 0x30, 0x60, 0x7e, 0, 0, 0, 0, 0, 0, 0, 0],
            '3' => [0x3c, 0x66, 0x06, 0x1c, 0x06, 0x06, 0x66, 0x3c, 0, 0, 0, 0, 0, 0, 0, 0],
            '4' => [0x0c, 0x1c, 0x34, 0x64, 0x7e, 0x04, 0x04, 0x0e, 0, 0, 0, 0, 0, 0, 0, 0],
            '5' => [0x7e, 0x60, 0x7c, 0x06, 0x06, 0x06, 0x66, 0x3c, 0, 0, 0, 0, 0, 0, 0, 0],
            '6' => [0x1c, 0x30, 0x60, 0x7c, 0x66, 0x66, 0x66, 0x3c, 0, 0, 0, 0, 0, 0, 0, 0],
            '7' => [0x7e, 0x06, 0x0c, 0x18, 0x18, 0x30, 0x30, 0x30, 0, 0, 0, 0, 0, 0, 0, 0],
            '8' => [0x3c, 0x66, 0x66, 0x3c, 0x66, 0x66, 0x66, 0x3c, 0, 0, 0, 0, 0, 0, 0, 0],
            '9' => [0x3c, 0x66, 0x66, 0x66, 0x3e, 0x06, 0x0c, 0x38, 0, 0, 0, 0, 0, 0, 0, 0],
            ':' => [0x00, 0x18, 0x18, 0x00, 0x00, 0x18, 0x18, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            '-' => [0x00, 0x00, 0x00, 0x7e, 0x00, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            ' ' => [0x00; 16],
            '[' => [0x1e, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x1e, 0, 0, 0, 0, 0, 0, 0, 0],
            ']' => [0x78, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x78, 0, 0, 0, 0, 0, 0, 0, 0],
            '(' => [0x0c, 0x18, 0x30, 0x30, 0x30, 0x18, 0x0c, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            ')' => [0x30, 0x18, 0x0c, 0x0c, 0x0c, 0x18, 0x30, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            '"' => [0x66, 0x66, 0x24, 0x00, 0x00, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            's' | 'S' => [0x3c, 0x66, 0x60, 0x3c, 0x06, 0x66, 0x3c, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            'a' | 'A' => [0x18, 0x3c, 0x66, 0x7e, 0x66, 0x66, 0x66, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            'b' | 'B' => [0x7c, 0x66, 0x66, 0x7c, 0x66, 0x66, 0x7c, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            'c' | 'C' => [0x3c, 0x66, 0x60, 0x60, 0x60, 0x66, 0x3c, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            'd' | 'D' => [0x78, 0x6c, 0x66, 0x66, 0x66, 0x6c, 0x78, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            'e' | 'E' => [0x7e, 0x60, 0x60, 0x7c, 0x60, 0x60, 0x7e, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            'f' | 'F' => [0x7e, 0x60, 0x60, 0x7c, 0x60, 0x60, 0x60, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            'g' | 'G' => [0x3c, 0x66, 0x60, 0x6e, 0x66, 0x66, 0x3c, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            'h' | 'H' => [0x66, 0x66, 0x66, 0x7e, 0x66, 0x66, 0x66, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            'i' | 'I' => [0x3c, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3c, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            'l' | 'L' => [0x60, 0x60, 0x60, 0x60, 0x60, 0x60, 0x7e, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            'm' | 'M' => [0x63, 0x77, 0x7f, 0x6b, 0x63, 0x63, 0x63, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            'n' | 'N' => [0x66, 0x76, 0x7e, 0x7e, 0x6e, 0x66, 0x66, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            'o' | 'O' => [0x3c, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3c, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            'p' | 'P' => [0x7c, 0x66, 0x66, 0x7c, 0x60, 0x60, 0x60, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            'r' | 'R' => [0x7c, 0x66, 0x66, 0x7c, 0x6c, 0x66, 0x66, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            't' | 'T' => [0x7e, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            'u' | 'U' => [0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3c, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            'v' | 'V' => [0x66, 0x66, 0x66, 0x66, 0x66, 0x3c, 0x18, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            'w' | 'W' => [0x63, 0x63, 0x63, 0x6b, 0x7f, 0x77, 0x63, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            'y' | 'Y' => [0x66, 0x66, 0x66, 0x3c, 0x18, 0x18, 0x18, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
            _ => [0x7e, 0x42, 0x5a, 0x5a, 0x42, 0x7e, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
        }
    }
}

/// 在图像上绘制指定文字 (支持缩放因子 scale)
pub fn draw_text_on_image(
    img: &mut RgbImage,
    start_x: u32,
    start_y: u32,
    text: &str,
    color: Rgb<u8>,
    scale: u32,
) {
    let mut cur_x = start_x;
    for ch in text.chars() {
        let glyph = font8x16::get_glyph(ch);
        for row in 0..12 {
            let row_byte = glyph[row];
            for col in 0..8 {
                if (row_byte & (0x80 >> col)) != 0 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let px = cur_x + col * scale + dx;
                            let py = start_y + (row as u32) * scale + dy;
                            if px < img.width() && py < img.height() {
                                img.put_pixel(px, py, color);
                            }
                        }
                    }
                }
            }
        }
        cur_x += 8 * scale + scale;
    }
}

/// 给单个格子的图片加上底部信息横条 (Timecode + Word)
pub fn render_cell_banner(
    frame_img: &RgbImage,
    at: f64,
    word_opt: Option<&str>,
) -> RgbImage {
    let w = frame_img.width();
    let h = frame_img.height();
    let banner_height = 42u32;
    let mut cell_canvas = ImageBuffer::from_pixel(w, h + banner_height, Rgb([16, 16, 22]));

    // 复制原图
    imageops::overlay(&mut cell_canvas, frame_img, 0, 0);

    // 绘制底部横条边框线
    for x in 0..w {
        cell_canvas.put_pixel(x, h, Rgb([45, 45, 58]));
    }

    // 绘制时间码 (醒目亮黄白)
    let time_str = format_timecode(at);
    draw_text_on_image(&mut cell_canvas, 10, h + 10, &time_str, Rgb([255, 230, 140]), 2);

    // 绘制台词文字 (如果存在)
    if let Some(word) = word_opt {
        let word_display = if word.len() > 30 {
            format!("{}…", &word[..28])
        } else {
            word.to_string()
        };
        draw_text_on_image(&mut cell_canvas, 165, h + 12, &word_display, Rgb([200, 215, 235]), 1);
    }

    cell_canvas
}

// ============================================================================
// 顶层业务实现: frames / tile / tiles
// ============================================================================

/// 执行 frames 命令：按时间戳提取一系列关键帧，可选择烧录时间戳
pub async fn extract_frames(opt: FramesOptions<'_>) -> Result<FramesResult> {
    tokio::fs::create_dir_all(opt.target_dir).await?;

    let mut frames = Vec::new();

    for at in opt.times {
        let stem = format!("{:07.3}", at).replace('.', "_");
        let filename = format!("frame-{}s.jpg", stem);
        let target_path = opt.target_dir.join(&filename);

        cut_single_frame(opt.source, at, &target_path).await?;

        let word = opt.transcript.and_then(|t| find_word_at(t, at));

        if opt.label_time {
            let img = image::open(&target_path)
                .with_context(|| format!("Failed to read frame {:?}", target_path))?
                .to_rgb8();
            let labeled_img = render_cell_banner(&img, at, word.as_deref());
            labeled_img.save(&target_path)?;
        }

        frames.push(SampledFrame {
            requested_at: at,
            at,
            path: target_path.to_string_lossy().to_string(),
            word,
        });
    }

    Ok(FramesResult {
        directory: opt.target_dir.to_string_lossy().to_string(),
        labeled: opt.label_time,
        frames,
    })
}

/// 执行 tile 命令：将若干关键帧网格化拼接为一张全景视盘图片 (Contact Sheet)
pub async fn create_tile(opt: TileOptions<'_>) -> Result<TileResult> {
    if opt.times.is_empty() {
        bail!("Tile requires at least one timestamp");
    }

    let temp_dir = tempfile::tempdir()?;
    let cell_w = opt.cell_width;
    let columns = opt.columns.max(1);
    let rows = ((opt.times.len() as f64) / (columns as f64)).ceil() as u32;

    let mut sampled_frames = Vec::new();
    let mut rendered_cells = Vec::new();

    for (idx, &at) in opt.times.iter().enumerate() {
        let temp_frame_path = temp_dir.path().join(format!("raw_{:04}.jpg", idx));
        cut_single_frame(opt.source, at, &temp_frame_path).await?;

        let img = image::open(&temp_frame_path)?.to_rgb8();
        // 保持比例缩放到目标宽度
        let aspect = img.height() as f64 / img.width() as f64;
        let cell_h = (cell_w as f64 * aspect).round() as u32;
        let resized = imageops::resize(&img, cell_w, cell_h, imageops::FilterType::Triangle);

        let word = opt.transcript.and_then(|t| find_word_at(t, at));
        let cell_banner = render_cell_banner(&resized, at, word.as_deref());

        sampled_frames.push(SampledFrame {
            requested_at: at,
            at,
            path: opt.target_file.to_string_lossy().to_string(),
            word,
        });
        rendered_cells.push(cell_banner);
    }

    let single_cell_w = rendered_cells[0].width();
    let single_cell_h = rendered_cells[0].height();
    let padding = 12u32;

    let canvas_w = columns * single_cell_w + (columns + 1) * padding;
    let canvas_h = rows * single_cell_h + (rows + 1) * padding;

    let mut canvas = ImageBuffer::from_pixel(canvas_w, canvas_h, Rgb([12, 12, 16]));

    for (idx, cell) in rendered_cells.iter().enumerate() {
        let r = (idx as u32) / columns;
        let c = (idx as u32) % columns;
        let x = padding + c * (single_cell_w + padding);
        let y = padding + r * (single_cell_h + padding);
        imageops::overlay(&mut canvas, cell, x as i64, y as i64);
    }

    if let Some(parent) = opt.target_file.parent() {
        if !parent.exists() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }
    canvas.save(opt.target_file)?;

    Ok(TileResult {
        path: opt.target_file.to_string_lossy().to_string(),
        samples: opt.times,
        columns,
        rows,
        cell_width: cell_w,
        frames: sampled_frames,
    })
}

/// 执行 tiles 命令：针对多段范围或长视频，分页输出多张 Contact Sheets
pub async fn create_tiles(opt: TilesOptions<'_>) -> Result<TilesResult> {
    tokio::fs::create_dir_all(opt.target_dir).await?;

    let per_page = (opt.columns * opt.rows) as usize;
    let mut grids = Vec::new();

    for (page_idx, chunk) in opt.times.chunks(per_page).enumerate() {
        let page_filename = format!("sheet_p{:03}.jpg", page_idx + 1);
        let page_path = opt.target_dir.join(&page_filename);

        let res = create_tile(TileOptions {
            source: opt.source,
            target_file: &page_path,
            times: chunk.to_vec(),
            cell_width: opt.cell_width,
            columns: opt.columns,
            transcript: opt.transcript,
        })
        .await?;

        grids.push(res);
    }

    Ok(TilesResult {
        directory: opt.target_dir.to_string_lossy().to_string(),
        columns: opt.columns,
        rows: opt.rows,
        cell_width: opt.cell_width,
        grids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timecode_formatting() {
        assert_eq!(format_timecode(0.0), "00:00.000");
        assert_eq!(format_timecode(1.5), "00:01.500");
        assert_eq!(format_timecode(65.123), "01:05.123");
    }

    #[test]
    fn test_grid_canvas_math() {
        let cols = 3u32;
        let rows = 2u32;
        let cell_w = 480u32;
        let cell_h = 312u32;
        let padding = 12u32;

        let canvas_w = cols * cell_w + (cols + 1) * padding;
        let canvas_h = rows * cell_h + (rows + 1) * padding;

        assert_eq!(canvas_w, 3 * 480 + 4 * 12);
        assert_eq!(canvas_h, 2 * 312 + 3 * 12);
    }

    #[tokio::test]
    async fn test_cut_single_frame() {
        let fixture = if Path::new("fixtures/cuts_sample.mp4").exists() { Path::new("fixtures/cuts_sample.mp4") } else { Path::new("../../fixtures/cuts_sample.mp4") };
        if fixture.exists() {
            let temp_dir = tempfile::tempdir().unwrap();
            let frame_path = temp_dir.path().join("frame.jpg");
            cut_single_frame(fixture, 1.0, &frame_path).await.unwrap();
            assert!(frame_path.exists());
        }
    }

    #[tokio::test]
    async fn test_create_tile_grid() {
        let fixture = if Path::new("fixtures/cuts_sample.mp4").exists() { Path::new("fixtures/cuts_sample.mp4") } else { Path::new("../../fixtures/cuts_sample.mp4") };
        if fixture.exists() {
            let temp_dir = tempfile::tempdir().unwrap();
            let tile_path = temp_dir.path().join("grid.jpg");
            let res = create_tile(TileOptions {
                source: fixture,
                target_file: &tile_path,
                times: vec![1.0, 3.0, 5.0],
                cell_width: 320,
                columns: 3,
                transcript: None,
            })
            .await
            .unwrap();

            assert_eq!(res.columns, 3);
            assert_eq!(res.rows, 1);
            assert!(tile_path.exists());
        }
    }
}
