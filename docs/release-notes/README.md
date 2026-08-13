# Release Notes

每次发布一个版本，在仓库里维护一个该版本的更新说明文件。

- 文件名：`v<版本号>.md`，例如 `v0.2.3.md`
- 内容只写"本次更新内容"（变更/修复/优化），**不要**写下载清单（发布流水线会自动生成下载段）
- 发布流程：
  1. 写 `vX.Y.Z.md` 并单独提交（`docs: add release notes vX.Y.Z`）
  2. 运行 `pnpm release:version`，选择同一版本号（它会更新版本文件、打 tag、push）
  3. CI 自动构建并发布到 GitHub Releases（正文含本文件内容 + 下载清单）
