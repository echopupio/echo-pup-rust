#!/bin/bash
# 下载 sherpa-onnx 离线标点恢复模型
# 用法: ./scripts/download_punctuation_model.sh

set -u

HOME_DIR="${HOME:-$(cd ~ && pwd)}"
MODEL_DIR="${HOME_DIR}/.echopup/models/punctuation"
mkdir -p "$MODEL_DIR"

MODEL_FILE="${MODEL_DIR}/model.onnx"
TMP_FILE="${MODEL_FILE}.part"

# sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12
ARCHIVE_NAME="sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12"
ARCHIVE_URL="https://github.com/k2-fsa/sherpa-onnx/releases/download/punctuation-models/${ARCHIVE_NAME}.tar.bz2"

echo "标点模型目录: ${MODEL_DIR}"
echo "目标文件: ${MODEL_FILE}"

if [ -f "$MODEL_FILE" ] && [ -s "$MODEL_FILE" ]; then
    echo "标点模型已存在:"
    ls -lh "$MODEL_FILE"
    exit 0
fi

if [ -f "$MODEL_FILE" ] && [ ! -s "$MODEL_FILE" ]; then
    echo "发现空文件，已删除: $MODEL_FILE"
    rm -f "$MODEL_FILE"
fi

echo "下载地址: ${ARCHIVE_URL}"
echo "开始下载..."

ARCHIVE_FILE="/tmp/${ARCHIVE_NAME}.tar.bz2"

curl -fL -C - \
  --retry 5 \
  --retry-delay 5 \
  --connect-timeout 20 \
  --max-time 0 \
  -o "$ARCHIVE_FILE" \
  "$ARCHIVE_URL"

CURL_EXIT=$?

if [ $CURL_EXIT -ne 0 ] || [ ! -s "$ARCHIVE_FILE" ]; then
    echo "下载失败，curl exit code: $CURL_EXIT"
    exit 1
fi

echo "解压中..."
tar -xjf "$ARCHIVE_FILE" -C /tmp/

EXTRACTED_MODEL="/tmp/${ARCHIVE_NAME}/model.onnx"
if [ ! -f "$EXTRACTED_MODEL" ]; then
    echo "解压后未找到 model.onnx"
    ls -la "/tmp/${ARCHIVE_NAME}/"
    exit 1
fi

cp "$EXTRACTED_MODEL" "$MODEL_FILE"
rm -rf "/tmp/${ARCHIVE_NAME}" "$ARCHIVE_FILE"

echo "下载完成:"
ls -lh "$MODEL_FILE"
