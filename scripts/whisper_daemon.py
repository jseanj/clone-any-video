#!/usr/bin/env python3
"""
Local Whisper Daemon for clone-any-video.
Implements the clone-any-video.whisperx-service@1 HTTP loopback protocol:
  GET  /health     -> {"ok": true, "protocol": "clone-any-video.whisperx-service@1", ...}
  POST /transcribe -> {"language": "...", "segments": [{"text": "...", "start": ..., "end": ..., "words": [...]}]}
"""

from __future__ import annotations

import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import importlib.metadata
import threading
import logging
import math
import os
import sys
import time

# Auto use high-speed HuggingFace mirror in China if not overridden
if "HF_ENDPOINT" not in os.environ:
    os.environ["HF_ENDPOINT"] = "https://hf-mirror.com"

logging.basicConfig(
    level=logging.INFO,
    format="[%(asctime)s] %(levelname)s %(name)s: %(message)s",
    datefmt="%H:%M:%S",
)
LOGGER = logging.getLogger("whisper_daemon")

PROTOCOL = "clone-any-video.whisperx-service@1"
SERVICE_VERSION = "1.0.0"


def finite_float(value: object) -> float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    val = float(value)
    return val if math.isfinite(val) else None


def engine_version():
    try:
        return importlib.metadata.version("faster-whisper")
    except importlib.metadata.PackageNotFoundError:
        return "unavailable"


class DaemonState:
    def __init__(self, model_name: str, device: str, compute: str):
        self.model_name = model_name
        self.device = device
        self.compute = compute
        self.model = None
        self.inference_lock = threading.Lock()
        self._load_model()

    def _load_model(self):
        LOGGER.info(f"Loading faster-whisper model '{self.model_name}' on {self.device} ({self.compute})...")
        started = time.monotonic()
        try:
            from faster_whisper import WhisperModel
            self.model = WhisperModel(
                self.model_name,
                device=self.device,
                compute_type=self.compute,
                download_root=os.path.expanduser("~/.cache/whisper_models"),
            )
            LOGGER.info(f"Model ready in {time.monotonic() - started:.2f}s")
        except Exception as e:
            LOGGER.error(f"Could not load faster_whisper ({e}); transcription remains unavailable.")
            self.model = None

    def transcribe_file(self, audio_path: str, language: str | None) -> dict:
        if not os.path.exists(audio_path):
            raise FileNotFoundError(f"Audio file not found: {audio_path}")

        if self.model is None:
            raise RuntimeError("Whisper model is not loaded; no simulated transcription is permitted")

        LOGGER.info(f"Transcribing {audio_path} (language={language})...")
        started = time.monotonic()
        lang_arg = None if language in (None, "", "auto") else language
        segments_gen, info = self.model.transcribe(
            audio_path,
            language=lang_arg,
            word_timestamps=True,
            vad_filter=True,
        )

        segments = []
        for s in segments_gen:
            words = []
            if s.words:
                for w in s.words:
                    word_dict = {
                        "word": w.word.strip(),
                    }
                    if w.start is not None and w.end is not None:
                        word_dict["start"] = round(w.start, 3)
                        word_dict["end"] = round(w.end, 3)
                    if w.probability is not None:
                        word_dict["score"] = round(w.probability, 3)
                    words.append(word_dict)
            segments.append({
                "text": s.text.strip(),
                "start": round(s.start, 3),
                "end": round(s.end, 3),
                "words": words,
            })

        LOGGER.info(f"Transcription finished in {time.monotonic() - started:.2f}s ({len(segments)} segments)")
        return {
            "language": info.language,
            "duration": round(info.duration, 3),
            "segments": segments,
        }


class WhisperHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "whisperx/1.0.0"

    def _send_json(self, status_code: int, data: dict):
        body = json.dumps(data, ensure_ascii=False).encode("utf-8")
        self.send_response(status_code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/health":
            state: DaemonState = self.server.daemon_state
            is_loaded = state.model is not None
            self._send_json(200 if is_loaded else 503, {
                "ok": is_loaded,
                "model_loaded": is_loaded,
                "protocol": PROTOCOL,
                "serviceVersion": SERVICE_VERSION,
                "model": state.model_name,
                "device": state.device,
                "compute": state.compute,
                "batchSize": 1,
                "engine": "faster-whisper",
                "engineVersion": engine_version(),
                "alignment": "asr-word-timestamps",
                "forced_alignment": False,
            })
        else:
            self._send_json(404, {"error": {"code": "NOT_FOUND", "message": "not found"}})

    def do_POST(self):
        if self.path != "/transcribe":
            self._send_json(404, {"error": {"code": "NOT_FOUND", "message": "not found"}})
            return

        try:
            content_len = int(self.headers.get("Content-Length", 0))
            if content_len <= 0:
                self._send_json(400, {"error": {"code": "EMPTY_BODY", "message": "Empty request body"}})
                return

            body = self.rfile.read(content_len)
            payload = json.loads(body.decode("utf-8"))
            audio_path = payload.get("audio_path")
            language = payload.get("language")

            if not audio_path:
                self._send_json(400, {"error": {"code": "INVALID_ARG", "message": "audio_path is required"}})
                return

            state: DaemonState = self.server.daemon_state
            if state.model is None:
                self._send_json(503, {"error":{"code":"MODEL_UNAVAILABLE","message":"Load a real Whisper model first"}})
                return
            with state.inference_lock:
                result = state.transcribe_file(audio_path, language)
            result["engine"] = {"name":"faster-whisper","version":engine_version(),"model":state.model_name,"compute":state.compute,"forced_alignment":False}
            self._send_json(200, result)

        except Exception as e:
            LOGGER.exception("Inference error")
            self._send_json(500, {"error": {"code": "INFERENCE_FAILED", "message": str(e)}})

    def log_message(self, format, *args):
        LOGGER.info("%s - %s", self.client_address[0], format % args)


class WhisperServer(ThreadingHTTPServer):
    allow_reuse_address = True
    daemon_threads = True

def run_daemon(host: str = "127.0.0.1", port: int = 8765, model: str = "medium", device: str = "cpu", compute: str = "int8"):
    state = DaemonState(model_name=model, device=device, compute=compute)
    server = WhisperServer((host, port), WhisperHandler)
    server.daemon_state = state
    LOGGER.info(f"Whisper Loopback Daemon listening on http://{host}:{port}")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        LOGGER.info("Shutdown requested")
    finally:
        server.server_close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Local Whisper Daemon")
    parser.add_argument("--host", default="127.0.0.1", help="Bind host (default: 127.0.0.1)")
    parser.add_argument("--port", type=int, default=8765, help="Port (default: 8765)")
    parser.add_argument("--model", default="medium", help="Whisper model name (tiny, base, small, medium)")
    parser.add_argument("--device", default="cpu", help="Device (cpu, mps, cuda)")
    parser.add_argument("--compute", default="int8", help="Compute type (float32, int8, int8_float16)")

    args = parser.parse_args()
    run_daemon(host=args.host, port=args.port, model=args.model, device=args.device, compute=args.compute)
