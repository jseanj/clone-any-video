#!/usr/bin/env python3
"""Warm acoustic alignment only. ASR remains the separate medium service on 8765."""
from __future__ import annotations
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.metadata
import json
import logging
import math
from pathlib import Path
import threading

LOG = logging.getLogger("acoustic_alignment")
PROTOCOL = "clone-any-video.acoustic-alignment@1"


def clean(value):
    if isinstance(value, dict):
        return {k: clean(v) for k, v in value.items()}
    if isinstance(value, list):
        return [clean(v) for v in value]
    if isinstance(value, float) and not math.isfinite(value):
        return None
    return value


def finite(value):
    return isinstance(value, (float, int)) and not isinstance(value, bool) and math.isfinite(value)


class AlignmentState:
    def __init__(self, language="en", model="WAV2VEC2_ASR_BASE_960H", device="cpu", download=False):
        self.language, self.model_name, self.device = language, model, device
        self.model = self.metadata = self.engine = None
        self.error = None
        self.lock = threading.Lock()
        try:
            import whisperx
            import nltk
            # Prepare sentence data explicitly; never download it in an inference request.
            try:
                nltk.data.find("tokenizers/punkt_tab")
            except LookupError:
                if not download or not nltk.download("punkt_tab", quiet=True):
                    raise RuntimeError("Missing punkt_tab; run alignment_daemon.py --prepare first")
            self.engine = whisperx
            # WhisperX's cache-only flag applies to Hugging Face models, but not
            # torchaudio bundles. Guard that path before calling get_model.
            if not download:
                import torch
                import torchaudio
                if model in torchaudio.pipelines.__all__:
                    bundle = torchaudio.pipelines.__dict__[model]
                    checkpoint = Path(torch.hub.get_dir()) / "checkpoints" / Path(bundle._path).name
                    if not checkpoint.is_file():
                        raise RuntimeError("Missing acoustic weights; run alignment_daemon.py --prepare first")
            self.model, self.metadata = whisperx.load_align_model(
                language_code=language, device=device, model_name=model,
                model_cache_only=not download,
            )
        except Exception as error:
            self.error = str(error)
            LOG.exception("Alignment model is unavailable")

    def health(self):
        def version(name):
            try:
                return importlib.metadata.version(name)
            except importlib.metadata.PackageNotFoundError:
                return None
        return {"protocol": PROTOCOL, "ok": self.model is not None, "model_loaded": self.model is not None,
                "engine": "whisperx-alignment", "engineVersion": version("whisperx"),
                "torchVersion": version("torch"), "language": self.language, "model": self.model_name,
                "device": self.device, "capabilities": ["acoustic-forced-alignment"], "error": self.error}

    def align(self, payload):
        if payload.get("language") != self.language:
            raise ValueError("Requested language differs from the loaded alignment model")
        audio_path = payload.get("audio_path")
        if not isinstance(audio_path, str) or not Path(audio_path).is_file():
            raise ValueError("audio_path must name an existing local file")
        segments = payload.get("segments")
        if not isinstance(segments, list) or not segments:
            raise ValueError("Nonempty transcript segments are required")
        audio = self.engine.load_audio(audio_path)
        duration = len(audio) / 16000.0
        selected = []
        last_end = 0.0
        for segment in segments:
            text, start, end = segment.get("text"), segment.get("start"), segment.get("end")
            if not isinstance(text, str) or not text.strip() or not finite(start) or not finite(end):
                raise ValueError("Each segment needs text and finite start/end")
            if start < last_end - 0.001 or start < 0 or end <= start or end > duration + 0.02:
                raise ValueError("Transcript interval exceeds or overlaps its actual audio domain")
            end = min(end, duration)
            selected.append({"text": text, "start": start, "end": end})
            last_end = end
        # Do not fill missing acoustic observations with nearest/linear guessed timestamps.
        result = self.engine.align(selected, self.model, self.metadata, audio, self.device,
                                   interpolate_method="ignore", return_char_alignments=True)
        measured, missing, weak = 0, [], []
        for segment in result["segments"]:
            for word in segment.get("words", []):
                if not finite(word.get("start")) or not finite(word.get("end")):
                    missing.append(word.get("word", word.get("text", "")))
                else:
                    measured += 1
                    if finite(word.get("score")) and word["score"] < 0.5:
                        weak.append(word)
        return clean({"protocol": PROTOCOL, "language": self.language, "duration": duration,
                "segments": result["segments"], "forced_alignment": True, "interpolation": "none",
                "coverage": {"timed_words": measured, "missing_words": missing, "weak_words": weak},
                "engine": self.health()})


class Handler(BaseHTTPRequestHandler):
    def reply(self, code, value):
        data = json.dumps(value, allow_nan=False).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if self.path != "/health":
            self.reply(404, {"error": "not found"})
            return
        value = self.server.state.health()
        self.reply(200 if value["ok"] else 503, value)

    def do_POST(self):
        if self.path != "/align":
            self.reply(404, {"error": "not found"})
            return
        state = self.server.state
        if state.model is None:
            self.reply(503, {"error": "ALIGNMENT_MODEL_UNAVAILABLE", "message": state.error})
            return
        if not state.lock.acquire(blocking=False):
            self.reply(409, {"error": "ALIGNMENT_BUSY"})
            return
        try:
            length = int(self.headers.get("Content-Length", 0))
            if not 0 < length <= 4_000_000:
                raise ValueError("Invalid request size")
            payload = json.loads(self.rfile.read(length))
            self.reply(200, state.align(payload))
        except ValueError as error:
            self.reply(400, {"error": "INVALID_ALIGNMENT_INPUT", "message": str(error)})
        except Exception as error:
            LOG.exception("Acoustic alignment failed")
            self.reply(500, {"error": "ALIGNMENT_FAILED", "message": str(error)})
        finally:
            state.lock.release()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", choices=["127.0.0.1", "localhost"], default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8766)
    parser.add_argument("--language", default="en")
    parser.add_argument("--model", default="WAV2VEC2_ASR_BASE_960H")
    parser.add_argument("--device", default="cpu")
    parser.add_argument("--prepare", action="store_true", help="Explicitly download resources, verify loading, then exit")
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO)
    state = AlignmentState(args.language, args.model, args.device, args.prepare)
    if args.prepare:
        print(json.dumps(state.health()))
        raise SystemExit(0 if state.model is not None else 1)
    server = ThreadingHTTPServer((args.host, args.port), Handler)
    server.daemon_threads = True
    server.state = state
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
