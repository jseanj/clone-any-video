use anyhow::{bail, Context, Result};
use image::{Rgb, RgbImage};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

use crate::project::ProjectManifest;
#[cfg(test)]
use crate::project::TrackItem;

// ============================================================================
// 1. 浏览器环境探测与 Provider 模型 (Chrome Detector & Provider)
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum BrowserProvider {
    Chrome { executable: PathBuf },
    Mock,
}

/// 自动探测系统中可用的 Google Chrome / Chromium 可执行文件路径
pub fn detect_chrome_executable() -> Option<PathBuf> {
    // 1. 优先读取环境变量
    for env_var in ["CHROME_BIN", "CHROME_PATH", "PUPPETEER_EXECUTABLE_PATH"] {
        if let Ok(val) = std::env::var(env_var) {
            let p = PathBuf::from(val);
            if p.exists() {
                return Some(p);
            }
        }
    }

    // 2. macOS 常见安装路径
    #[cfg(target_os = "macos")]
    {
        let mac_candidates = [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        ];
        for candidate in mac_candidates {
            let p = PathBuf::from(candidate);
            if p.exists() {
                return Some(p);
            }
        }
    }

    // 3. Linux 常见二进制
    #[cfg(target_os = "linux")]
    {
        let linux_candidates = [
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/snap/bin/chromium",
        ];
        for candidate in linux_candidates {
            let p = PathBuf::from(candidate);
            if p.exists() {
                return Some(p);
            }
        }
    }

    // 4. Windows 常见路径
    #[cfg(target_os = "windows")]
    {
        let win_candidates = [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        ];
        for candidate in win_candidates {
            let p = PathBuf::from(candidate);
            if p.exists() {
                return Some(p);
            }
        }
    }

    // 5. 探测 PATH 命令
    for cmd in [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
    ] {
        if let Ok(out) = std::process::Command::new("which").arg(cmd).output() {
            if out.status.success() {
                let path_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !path_str.is_empty() {
                    return Some(PathBuf::from(path_str));
                }
            }
        }
    }

    None
}

pub fn resolve_browser_provider(force_mock: bool) -> Result<BrowserProvider> {
    if force_mock {
        return Ok(BrowserProvider::Mock);
    }
    match detect_chrome_executable() {
        Some(executable) => Ok(BrowserProvider::Chrome { executable }),
        None => anyhow::bail!(
            "Google Chrome executable not detected on this system.\n\
             Production mode prohibits silent mock degradation. Install Chrome or pass --mock explicitly for simulated rendering."
        ),
    }
}

// ============================================================================
// 2. 捕获配置模型 (CaptureConfig)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptureConfig {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub crf: u32,
    pub preset: String,
    pub transparent: bool,
    pub timeout_seconds: u64,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            width: 640,
            height: 360,
            fps: 30.0,
            crf: 23,
            preset: "medium".into(),
            transparent: true,
            timeout_seconds: 30,
        }
    }
}

// ============================================================================
// 3. 核心驱动实现：单帧截取、帧序列收割与视频转码
// ============================================================================

