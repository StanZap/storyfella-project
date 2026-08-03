"""Text-to-box grounding adapters."""

import threading
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol

from PIL import Image

from runtime.device import RuntimeDependencyError, select_device
from runtime.segmenter import BoxPrompt

DEFAULT_GROUNDING_MODEL = "IDEA-Research/grounding-dino-tiny"


class GroundingError(RuntimeError):
    """A stable failure type for text grounding."""


@dataclass(frozen=True)
class GroundingResult:
    boxes: tuple[BoxPrompt, ...]
    scores: tuple[float, ...]


class TextGrounder(Protocol):
    def ground(self, image_path: Path, prompt: str, device: str) -> GroundingResult: ...


class GroundingDinoGrounder:
    def __init__(self, model_id: str = DEFAULT_GROUNDING_MODEL) -> None:
        self._model_id = model_id
        self._model: Any | None = None
        self._processor: Any | None = None
        self._loaded_device: str | None = None
        self._lock = threading.Lock()

    def ground(self, image_path: Path, prompt: str, device: str) -> GroundingResult:
        if not image_path.is_file():
            raise GroundingError(f"image does not exist: {image_path}")
        try:
            with self._lock:
                model, processor, torch, resolved_device = self._load_model(device)
                with Image.open(image_path) as source:
                    image = source.convert("RGB")
                inputs = processor(
                    images=image,
                    text=[[prompt]],
                    return_tensors="pt",
                ).to(resolved_device)
                with torch.inference_mode():
                    outputs = model(**inputs)
                result = processor.post_process_grounded_object_detection(
                    outputs,
                    inputs.input_ids,
                    threshold=0.35,
                    text_threshold=0.25,
                    target_sizes=[image.size[::-1]],
                )[0]
        except (GroundingError, RuntimeDependencyError):
            raise
        except Exception as error:
            raise GroundingError(str(error)) from error

        boxes = tuple(BoxPrompt(*map(float, box.tolist())) for box in result["boxes"])
        scores = tuple(float(score.item()) for score in result["scores"])
        if not boxes:
            raise GroundingError(f"no object matched text prompt: {prompt}")
        return GroundingResult(boxes, scores)

    def _load_model(self, device: str) -> tuple[Any, Any, Any, str]:
        try:
            import torch
            from transformers import AutoModelForZeroShotObjectDetection, AutoProcessor
        except ImportError as error:
            raise RuntimeDependencyError(
                "segmentation dependencies are not installed; run "
                "`uv sync --project python --extra segmentation`"
            ) from error

        selection = select_device(device)
        if self._model is None or self._loaded_device != selection.name:
            self._processor = AutoProcessor.from_pretrained(self._model_id)
            self._model = AutoModelForZeroShotObjectDetection.from_pretrained(self._model_id)
            self._model.to(selection.name)
            self._model.eval()
            self._loaded_device = selection.name
        return self._model, self._processor, torch, selection.name
