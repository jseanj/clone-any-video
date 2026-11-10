# 生产任务图离线示例

从仓库根目录运行：

```bash
cargo build -p ch16_video_agent_final
./target/debug/ch16_video_agent_final plan examples/production-graph/run.json --mock
./target/debug/ch16_video_agent_final build examples/production-graph/run.svrun --mock --output output/graph-demo.mp4
```

Author 声明 voice/transcript → align-speech → project-timing → mix、商品参考 → image → video、script/clock → caption-schedule → render-captions，最后 compose → review。JSON 与 XML Run 选相同目标。`product.png` 是合成色块夹具；`transcript.mock.json` 的时间也是教学夹具，均不是真实生产证据。全部命令必须使用 `--mock`；Flow project ID 仅为占位符。

构建后打印 Result 路径。把该绝对路径填入 `run.json` 的 `reuse`，再次 plan 可看到所有任务复用。只修改 `script.svml` 双文本左侧 Display，voice/mix/image/video 仍复用；修改右侧 Spoken、商品图或字幕 recipe 会影响对应分支。`python3 scripts/test_production_graph.py` 在隔离临时目录验证这些行为，不改此示例。

完整协议与正式生产边界见 [production-graph.md](../../docs/production-graph.md)。

`local-align` 的词时序仍来自明确标注的教学夹具。`align` 输出共享 clock 与 project；schedule 消费这份 clock。正式生产用同一音频的转录/校订证据替换夹具，细节见 [质量时序与采样](../../docs/quality-timing-and-sampling.md)。
