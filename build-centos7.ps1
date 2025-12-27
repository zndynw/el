# CentOS 7 Docker编译脚本 (PowerShell版本)
# 用于在Docker容器中编译适用于CentOS 7的二进制程序

$ErrorActionPreference = "Stop"

Write-Host "=== Building el for CentOS 7 using Docker ===" -ForegroundColor Green

# 创建输出目录
$OUTPUT_DIR = ".\target\centos7"
New-Item -ItemType Directory -Force -Path $OUTPUT_DIR | Out-Null

# 构建Docker镜像
Write-Host "Building Docker image..." -ForegroundColor Yellow
docker build -t el-centos7-builder .

if ($LASTEXITCODE -ne 0) {
    Write-Host "Docker build failed!" -ForegroundColor Red
    exit 1
}

# 运行容器编译并复制结果
Write-Host "Compiling project in Docker container..." -ForegroundColor Yellow
$absolutePath = (Resolve-Path $OUTPUT_DIR).Path
docker run --rm -v "${absolutePath}:/output" el-centos7-builder

if ($LASTEXITCODE -ne 0) {
    Write-Host "Docker run failed!" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "=== Build completed successfully ===" -ForegroundColor Green
Write-Host "Binary location: $OUTPUT_DIR\el" -ForegroundColor Cyan
Write-Host ""
Write-Host "To verify the binary on Linux:" -ForegroundColor Yellow
Write-Host "  file $OUTPUT_DIR/el"
Write-Host "  ldd $OUTPUT_DIR/el"
