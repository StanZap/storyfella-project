"""Client for a persistent stable-diffusion.cpp generation server."""

import base64
import json
import os
import threading
import time
import urllib.error
import urllib.request
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from runtime.model_manifest import profile


class StableDiffusionCppError(RuntimeError):
    """A stable failure type for the native generation boundary."""


@dataclass(frozen=True)
class LoraSelection:
    path: str
    multiplier: float = 1.0


@dataclass(frozen=True)
class NativeJob:
    id: str
    status: str
    queue_position: int = 0
    image_path: Path | None = None
    error: str | None = None


class StableDiffusionCppClient:
    """Uses sd.cpp's async API so its loaded model remains resident between jobs."""

    def __init__(
        self,
        base_url: str | None = None,
        output_directory: Path | None = None,
        timeout_seconds: float = 30.0,
    ) -> None:
        project_root = Path(__file__).resolve().parents[2]
        self.base_url = (base_url or os.environ.get("SVS_SD_CPP_URL", "http://127.0.0.1:7861")).rstrip("/")
        self.output_directory = output_directory or Path(
            os.environ.get("SVS_GENERATED_DIR", project_root / "cache" / "generated")
        )
        self.timeout_seconds = timeout_seconds
        self._completed_jobs: dict[str, Path] = {}
        self._completed_jobs_lock = threading.Lock()

    def capabilities(self) -> dict[str, Any]:
        return self._request("GET", "/sdcpp/v1/capabilities")

    def submit(
        self,
        *,
        prompt: str,
        width: int,
        height: int,
        steps: int,
        seed: int,
        loras: tuple[LoraSelection, ...] = (),
        reference_images: tuple[Path, ...] = (),
    ) -> NativeJob:
        body: dict[str, Any] = {
            "prompt": prompt,
            "width": width,
            "height": height,
            "seed": seed,
            "batch_count": 1,
            "output_format": "png",
            "sample_params": {
                "sample_steps": steps,
                "sample_method": "euler",
                "guidance": {"txt_cfg": 1.0},
            },
        }
        if loras:
            body["lora"] = [
                {"path": item.path, "multiplier": item.multiplier} for item in loras
            ]
        if reference_images:
            encoded_references: list[str] = []
            for path in reference_images:
                try:
                    encoded_references.append(base64.b64encode(path.read_bytes()).decode())
                except OSError as cause:
                    raise StableDiffusionCppError(
                        f"could not read reference image {path}: {cause}"
                    ) from cause
            body["ref_images"] = encoded_references
        result = self._request("POST", "/sdcpp/v1/img_gen", body)
        return NativeJob(id=str(result["id"]), status=str(result["status"]))

    def job(self, job_id: str, *, save_result: bool = True) -> NativeJob:
        if save_result:
            with self._completed_jobs_lock:
                cached_path = self._completed_jobs.get(job_id)
            if cached_path is not None:
                return NativeJob(id=job_id, status="completed", image_path=cached_path)
        result = self._request("GET", f"/sdcpp/v1/jobs/{job_id}")
        status = str(result["status"])
        image_path = None
        error = None
        if status == "completed" and save_result:
            images = result.get("result", {}).get("images", [])
            if not images:
                raise StableDiffusionCppError("completed generation job contained no images")
            self.output_directory.mkdir(parents=True, exist_ok=True)
            image_path = self.output_directory / f"{uuid.uuid4()}.png"
            try:
                image_path.write_bytes(base64.b64decode(images[0]["b64_json"], validate=True))
            except (KeyError, ValueError) as cause:
                raise StableDiffusionCppError("generation job returned invalid base64 image data") from cause
            image_path = image_path.resolve()
            with self._completed_jobs_lock:
                self._completed_jobs[job_id] = image_path
        elif status in {"failed", "cancelled"}:
            error = str((result.get("error") or {}).get("message", status))
        return NativeJob(
            id=job_id,
            status=status,
            queue_position=int(result.get("queue_position", 0)),
            image_path=image_path,
            error=error,
        )

    def wait(self, job_id: str, timeout_seconds: float) -> NativeJob:
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline:
            job = self.job(job_id)
            if job.status in {"completed", "failed", "cancelled"}:
                return job
            time.sleep(0.1)
        raise StableDiffusionCppError(f"generation job {job_id} exceeded {timeout_seconds:.1f}s")

    def cancel(self, job_id: str) -> NativeJob:
        result = self._request("POST", f"/sdcpp/v1/jobs/{job_id}/cancel", {})
        return NativeJob(
            id=job_id,
            status=str(result["status"]),
            queue_position=int(result.get("queue_position", 0)),
            error=(result.get("error") or {}).get("message"),
        )

    def assert_profile(self, profile_id: str) -> None:
        expected = profile(profile_id).diffusion.filename
        capabilities = self.capabilities()
        actual = str(capabilities.get("model", {}).get("name", ""))
        if actual != expected:
            raise StableDiffusionCppError(
                f"resident backend has {actual or 'no model'} loaded; {profile_id} requires {expected}. "
                "Switching quantization profiles requires restarting only the native generation server."
            )

    def _request(self, method: str, path: str, body: dict[str, Any] | None = None) -> dict[str, Any]:
        encoded = json.dumps(body).encode() if body is not None else None
        request = urllib.request.Request(
            f"{self.base_url}{path}",
            data=encoded,
            method=method,
            headers={"Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(request, timeout=self.timeout_seconds) as response:
                payload = json.load(response)
        except urllib.error.HTTPError as error:
            detail = error.read().decode(errors="replace")
            raise StableDiffusionCppError(f"sd.cpp returned HTTP {error.code}: {detail}") from error
        except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as error:
            raise StableDiffusionCppError(f"sd.cpp is unavailable at {self.base_url}: {error}") from error
        if not isinstance(payload, dict):
            raise StableDiffusionCppError("sd.cpp returned a non-object response")
        return payload
