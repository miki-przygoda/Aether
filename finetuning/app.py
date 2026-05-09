"""
Finetuning microservice — FastAPI wrapper around train_whisper.py.

Endpoints:
  POST /train   — accepts multipart/form-data with user_id + audio samples,
                  runs fine-tuning synchronously, returns JSON status.
  GET  /health  — liveness probe.

The service is intentionally single-threaded (one job at a time).
brain-node serialises calls and only starts this container during a training job.
"""

import os
import tempfile
import threading
from pathlib import Path
from typing import Optional

from fastapi import FastAPI, UploadFile, Form, File, HTTPException
from fastapi.responses import JSONResponse

from train_whisper import finetune

app = FastAPI(title="Aether Finetuning Service")

_lock = threading.Lock()
_job_state: dict = {"running": False, "percent": 0, "message": "idle", "error": None}


@app.get("/health")
def health():
    return {"status": "ok"}


@app.get("/status")
def status():
    return dict(_job_state)


@app.post("/train")
async def train(
    user_id: str = Form(...),
    samples: list[UploadFile] = File(...),
):
    if not _lock.acquire(blocking=False):
        raise HTTPException(status_code=409, detail="A training job is already running")

    try:
        _job_state.update(running=True, percent=0, message="Starting…", error=None)

        with tempfile.TemporaryDirectory() as tmpdir:
            tmp = Path(tmpdir)
            sample_list = []

            for i, upload in enumerate(samples):
                dest = tmp / f"sample_{i:04d}.wav"
                dest.write_bytes(await upload.read())
                # transcript carried in filename as URL-encoded field; fall back to empty
                transcript = upload.filename or ""
                sample_list.append({"audio_path": str(dest), "transcript": transcript})

            def _progress(pct: int, msg: str):
                _job_state.update(percent=pct, message=msg)

            finetune(user_id, sample_list, progress_cb=_progress)

        _job_state.update(running=False, percent=100, message="Complete")
        return JSONResponse({"status": "complete", "user_id": user_id})

    except Exception as exc:
        _job_state.update(running=False, percent=0, message="Failed", error=str(exc))
        raise HTTPException(status_code=500, detail=str(exc)) from exc
    finally:
        _lock.release()
