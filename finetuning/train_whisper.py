"""
Whisper fine-tuning script using LoRA / PEFT.

Called by app.py with a list of (audio_path, transcript) pairs for a single user.
Saves the merged GGUF-ready model to <config_dir>/voice_models/<user_id>/.
"""

import os
import json
import tempfile
import shutil
from pathlib import Path
from typing import Generator

import torch
import torchaudio
from datasets import Dataset, Audio
from transformers import (
    WhisperProcessor,
    WhisperForConditionalGeneration,
)
from peft import LoraConfig, get_peft_model, TaskType
import evaluate

BASE_MODEL = os.getenv("WHISPER_BASE_MODEL", "openai/whisper-small")
CONFIG_DIR = Path(os.getenv("CONFIG_DIR", "/data/config"))

wer_metric = evaluate.load("wer")


def _load_audio_as_array(path: str, target_sr: int = 16_000):
    waveform, sr = torchaudio.load(path)
    if sr != target_sr:
        resampler = torchaudio.transforms.Resample(sr, target_sr)
        waveform = resampler(waveform)
    return waveform.squeeze(0).numpy()


def finetune(
    user_id: str,
    samples: list[dict],  # [{"audio_path": str, "transcript": str}, ...]
    progress_cb: Generator | None = None,
) -> Path:
    """
    Fine-tune Whisper on `samples` for `user_id`.
    Returns path to saved model directory.
    Calls progress_cb(percent, message) at key stages.
    """

    def _progress(pct: int, msg: str):
        if progress_cb is not None:
            progress_cb(pct, msg)

    _progress(5, "Loading base Whisper model…")
    processor = WhisperProcessor.from_pretrained(BASE_MODEL)
    model = WhisperForConditionalGeneration.from_pretrained(BASE_MODEL)

    lora_cfg = LoraConfig(
        task_type=TaskType.SEQ_2_SEQ_LM,
        r=8,
        lora_alpha=32,
        target_modules=["q_proj", "v_proj"],
        lora_dropout=0.05,
    )
    model = get_peft_model(model, lora_cfg)
    model.print_trainable_parameters()

    _progress(15, "Preparing dataset…")

    def _make_row(s: dict):
        audio = _load_audio_as_array(s["audio_path"])
        inputs = processor(audio, sampling_rate=16_000, return_tensors="pt")
        labels = processor.tokenizer(s["transcript"], return_tensors="pt").input_ids
        return {
            "input_features": inputs.input_features.squeeze(0),
            "labels": labels.squeeze(0),
        }

    rows = [_make_row(s) for s in samples]
    dataset = Dataset.from_list(rows)

    device = "cuda" if torch.cuda.is_available() else "cpu"
    model = model.to(device)
    model.train()

    optimizer = torch.optim.AdamW(model.parameters(), lr=1e-4)

    epochs = 3
    for epoch in range(epochs):
        pct = 20 + int((epoch / epochs) * 60)
        _progress(pct, f"Training epoch {epoch + 1}/{epochs}…")

        for row in dataset:
            input_features = torch.tensor(row["input_features"]).unsqueeze(0).to(device)
            labels = torch.tensor(row["labels"]).unsqueeze(0).to(device)

            optimizer.zero_grad()
            out = model(input_features=input_features, labels=labels)
            out.loss.backward()
            optimizer.step()

    _progress(85, "Merging LoRA weights…")
    merged = model.merge_and_unload()

    out_dir = CONFIG_DIR / "voice_models" / user_id
    out_dir.mkdir(parents=True, exist_ok=True)

    _progress(90, "Saving model…")
    merged.save_pretrained(str(out_dir))
    processor.save_pretrained(str(out_dir))

    # Write metadata so brain-node knows which model to load.
    meta = {"user_id": user_id, "base_model": BASE_MODEL, "sample_count": len(samples)}
    (out_dir / "meta.json").write_text(json.dumps(meta, indent=2))

    _progress(100, f"Fine-tuning complete — model saved to {out_dir}")
    return out_dir
