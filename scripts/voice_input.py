"""One-shot local dictation worker. Audio never leaves this computer."""
import json
from pathlib import Path
import select
import signal
import subprocess
import sys
import tempfile
import wave


def emit(**event):
    print(json.dumps(event), flush=True)


def transcribe(path):
    from faster_whisper import WhisperModel
    cache = Path.home() / ".cache/huggingface/hub/models--Systran--faster-whisper-base"
    snapshots = sorted((cache / "snapshots").glob("*"))
    model_path = next((p for p in snapshots if (p / "model.bin").is_file()), None)
    if model_path is None:
        raise RuntimeError("Local Whisper base model is missing. Run scripts/setup-voice.sh.")
    model = WhisperModel(str(model_path), device="cpu", compute_type="int8", cpu_threads=4, num_workers=1)
    segments, _ = model.transcribe(str(path), beam_size=1, vad_filter=True, condition_on_previous_text=False)
    return " ".join(s.text.strip() for s in segments).strip()


def record():
    # Private temporary directory; WAV is deleted after success, failure or quit.
    with tempfile.TemporaryDirectory(prefix="optic-chat-voice-") as directory:
        path = Path(directory) / "speech.wav"
        with open(Path(directory) / "record-error.txt", "w+") as errors:
            recorder = subprocess.Popen([
                "pw-record", "--rate", "16000", "--channels", "1", "--format", "s16",
                "--media-role", "Communication", str(path)
            ], stdout=subprocess.DEVNULL, stderr=errors)
            try:
                # Detect immediate microphone/permission failures before saying listening.
                import time
                time.sleep(0.15)
                if recorder.poll() is not None:
                    errors.seek(0)
                    raise RuntimeError("Microphone unavailable: " + errors.read()[:400])
                emit(phase="recording")
                # A second press stops recording; a two-minute cap avoids forgotten capture.
                import time
                deadline = time.monotonic() + 120
                while time.monotonic() < deadline:
                    if select.select([sys.stdin], [], [], 0.2)[0]:
                        if not sys.stdin.readline():
                            return
                        break
                    if recorder.poll() is not None:
                        raise RuntimeError("Microphone disconnected during recording.")
            finally:
                if recorder.poll() is None:
                    recorder.send_signal(signal.SIGINT)
                    try:
                        recorder.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        recorder.kill()
                        recorder.wait()
            # pw-record returns 1 on a normal SIGINT stop on some PipeWire
            # versions. Validate the finalized WAV instead of rejecting that.
            if recorder.returncode not in (0, 1, -signal.SIGINT):
                errors.seek(0)
                raise RuntimeError("Recording failed: " + errors.read()[:400])
        with wave.open(str(path)) as audio:
            if audio.getnframes() == 0:
                raise RuntimeError("No audio captured. Try recording for a little longer.")
        emit(phase="transcribing")
        emit(text=transcribe(path))


def main():
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
    try:
        if len(sys.argv) == 3 and sys.argv[1] == "--transcribe":
            emit(text=transcribe(Path(sys.argv[2])))
        else:
            record()
    except Exception as error:
        emit(error=str(error))
        sys.exit(1)


if __name__ == "__main__":
    main()
