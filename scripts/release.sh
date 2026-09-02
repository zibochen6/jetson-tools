#!/bin/sh
# 本地发布脚本（CI 未跑通时的替代品；正式流程是打 tag 推 GitHub 触发 Actions）
#
# 用法: scripts/release.sh <version>     例如 0.2.1
# 前置: gh 已登录；xcode CLT；brew freerdp（bridge 依赖）
# 流程: 构建 dmg+app → 打 app.tar.gz → gh release create v<version>
#
set -e

V="${1:?用法: scripts/release.sh <version>}"
TAG="v$V"
REPO="zibochen6/jetson-tools"

echo "==> 提示: 请确认已同步 src-tauri/Cargo.toml、tauri.conf.json、package.json 的版本为 $V"
read -r -p "确认继续? [y/N] " ok
[ "$ok" = "y" ] || [ "$ok" = "Y" ] || exit 1

echo "==> 构建 (app 自建环回隧道, KI-021)"
pnpm tauri build

echo "==> 打 updater 资产"
tar -czf "/tmp/Jetson Remote.app.tar.gz" \
  -C src-tauri/target/release/bundle/macos "Jetson Remote.app"

DMG=$(ls src-tauri/target/release/bundle/dmg/Jetson\ Remote_*.dmg | head -1)
echo "==> 发布 $TAG"
gh release create "$TAG" "$DMG" "/tmp/Jetson Remote.app.tar.gz" \
  --repo "$REPO" --generate-notes --title "$TAG"

echo "==> 完成: https://github.com/$REPO/releases/tag/$TAG"