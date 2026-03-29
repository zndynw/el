#!/bin/bash

set -euo pipefail

TARGET_TRIPLE="x86_64-unknown-linux-gnu.2.17"
FALLBACK_TARGET_TRIPLE="x86_64-unknown-linux-gnu"
OUTPUT_DIR="./target/glibc217"
BINARY_NAME="el"
OUTPUT_BINARY="${OUTPUT_DIR}/${BINARY_NAME}"

export ZIG_LOCAL_CACHE_DIR="./target/zig-local-cache"
export ZIG_GLOBAL_CACHE_DIR="./target/zig-global-cache"

echo "=== Building el for Linux x86_64 glibc 2.17 ==="

mkdir -p "${OUTPUT_DIR}"

echo "Running cargo zigbuild..."
cargo zigbuild --release --target "${TARGET_TRIPLE}"

if [[ -f "./target/${TARGET_TRIPLE}/release/${BINARY_NAME}" ]]; then
  BUILT_BINARY="./target/${TARGET_TRIPLE}/release/${BINARY_NAME}"
elif [[ -f "./target/${FALLBACK_TARGET_TRIPLE}/release/${BINARY_NAME}" ]]; then
  BUILT_BINARY="./target/${FALLBACK_TARGET_TRIPLE}/release/${BINARY_NAME}"
else
  echo "Built binary not found under target/${TARGET_TRIPLE} or target/${FALLBACK_TARGET_TRIPLE}" >&2
  exit 1
fi

cp "${BUILT_BINARY}" "${OUTPUT_BINARY}"

echo "Compressing binary with upx..."
upx --best --lzma "${OUTPUT_BINARY}"

echo "=== Build completed successfully ==="
echo "Binary location: ${OUTPUT_BINARY}"
stat --printf="Compressed size: %s bytes\n" "${OUTPUT_BINARY}"
