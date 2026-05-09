# syntax=docker/dockerfile:1
#
# Whisper fine-tuning service.
# CPU-only by default; override base image for GPU support (see compose.gpu.yml).
#
# This container is only started during a voice personalisation training job.
# It exits when the job completes; normal assistant operation never requires it.

FROM python:3.11-slim-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
        ffmpeg \
        git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY app.py train_whisper.py ./

ENV CONFIG_DIR=/data/config \
    WHISPER_BASE_MODEL=openai/whisper-small

EXPOSE 8000

CMD ["uvicorn", "app:app", "--host", "0.0.0.0", "--port", "8000"]
