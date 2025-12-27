#!/bin/bash

# CentOS 7 Docker编译脚本
# 用于在Docker容器中编译适用于CentOS 7的二进制程序

set -e

echo "=== Building el for CentOS 7 using Docker ==="

# 创建输出目录
OUTPUT_DIR="./target/centos7"
mkdir -p "$OUTPUT_DIR"

# 构建Docker镜像
echo "Building Docker image..."
docker build -t el-centos7-builder .

# 运行容器并复制编译结果
echo "Compiling project in Docker container..."
docker run --rm -v "$(pwd)/$OUTPUT_DIR:/output" el-centos7-builder sh -c "cp /build/target/release/el /output/"

echo "=== Build completed successfully ==="
echo "Binary location: $OUTPUT_DIR/el"
echo ""
echo "To verify the binary:"
echo "  file $OUTPUT_DIR/el"
echo "  ldd $OUTPUT_DIR/el"