/// 捕获 HTML 视听组件在特定帧索引处的单个 PNG 图像
pub async fn capture_single_frame(
    html_path: &Path,
    frame_index: u64,
    total_frames: u64,
    config: &CaptureConfig,
    out_png: &Path,
    provider: &BrowserProvider,
) -> Result<()> {
    if let Some(parent) = out_png.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let progress = if total_frames > 1 {
        frame_index as f64 / (total_frames - 1) as f64
    } else {
        0.0
    };

    match provider {
        BrowserProvider::Chrome { executable } => {
            // 为当前帧生成一个内联驱动的临时 HTML 文件，执行精确的 window.renderFrame(frame, progress)
            let raw_html = tokio::fs::read_to_string(html_path)
                .await
                .with_context(|| format!("Failed to read source HTML: {:?}", html_path))?;

            // 注入 <base href="..."> 确保组件引用的相对图片、样式表、脚本稳定寻址
            let html_dir = html_path
                .canonicalize()
                .unwrap_or_else(|_| html_path.to_path_buf())
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let base_tag = if !html_dir.is_empty() {
                format!("<base href=\"file://{}/\">", html_dir.trim_end_matches('/'))
            } else {
                String::new()
            };

            // 注入自执行渲染驱动脚本
            let driver_script = format!(
                r#"<script>
  if (typeof window.renderFrame === 'function') {{
    window.renderFrame({}, {:.6});
  }}
</script></body>"#,
                frame_index, progress
            );

            let mut staged_html = raw_html;
            if !base_tag.is_empty() {
                if staged_html.contains("<head>") {
                    staged_html =
                        staged_html.replacen("<head>", &format!("<head>\n  {}", base_tag), 1);
                } else if staged_html.contains("<html>") {
                    staged_html = staged_html.replacen(
                        "<html>",
                        &format!("<html>\n<head>{}</head>", base_tag),
                        1,
                    );
                } else {
                    staged_html = format!("{}\n{}", base_tag, staged_html);
                }
            }

            staged_html = if staged_html.contains("</body>") {
                staged_html.replace("</body>", &driver_script)
            } else {
                format!("{}{}", staged_html, driver_script)
            };

            // 在源 HTML 所在同级目录暂存临时驱动文件，维持本地同源沙箱与资源可访问性
            let staged_dir = html_path.parent().unwrap_or_else(|| Path::new("."));
            let staged_file = staged_dir.join(format!(
                ".staged_frame_{}_{}.html",
                std::process::id(),
                frame_index
            ));
            tokio::fs::write(&staged_file, staged_html).await?;

            let file_url = format!("file://{}", staged_file.canonicalize()?.to_string_lossy());
            let screenshot_arg = format!("--screenshot={}", out_png.to_string_lossy());
            let window_size_arg = format!("--window-size={},{}", config.width, config.height);

            let mut cmd = Command::new(executable);
            cmd.arg("--headless=new")
                .arg("--disable-gpu")
                .arg("--no-sandbox")
                .arg("--disable-dev-shm-usage")
                .arg("--no-first-run")
                .arg("--disable-extensions")
                .arg("--disable-background-networking")
                .arg("--disable-sync")
                .arg("--disable-default-apps")
                .arg("--hide-scrollbars")
                .arg("--mute-audio")
                .arg("--default-background-color=00000000")
                .arg(&window_size_arg)
                .arg(&screenshot_arg)
                .arg(&file_url)
                .stdout(Stdio::null())
                .stderr(Stdio::null());

            let status = cmd
                .status()
                .await
                .context("Failed running Chrome headless")?;
            if !status.success() || !out_png.exists() {
                // 回退到常规 --headless
                let mut fallback_cmd = Command::new(executable);
                fallback_cmd
                    .arg("--headless")
                    .arg("--disable-gpu")
                    .arg("--no-first-run")
                    .arg("--disable-extensions")
                    .arg("--hide-scrollbars")
                    .arg("--mute-audio")
                    .arg("--default-background-color=00000000")
                    .arg(&window_size_arg)
                    .arg(&screenshot_arg)
                    .arg(&file_url)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                let fb_status = fallback_cmd.status().await?;
                let _ = tokio::fs::remove_file(&staged_file).await;
                if !fb_status.success() || !out_png.exists() {
                    bail!("Chrome headless failed to capture frame to {:?}", out_png);
                }
            } else {
                let _ = tokio::fs::remove_file(&staged_file).await;
            }
        }

        BrowserProvider::Mock => {
            // 纯 Rust 生成合成测试图像，包含平滑背景渐变、帧号与进度刻度
            generate_mock_frame(config.width, config.height, frame_index, progress, out_png)?;
        }
    }

    Ok(())
}

