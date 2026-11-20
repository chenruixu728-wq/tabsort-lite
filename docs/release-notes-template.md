TabSort Lite {{VERSION}} · Edge 正式版

Windows x64 本地标签页整理工具：读取已连接的 Edge 配置，按标题和网址生成主题，按 1–6 个窗口预览并整理，支持本地快照恢复；也可选接入 DeepSeek 或兼容 Chat Completions 的接口做 AI 再分组。

## 下载
- `TabSortLite-Edge-{{VERSION}}.zip`：完整运行包（EXE + 扩展 + 模型 + ONNX Runtime），解压即用。
- `TabSortLite-Edge-{{VERSION}}-Source.zip`：源码快照。
- `TabSortLite-Edge-{{VERSION}}-SHA256.txt`：以上文件与包内 EXE 的 SHA-256 校验值。

## 使用
1. 解压整个文件夹，双击 `TabSortLite.exe`，点「注册 Edge 连接」。
2. 在 Edge 打开 `edge://extensions`，开启开发人员模式，选择「加载解压缩的扩展」，选中包内 `extension` 文件夹。
3. 点击 Edge 工具栏的 TabSort Lite Connector 完成连接。

说明：EXE 未代码签名，扩展需开发人员模式加载；系统需 Microsoft Visual C++ x64 运行库。详细限制见包内 README。

## 许可
MIT，见仓库 LICENSE。内置 E5 模型与 ONNX Runtime 同为 MIT，许可文本随包提供。
