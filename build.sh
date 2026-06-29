#!/bin/bash
# 构建脚本 - 自动更新版本号并构建项目

set -e

cd "$(dirname "$0")"

# 获取当前短 commit hash
COMMIT_HASH=$(git rev-parse HEAD)

# 获取当前版本号
VERSION=$(grep '^version = ' Cargo.toml | sed 's/version = "\([^"]*\)"/\1/')

# 移除旧的 commit hash（如果存在）
BASE_VERSION=$(echo "$VERSION" | sed 's/+\([a-f0-9]*\)$//')

# 构建新的版本号
NEW_VERSION="${BASE_VERSION}+${COMMIT_HASH}"

# 更新主项目 Cargo.toml
sed -i "s/^version = \"[^\"]*\"/version = \"${NEW_VERSION}\"/" Cargo.toml

echo "✅ 主项目版本号已更新为: ${NEW_VERSION}"

# 更新 lsql Cargo.toml
sed -i "s/^version = \"[^\"]*\"/version = \"${NEW_VERSION}\"/" lsql/Cargo.toml

echo "✅ lsql 版本号已更新为: ${NEW_VERSION}"

# 更新 Dockerfile.prod 中的版本号（使用版本号前半段）
sed -i "s/^ARG VERSION=[0-9.]*$/ARG VERSION=${BASE_VERSION}/" Dockerfile.prod
echo "✅ Dockerfile.prod 版本号已更新为: ${BASE_VERSION}"

# 构建项目
echo "🚀 开始构建项目..."
cargo build --release --bin laoflchdb

echo "✅ 构建完成！"