/// 捕获完整帧序列并保存为 PNG 图片列表
pub async fn capture_frame_sequence(
    html_path: &Path,
    start_frame: u64,
    end_frame: u64,
    config: &CaptureConfig,
    out_dir: &Path,
    provider: &BrowserProvider,
) -> Result<Vec<PathBuf>> {
    tokio::fs::create_dir_all(out_dir).await?;
    let total_frames = end_frame.saturating_sub(start_frame).max(1);

    #[cfg(unix)]
    if let BrowserProvider::Chrome { executable } = provider {
        let mut session = crate::chrome_session::ChromeSession::open(
            executable,
            html_path,
            config.width,
            config.height,
        )
        .await?;
        let mut paths = Vec::new();
        for frame in start_frame..end_frame {
            let out = out_dir.join(format!("frame_{:06}.png", frame));
            let progress = if total_frames > 1 {
                (frame - start_frame) as f64 / (total_frames - 1) as f64
            } else {
                0.0
            };
            session.frame(frame, progress, &out).await?;
            paths.push(out);
        }
        session.close().await?;
        return Ok(paths);
    }
    let mut captured_paths = Vec::new();

    for f in start_frame..end_frame {
        let file_name = format!("frame_{:06}.png", f);
        let out_path = out_dir.join(&file_name);
        capture_single_frame(html_path, f, total_frames, config, &out_path, provider).await?;
        captured_paths.push(out_path);
    }

    Ok(captured_paths)
}

/// 将 HTML 视听组件逐帧捕获并编码为广播级 MP4 视频
pub async fn render_html_to_mp4(
    html_path: &Path,
    total_frames: u64,
    config: &CaptureConfig,
    out_mp4: &Path,
    provider: &BrowserProvider,
) -> Result<()> {
    if let Some(parent) = out_mp4.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let temp_dir = tempfile::tempdir()?;
    let frames_dir = temp_dir.path().join("frames");
    tokio::fs::create_dir_all(&frames_dir).await?;

    // 1. 逐帧捕获
    capture_frame_sequence(html_path, 0, total_frames, config, &frames_dir, provider).await?;

    // 2. 调用 FFmpeg 将图像序列编码为视频
    // 若开启 transparent，根据容器格式保留 Alpha 通道 (MOV 采用 ProRes 4444 yuva444p10le，WEBM 采用 VP9 yuva420p)
    let pattern = frames_dir.join("frame_%06d.png");
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y")
        .arg("-v")
        .arg("error")
        .arg("-framerate")
        .arg(format!("{:.3}", config.fps))
        .arg("-i")
        .arg(pattern.to_string_lossy().to_string())
        .arg("-frames:v")
        .arg(format!("{}", total_frames))
        .arg("-an");

    if config.transparent {
        let ext = out_mp4
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("mp4");
        if ext == "mov" {
            cmd.arg("-c:v")
                .arg("prores_ks")
                .arg("-profile:v")
                .arg("4")
                .arg("-pix_fmt")
                .arg("yuva444p10le");
        } else if ext == "webm" {
            cmd.arg("-c:v")
                .arg("libvpx-vp9")
                .arg("-pix_fmt")
                .arg("yuva420p");
        } else {
            // 标准 MP4 容器使用 PNG 编码并保留 RGBA 像素格式，保持透明通道
            cmd.arg("-c:v").arg("png").arg("-pix_fmt").arg("rgba");
        }
    } else {
        cmd.arg("-c:v")
            .arg("libx264")
            .arg("-crf")
            .arg(format!("{}", config.crf))
            .arg("-preset")
            .arg(&config.preset)
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg("-vf")
            .arg("scale=out_color_matrix=bt709:out_range=tv,format=yuv420p,setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv");
    }

    cmd.arg("-movflags")
        .arg("+faststart")
        .arg(out_mp4.to_string_lossy().to_string());

    let status = cmd
        .status()
        .await
        .context("Failed executing FFmpeg encoder")?;
    if !status.success() || !out_mp4.exists() {
        bail!("FFmpeg failed to encode MP4 video to {:?}", out_mp4);
    }
    let mode = if matches!(provider, BrowserProvider::Mock) {
        crate::artifacts::Mode::Mock
    } else {
        crate::artifacts::Mode::Production
    };
    crate::artifacts::record(out_mp4, mode, "browser-capture")?;
    Ok(())
}

