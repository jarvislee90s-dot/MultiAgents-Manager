#!/usr/bin/env bash
# release.sh - 打包软件并归档到 release/ 目录
# 用法: ./scripts/release.sh [version]
# 如果省略 version，从 tauri.conf.json 自动读取

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$PROJECT_DIR"

# 获取版本号
VERSION="${1:-$(python3 -c "import json; print(json.load(open('src-tauri/tauri.conf.json'))['version'])")}"

echo "🔨 Building v$VERSION..."
pnpm tauri build

DMG_SRC="$PROJECT_DIR/src-tauri/target/release/bundle/dmg/MultiAgents Manager_${VERSION}_aarch64.dmg"
APP_SRC="$PROJECT_DIR/src-tauri/target/release/bundle/macos/MultiAgents Manager.app"
TAR_SRC="$PROJECT_DIR/src-tauri/target/release/bundle/macos/MultiAgents Manager.app.tar.gz"

RELEASE_DIR="$PROJECT_DIR/release"
mkdir -p "$RELEASE_DIR"

echo "📦 Copying artifacts to $RELEASE_DIR/..."

if [ -f "$DMG_SRC" ]; then
  cp "$DMG_SRC" "$RELEASE_DIR/"
  echo "  ✅ DMG: MultiAgents Manager_${VERSION}_aarch64.dmg"
else
  echo "  ⚠️  DMG not found: $DMG_SRC"
fi

if [ -f "$TAR_SRC" ]; then
  cp "$TAR_SRC" "$RELEASE_DIR/"
  echo "  ✅ TAR: MultiAgents Manager.app.tar.gz"
else
  echo "  ⚠️  TAR not found: $TAR_SRC"
fi

# 生成 release notes 模板（如果不存在）
NOTES_FILE="$RELEASE_DIR/release-notes-v${VERSION}.md"
if [ ! -f "$NOTES_FILE" ]; then
  cat > "$NOTES_FILE" << EOF
# Release Notes

## v${VERSION} ($(date +%Y-%m-%d))

### 🚀 新增

### 🔧 优化

### 🐛 修复

### 📦 打包

- macOS DMG: \`MultiAgents Manager_${VERSION}_aarch64.dmg\`
- macOS .app: \`MultiAgents Manager.app\`
EOF
  echo "  📝 Created release notes template: $NOTES_FILE"
  echo "  ⚠️  请编辑 $NOTES_FILE 补充变更内容后提交"
fi

echo ""
echo "✅ Release v$VERSION ready in $RELEASE_DIR/"
ls -lh "$RELEASE_DIR/"
