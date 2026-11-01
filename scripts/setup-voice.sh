#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
uv venv --python 3.12 .work/voice-venv
uv pip install --python .work/voice-venv/bin/python faster-whisper==1.2.1 av==16.1.0
.work/voice-venv/bin/python -c 'from huggingface_hub import snapshot_download; snapshot_download("Systran/faster-whisper-base")'
