#!/bin/bash
# 下载 whisper.cpp 模型脚本
# 用法:
#   ./scripts/download_model.sh
#   ./scripts/download_model.sh small
#   ./scripts/download_model.sh large
#   ./scripts/download_model.sh large-v3
#   ./scripts/download_model.sh turbo

set -u

HOME_DIR="${HOME:-$(cd ~ && pwd)}"
MODEL_DIR="${HOME_DIR}/.echopup/models"
mkdir -p "$MODEL_DIR"

INPUT_SIZE="${1:-large-v3}"

case "$INPUT_SIZE" in
  tiny)
    MODEL_FILE_NAME="ggml-tiny.bin"
    ;;
  base)
    MODEL_FILE_NAME="ggml-base.bin"
    ;;
  small)
    MODEL_FILE_NAME="ggml-small.bin"
    ;;
  medium)
    MODEL_FILE_NAME="ggml-medium.bin"
    ;;
  large)
    # 兼容旧习惯：large 自动映射到当前可用的 large-v3
    MODEL_FILE_NAME="ggml-large-v3.bin"
    ;;
  large-v3)
    MODEL_FILE_NAME="ggml-large-v3.bin"
    ;;
  turbo|large-v3-turbo)
    MODEL_FILE_NAME="ggml-large-v3-turbo.bin"
    ;;
  *)
    echo "不支持的模型大小: $INPUT_SIZE"
    echo "可选值: tiny, base, small, medium, large, large-v3, turbo"
    exit 1
    ;;
esac

MODEL_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/${MODEL_FILE_NAME}"
MODEL_FILE="${MODEL_DIR}/${MODEL_FILE_NAME}"
TMP_FILE="${MODEL_FILE}.part"

echo "模型目录: $MODEL_DIR"
echo "输入参数: $INPUT_SIZE"
echo "实际文件: $MODEL_FILE_NAME"
echo "下载地址: $MODEL_URL"

# 正式文件存在且非空，直接退出
if [ -f "$MODEL_FILE" ] && [ -s "$MODEL_FILE" ]; then
    echo "模型已存在:"
    ls -lh "$MODEL_FILE"
    exit 0
fi

# 正式文件存在但为空，删除
if [ -f "$MODEL_FILE" ] && [ ! -s "$MODEL_FILE" ]; then
    echo "发现空文件，已删除: $MODEL_FILE"
    rm -f "$MODEL_FILE"
fi

echo "开始下载..."
echo "支持断点续传，如中断可重新执行同一命令继续下载。"

curl -fL -C - \
  --retry 10 \
  --retry-delay 5 \
  --connect-timeout 20 \
  --max-time 0 \
  -o "$TMP_FILE" \
  "$MODEL_URL"

CURL_EXIT=$?

if [ $CURL_EXIT -eq 0 ] && [ -s "$TMP_FILE" ]; then
    mv -f "$TMP_FILE" "$MODEL_FILE"
    echo "下载完成:"
    ls -lh "$MODEL_FILE"
    exit 0
else
    echo "下载失败，curl exit code: $CURL_EXIT"
    if [ -f "$TMP_FILE" ]; then
        echo "当前临时文件状态:"
        ls -lh "$TMP_FILE"
        echo "可重新执行脚本继续下载。"
    fi
    exit 1
fi