/// 扫描项目清单，为所有 visual_components 轨道项逐个收割并生成 MP4 视频
pub async fn batch_capture_project_components(
    manifest: &ProjectManifest,
    out_video_dir: &Path,
    config: &CaptureConfig,
    provider: &BrowserProvider,
) -> Result<ProjectManifest> {
    tokio::fs::create_dir_all(out_video_dir).await?;
    let mut updated = manifest.clone();
    let mut updated_items = Vec::new();

    for item in &manifest.tracks.visual_components {
        let file_path = item
            .payload
            .get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let html_path = PathBuf::from(file_path);
        if !html_path.exists() {
            // 若相对路径不存在，尝试在当前工作区探测
            eprintln!(
                "Warning: Component HTML {:?} not found, skipping capture",
                html_path
            );
            updated_items.push(item.clone());
            continue;
        }

        let total_frames = item
            .payload
            .get("duration_frames")
            .and_then(|v| v.as_u64())
            .unwrap_or(60);

        let out_video_name = format!("{}.mp4", item.id);
        let out_video_path = out_video_dir.join(&out_video_name);

        println!(
            "Capturing component [{}] {:?} -> {:?} ({} frames @ {:.1} fps)...",
            item.id, html_path, out_video_path, total_frames, config.fps
        );

        render_html_to_mp4(&html_path, total_frames, config, &out_video_path, provider).await?;

        let mut new_payload = item.payload.clone();
        if let Some(obj) = new_payload.as_object_mut() {
            obj.insert(
                "video_path".into(),
                serde_json::Value::String(out_video_path.to_string_lossy().to_string()),
            );
            obj.insert("captured".into(), serde_json::Value::Bool(true));
        }

        let mut new_item = item.clone();
        new_item.payload = new_payload;
        updated_items.push(new_item);
    }

    updated.tracks.visual_components = updated_items;
    Ok(updated)
}

// ============================================================================
// 4. Mock 图像合成算法 (纯 Rust 离线安全兜底)
// ============================================================================

