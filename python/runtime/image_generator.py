"""Lazy image-generation adapter used by the proof of concept."""

import os
import threading
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol

from runtime.device import RuntimeDependencyError, select_device

DEFAULT_MODEL = "Efficient-Large-Model/Sana_Sprint_0.6B_1024px_diffusers"


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


# TODO: Add a production model registry supplied by Rust instead of allowing the
# Python runtime to resolve model identifiers through Hugging Face.
