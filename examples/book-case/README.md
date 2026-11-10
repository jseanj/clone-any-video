# 全书固定案例

从仓库根目录运行 `python3 scripts/book_demo.py all`。案例副本和全部产物保存在 `output/book-demo/`；源输入不会被修改。

分步命令为 init、read、plan、audio、render、verify。audio 先构建 clock，再由 render 明确选入该 Result 复用配音。输出时长依本机声音而变化，以 project 和 ffprobe 为准。

本案例只验证教学流程，不需要模型 API、Chrome 或 Python 模型库；首次 Rust 编译仍需下载依赖。所有媒体均标记 mock。原片参考在 fixtures/origin.mp4；BRIEF.md、script.svml、layout.json、四份 timing.mock.json、author.json 和 run*.json 构成固定输入。
