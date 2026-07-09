# IPC 实体对齐表

重构期间 Rust <-> TypeScript 实体字段映射，用于 PR 验收静态检查。

| Rust struct | TS interface | 字段映射说明 |
|-------------|-------------|-------------|
| `Session` | `Session` | `pid: u32` -> `pid: number`；`status: SessionStatus` -> `status: string` |
| `SessionsResponse` | `SessionsResponse` | `sessions: Vec<Session>` -> `sessions: Session[]` |
| `ExtensionRecord` | `Extension` | `id/kind/name: String` -> `string`；`serde(rename_all = "camelCase")` |
| `AssignmentRecord` | `Assignment` | 同上 camelCase 映射 |
| `PresetRecord` | `Preset` | 同上 camelCase 映射 |
| `PresetItemRecord` | `PresetItem` | 同上 |
| `SubAgentRecord` | `SubAgent` | 同上 |
| `NativeExtensionRecord` | `NativeExtension` | 同上 |
| `ImportStats` | `ImportStats` | `imported/newly_added/skipped_dup: usize` -> `number`；`source_counts: Vec<(String, usize)>` -> `[string, number][]` |
| `ApplyResult` | `PresetApplyResult` | `success_count/fail_count: usize` -> `number`；`failures: Vec<String>` -> `string[]` |
| `CompatibilityReport` | `CompatibilityReport` | `compatible/incompatible: Vec` -> `[]` |
| `ToolDetection` | `ToolDetection` | `tool_id/name/detected` 等 camelCase 映射 |
| `ScreenshotResult` | `ScreenshotResult` | `path/success` 等 camelCase 映射 |
