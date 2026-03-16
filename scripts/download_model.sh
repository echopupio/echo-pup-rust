#!/bin/bash
# 下载 Whisper 模型的脚本

MODEL_DIR="$(dirname "$0")/../models"
mkdir -p "$MODEL_DIR"

# 模型大小选项: tiny, base, small, medium, large
MODEL_SIZE="${1:-small}"
MODEL_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-${MODEL_SIZE}.bin"
MODEL_FILE="$MODEL_DIR/ggml-${MODEL_SIZE}.bin"

if [ -f "$MODEL_FILE" ]; then
    echo "模型已存在: $MODEL_FILE"
    exit 0
fi

echo "下载 Whisper ${MODEL_SIZE} 模型..."
curl -L -o "$MODEL_FILE" "$MODEL_URL"

if [ $? -eq 0 ]; then
    echo "下载完成!"
    ls -lh "$MODEL_FILE"
else
    echo "下载失败"
    rm -f "$MODEL_FILE"
    exit 1
fi
