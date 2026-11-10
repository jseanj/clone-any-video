//! Isolated, persistent Chrome CDP pipe. One page renders an entire frame range.
//! https://chromedevtools.github.io/devtools-protocol/tot/Page/#method-captureScreenshot
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{
    os::{fd::AsRawFd, unix::net::UnixStream},
    path::Path,
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream as AsyncStream,
    process::{Child, Command},
};

pub struct ChromeSession {
    process: Child,
    writer: AsyncStream,
    reader: BufReader<AsyncStream>,
    next_id: u64,
    session: Option<String>,
    loaded: bool,
    _profile: tempfile::TempDir,
}
impl ChromeSession {
    pub async fn open(executable: &Path, html: &Path, width: u32, height: u32) -> Result<Self> {
        let (child_in, write) = UnixStream::pair()?;
        let (read, child_out) = UnixStream::pair()?;
        let input = child_in.as_raw_fd();
        let output = child_out.as_raw_fd();
        let profile = tempfile::tempdir()?;
        let mut cmd = Command::new(executable);
        cmd.args([
            "--headless=new",
            "--remote-debugging-pipe",
            "--disable-gpu",
            "--no-first-run",
            "--no-sandbox",
            "--disable-extensions",
            "--disable-background-networking",
            "--allow-file-access-from-files",
            "--hide-scrollbars",
            "--mute-audio",
        ])
        .arg(format!("--user-data-dir={}", profile.path().display()))
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
        // SAFETY: only async-signal-safe fcntl/dup2/close operations run after fork.
        // Copy descriptors above 9 first to avoid collisions with the CDP fd3/fd4 ABI.
        unsafe {
            cmd.pre_exec(move || {
                let rx = libc::fcntl(input, libc::F_DUPFD_CLOEXEC, 10);
                if rx < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                let tx = libc::fcntl(output, libc::F_DUPFD_CLOEXEC, 10);
                if tx < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::dup2(rx, 3) < 0 || libc::dup2(tx, 4) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                libc::close(rx);
                libc::close(tx);
                Ok(())
            });
        }
        let process = cmd.spawn().context("Launch persistent Chrome")?;
        drop(child_in);
        drop(child_out);
        write.set_nonblocking(true)?;
        read.set_nonblocking(true)?;
        let mut this = Self {
            process,
            writer: AsyncStream::from_std(write)?,
            reader: BufReader::new(AsyncStream::from_std(read)?),
            next_id: 0,
            session: None,
            loaded: false,
            _profile: profile,
        };
        let target = this
            .call("Target.createTarget", json!({"url":"about:blank"}))
            .await?;
        let attach = this
            .call(
                "Target.attachToTarget",
                json!({"targetId":target["targetId"],"flatten":true}),
            )
            .await?;
        this.session = Some(
            attach["sessionId"]
                .as_str()
                .context("Chrome session ID missing")?
                .into(),
        );
        this.call("Page.enable", json!({})).await?;
        this.call("Runtime.enable", json!({})).await?;
        this.call(
            "Emulation.setDeviceMetricsOverride",
            json!({"width":width,"height":height,"deviceScaleFactor":1,"mobile":false}),
        )
        .await?;
        this.call(
            "Emulation.setDefaultBackgroundColorOverride",
            json!({"color":{"r":0,"g":0,"b":0,"a":0}}),
        )
        .await?;
        this.loaded = false;
        // Url handles spaces/non-ASCII in file names, unlike hand-built file:// strings.
        let url = reqwest::Url::from_file_path(html.canonicalize()?)
            .map_err(|_| anyhow::anyhow!("Invalid HTML path"))?;
        let result = this
            .call("Page.navigate", json!({"url":url.as_str()}))
            .await?;
        if result.get("errorText").is_some() {
            bail!("Chrome navigation failed: {}", result);
        }
        tokio::time::timeout(Duration::from_secs(30), async {
            while !this.loaded {
                this.receive().await?;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("HTML load timeout")??;
        this.evaluate("Promise.all([document.fonts.ready,...Array.from(document.images).map(i=>i.decode())]).then(()=>true)",true).await?;
        Ok(this)
    }
    async fn receive(&mut self) -> Result<Value> {
        let mut bytes = Vec::new();
        let read = self.reader.read_until(0, &mut bytes).await?;
        if read == 0 {
            bail!("Chrome CDP pipe closed unexpectedly");
        }
        if bytes.last() == Some(&0) {
            bytes.pop();
        }
        let v: Value = serde_json::from_slice(&bytes)?;
        if v["method"] == "Page.loadEventFired"
            && v.get("sessionId") == self.session.as_ref().map(|s| json!(s)).as_ref()
        {
            self.loaded = true;
        }
        Ok(v)
    }
    async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        let mut request = json!({"id":id,"method":method,"params":params});
        if let Some(s) = &self.session {
            request["sessionId"] = json!(s);
        }
        let mut bytes = serde_json::to_vec(&request)?;
        bytes.push(0);
        self.writer.write_all(&bytes).await?;
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let v = self.receive().await?;
                if v["id"] == id {
                    if let Some(error) = v.get("error") {
                        bail!("CDP {}: {}", method, error);
                    }
                    return Ok(v["result"].clone());
                }
            }
        })
        .await
        .with_context(|| format!("CDP timeout: {}", method))?
    }
    async fn evaluate(&mut self, expression: &str, await_promise: bool) -> Result<Value> {
        let v = self
            .call(
                "Runtime.evaluate",
                json!({"expression":expression,"awaitPromise":await_promise,"returnByValue":true}),
            )
            .await?;
        if v.get("exceptionDetails").is_some() {
            bail!("HTML frame evaluation failed: {}", v);
        }
        Ok(v)
    }
    pub async fn frame(&mut self, frame: u64, progress: f64, output: &Path) -> Result<()> {
        self.evaluate(&format!("(async()=>{{if(typeof window.renderFrame!=='function')throw new Error('HTML must implement renderFrame(frame, progress)'); await window.renderFrame({},{});await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));return true;}})()",frame,progress),true).await?;
        let result = self
            .call(
                "Page.captureScreenshot",
                json!({"format":"png","fromSurface":true,"captureBeyondViewport":false}),
            )
            .await?;
        let bytes = STANDARD.decode(result["data"].as_str().context("Chrome returned no PNG")?)?;
        tokio::fs::write(output, bytes).await?;
        Ok(())
    }
    pub async fn close(mut self) -> Result<()> {
        let _ = self.process.kill().await;
        let _ = self.process.wait().await;
        Ok(())
    }
}