fn generate_mock_frame(
    width: u32,
    height: u32,
    _frame_index: u64,
    progress: f64,
    out_png: &Path,
) -> Result<()> {
    let mut img = RgbImage::new(width, height);

    // 绘制基于深海科技质感的背景梯度与进度条
    let bar_width = ((width as f64 - 80.0) * progress) as u32;

    for y in 0..height {
        let y_ratio = y as f64 / height as f64;
        let r = (17.0 + y_ratio * 15.0) as u8;
        let g = (24.0 + y_ratio * 12.0) as u8;
        let b = (39.0 + y_ratio * 30.0) as u8;

        for x in 0..width {
            let mut pixel_r = r;
            let mut pixel_g = g;
            let mut pixel_b = b;

            // 绘制下沿进度槽 (Progress bar)
            if y >= height - 20 && y <= height - 12 && x >= 40 && x <= width - 40 {
                if x <= 40 + bar_width {
                    pixel_r = 217;
                    pixel_g = 155;
                    pixel_b = 117; // #d99b75 琥珀金
                } else {
                    pixel_r = 60;
                    pixel_g = 55;
                    pixel_b = 96;
                }
            }

            img.put_pixel(x, y, Rgb([pixel_r, pixel_g, pixel_b]));
        }
    }

    img.save(out_png)
        .with_context(|| format!("Failed to save mock frame to {:?}", out_png))?;
    Ok(())
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_chrome_executable() {
        let detected = detect_chrome_executable();
        // 在 macOS 本机应当能够探测到 Chrome，但在 CI 环境可能没有，无论如何都不 panic
        if let Some(path) = detected {
            assert!(path.exists());
        }
    }

    #[tokio::test]
    async fn test_mock_browser_capture_single_frame() {
        let temp_dir = tempfile::tempdir().unwrap();
        let dummy_html = temp_dir.path().join("dummy.html");
        tokio::fs::write(&dummy_html, "<html><body><h1>Test</h1></body></html>")
            .await
            .unwrap();

        let out_png = temp_dir.path().join("out.png");
        let config = CaptureConfig {
            width: 320,
            height: 180,
            ..Default::default()
        };

        capture_single_frame(
            &dummy_html,
            5,
            30,
            &config,
            &out_png,
            &BrowserProvider::Mock,
        )
        .await
        .expect("Capture mock frame");

        assert!(out_png.exists());
        let meta = std::fs::metadata(&out_png).unwrap();
        assert!(meta.len() > 100);
    }

    #[tokio::test]
    async fn test_mock_capture_sequence_and_encode_mp4() {
        let temp_dir = tempfile::tempdir().unwrap();
        let dummy_html = temp_dir.path().join("dummy.html");
        tokio::fs::write(&dummy_html, "<html><body><h1>Animation</h1></body></html>")
            .await
            .unwrap();

        let out_mp4 = temp_dir.path().join("rendered.mp4");
        let config = CaptureConfig {
            width: 320,
            height: 180,
            fps: 30.0,
            ..Default::default()
        };

        // 渲染 6 帧小片段
        render_html_to_mp4(&dummy_html, 6, &config, &out_mp4, &BrowserProvider::Mock)
            .await
            .expect("Render MP4 from mock frames");

        assert!(out_mp4.exists());
        let meta = std::fs::metadata(&out_mp4).unwrap();
        assert!(meta.len() > 1000);
    }

    #[tokio::test]
    async fn test_batch_capture_project_components_manifest_update() {
        let temp_dir = tempfile::tempdir().unwrap();
        let dummy_html = temp_dir.path().join("shot_01.html");
        tokio::fs::write(&dummy_html, "<html><body><h1>Shot 1</h1></body></html>")
            .await
            .unwrap();

        let manifest = ProjectManifest {
            master_audio: None,
            format: "clone-any-video.project.v1".into(),
            meta: crate::project::ProjectMeta {
                id: "test".into(),
                title: "Test".into(),
                width: 320,
                height: 180,
                fps: 30.0,
                total_seconds: 2.0,
                total_frames: 60,
            },
            shots: vec![],
            tracks: crate::project::LayerTracks {
                visual_bg: vec![],
                visual_components: vec![TrackItem {
                    id: "comp_01".into(),
                    name: "Shot 1 Component".into(),
                    start_seconds: 0.0,
                    end_seconds: 2.0,
                    start_frame: 0,
                    end_frame: 60,
                    payload: serde_json::json!({
                        "file_path": dummy_html.to_string_lossy(),
                        "duration_frames": 4,
                    }),
                }],
                captions: vec![],
                voice: vec![],
                sfx: vec![],
                bgm: vec![],
            },
            asset_tasks: vec![],
        };

        let out_videos = temp_dir.path().join("videos");
        let config = CaptureConfig {
            width: 320,
            height: 180,
            fps: 30.0,
            ..Default::default()
        };

        let updated = batch_capture_project_components(
            &manifest,
            &out_videos,
            &config,
            &BrowserProvider::Mock,
        )
        .await
        .expect("Batch capture");

        assert_eq!(updated.tracks.visual_components.len(), 1);
        let captured = updated.tracks.visual_components[0]
            .payload
            .get("captured")
            .and_then(|v| v.as_bool());
        assert_eq!(captured, Some(true));
        assert!(out_videos.join("comp_01.mp4").exists());
    }
}
