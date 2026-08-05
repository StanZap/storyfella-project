"""Lazy image-generation adapter used by the proof of concept."""

import os
import threading
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol

from runtime.device import RuntimeDependencyError, select_device
from runtime.model_manifest import PROFILES
from runtime.sd_cpp import LoraSelection, StableDiffusionCppClient, StableDiffusionCppError

DEFAULT_MODEL = "krea-2-turbo-q2"
SANA_MODEL = "Efficient-Large-Model/Sana_Sprint_0.6B_1024px_diffusers"


class ImageGenerationError(RuntimeError):
    """A stable failure type for model-loading and inference errors."""


@dataclass(frozen=True)
class GenerationOptions:
    prompt: str
    width: int
    height: int
    steps: int
    seed: int
    model: str
    device: str
    loras: tuple[LoraSelection, ...] = ()
    reference_image_path: Path | None = None
    mask_path: Path | None = None


@dataclass(frozen=True)
class GenerationResult:
    image_path: Path
    model: str
    device: str
    dtype: str
    seed: int
    width: int
    height: int
    duration_ms: int


class ImageGenerator(Protocol):
    def generate(self, options: GenerationOptions) -> GenerationResult: ...


class DiffusersImageGenerator:
    """Loads one Diffusers pipeline on demand and serializes inference calls."""

    def __init__(self, output_directory: Path | None = None) -> None:
        project_root = Path(__file__).resolve().parents[2]
        configured_output = os.environ.get("SVS_GENERATED_DIR")
        self._output_directory = output_directory or (
            Path(configured_output) if configured_output else project_root / "cache" / "generated"
        )
        self._pipeline: Any | None = None
        self._loaded_key: tuple[str, str, str] | None = None
        self._lock = threading.Lock()

    def generate(self, options: GenerationOptions) -> GenerationResult:
        started = time.monotonic()
        try:
            with self._lock:
                pipeline, torch, device, dtype = self._load_pipeline(options)
                generator = torch.Generator(device="cpu").manual_seed(options.seed)
                output = pipeline(
                    prompt=options.prompt,
                    width=options.width,
                    height=options.height,
                    num_inference_steps=options.steps,
                    guidance_scale=0.0,
                    generator=generator,
                )
                if not output.images:
                    raise ImageGenerationError("the image pipeline returned no images")

                self._output_directory.mkdir(parents=True, exist_ok=True)
                destination = self._output_directory / f"{uuid.uuid4()}.png"
                output.images[0].save(destination, format="PNG")
        except (ImageGenerationError, RuntimeDependencyError):
            raise
        except Exception as error:
            raise ImageGenerationError(str(error)) from error

        return GenerationResult(
            image_path=destination.resolve(),
            model=options.model,
            device=device,
            dtype=dtype,
            seed=options.seed,
            width=options.width,
            height=options.height,
            duration_ms=round((time.monotonic() - started) * 1000),
        )

    def _load_pipeline(self, options: GenerationOptions) -> tuple[Any, Any, str, str]:
        try:
            import torch
            from diffusers import SanaSprintPipeline
        except ImportError as error:
            raise RuntimeDependencyError(
                "image generation dependencies are not installed; run "
                "`uv sync --project python --extra image-generation`"
            ) from error

        selection = select_device(options.device)
        key = (options.model, selection.name, selection.dtype)
        if self._pipeline is None or self._loaded_key != key:
            torch_dtype = getattr(torch, selection.dtype)
            pipeline = SanaSprintPipeline.from_pretrained(
                options.model,
                torch_dtype=torch_dtype,
            )
            pipeline.to(selection.name)
            if selection.name == "mps" and hasattr(pipeline, "enable_attention_slicing"):
                pipeline.enable_attention_slicing()
            pipeline.set_progress_bar_config(disable=True)
            self._pipeline = pipeline
            self._loaded_key = key

        return self._pipeline, torch, selection.name, selection.dtype


class StableDiffusionCppImageGenerator:
    """Delegates to one persistent native process; inference never unloads it."""

    def __init__(self, client: StableDiffusionCppClient | None = None) -> None:
        self.client = client or StableDiffusionCppClient()

    def generate(self, options: GenerationOptions) -> GenerationResult:
        if options.model not in PROFILES:
            raise ImageGenerationError(f"{options.model!r} is not a native Krea profile")
        started = time.monotonic()
        try:
            self.client.assert_profile(options.model)
            job = self.client.submit(
                prompt=options.prompt,
                width=options.width,
                height=options.height,
                steps=options.steps,
                seed=options.seed,
                loras=options.loras,
                reference_images=(options.reference_image_path,)
                if options.reference_image_path is not None
                else (),
                mask_images=(options.mask_path,)
                if options.mask_path is not None
                else (),
            )
            job = self.client.wait(job.id, timeout_seconds=600.0)
        except StableDiffusionCppError as error:
            raise ImageGenerationError(str(error)) from error
        if job.status != "completed" or job.image_path is None:
            raise ImageGenerationError(job.error or f"generation job ended as {job.status}")
        return GenerationResult(
            image_path=job.image_path,
            model=options.model,
            device="native",
            dtype=PROFILES[options.model].quantization,
            seed=options.seed,
            width=options.width,
            height=options.height,
            duration_ms=round((time.monotonic() - started) * 1000),
        )


class RoutedImageGenerator:
    """Keeps both backend choices explicit while Krea is the product default."""

    def __init__(self) -> None:
        self._native = StableDiffusionCppImageGenerator()
        self._diffusers = DiffusersImageGenerator()

    def generate(self, options: GenerationOptions) -> GenerationResult:
        if options.model in PROFILES:
            return self._native.generate(options)
        return self._diffusers.generate(options)


# TODO: Remove the legacy SANA/Hugging Face fallback once native Krea quality
# gates pass on both target platforms. Rust already provisions native models.
