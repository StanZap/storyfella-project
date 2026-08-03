"""Promptable segmentation adapters."""

import os
import threading
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol

from PIL import Image

from runtime.device import RuntimeDependencyError, select_device

DEFAULT_SEGMENTATION_MODEL = "facebook/sam2.1-hiera-tiny"


class SegmentationError(RuntimeError):
    """A stable failure type for model loading and segmentation."""


@dataclass(frozen=True)
class PointPrompt:
    x: float
    y: float
    label: int


@dataclass(frozen=True)
class BoxPrompt:
    x_min: float
    y_min: float
    x_max: float
    y_max: float


@dataclass(frozen=True)
class SegmentationOptions:
    image_path: Path
    points: tuple[PointPrompt, ...]
    boxes: tuple[BoxPrompt, ...]
    model: str
    device: str


@dataclass(frozen=True)
class MaskResult:
    path: Path
    score: float
    area_pixels: int
    bounding_box: BoxPrompt


@dataclass(frozen=True)
class SegmentationResult:
    masks: tuple[MaskResult, ...]
    model: str
    device: str
    dtype: str
    duration_ms: int


class Segmenter(Protocol):
    def segment(self, options: SegmentationOptions) -> SegmentationResult: ...


class Sam2Segmenter:
    """Lazy SAM 2.1 adapter for point- or box-prompted image masks."""

    def __init__(self, output_directory: Path | None = None) -> None:
        project_root = Path(__file__).resolve().parents[2]
        configured_output = os.environ.get("SVS_MASK_DIR")
        self._output_directory = output_directory or (
            Path(configured_output) if configured_output else project_root / "cache" / "masks"
        )
        self._model: Any | None = None
        self._processor: Any | None = None
        self._loaded_key: tuple[str, str, str] | None = None
        self._lock = threading.Lock()

    def segment(self, options: SegmentationOptions) -> SegmentationResult:
        if bool(options.points) == bool(options.boxes):
            raise SegmentationError("provide either points or boxes, but not both")
        if not options.image_path.is_file():
            raise SegmentationError(f"image does not exist: {options.image_path}")

        started = time.monotonic()
        try:
            with self._lock:
                model, processor, torch, device, dtype = self._load_model(options)
                with Image.open(options.image_path) as source:
                    image = source.convert("RGB")
                processor_args: dict[str, Any] = {"images": image, "return_tensors": "pt"}
                if options.boxes:
                    box_values = [
                        [box.x_min, box.y_min, box.x_max, box.y_max]
                        for box in options.boxes
                    ]
                    processor_args["input_boxes"] = [box_values]
                else:
                    point_values = [
                        [point.x, point.y] for point in options.points
                    ]
                    point_labels = [point.label for point in options.points]
                    processor_args["input_points"] = [[point_values]]
                    processor_args["input_labels"] = [[point_labels]]

                inputs = processor(**processor_args).to(device)
                with torch.inference_mode():
                    outputs = model(**inputs)
                masks = processor.post_process_masks(
                    outputs.pred_masks.cpu(), inputs["original_sizes"]
                )[0]
                scores = outputs.iou_scores[0].detach().float().cpu()
                results = self._save_best_masks(masks, scores)
        except (SegmentationError, RuntimeDependencyError):
            raise
        except Exception as error:
            raise SegmentationError(str(error)) from error

        return SegmentationResult(
            masks=tuple(results),
            model=options.model,
            device=device,
            dtype=dtype,
            duration_ms=round((time.monotonic() - started) * 1000),
        )

    def _load_model(self, options: SegmentationOptions) -> tuple[Any, Any, Any, str, str]:
        try:
            import torch
            from transformers import Sam2Model, Sam2Processor
        except ImportError as error:
            raise RuntimeDependencyError(
                "segmentation dependencies are not installed; run "
                "`uv sync --project python --extra segmentation`"
            ) from error

        selection = select_device(options.device)
        dtype = "bfloat16" if selection.name == "cuda" else "float32"
        key = (options.model, selection.name, dtype)
        if self._model is None or self._loaded_key != key:
            model = Sam2Model.from_pretrained(options.model, torch_dtype=getattr(torch, dtype))
            model.to(selection.name)
            model.eval()
            self._model = model
            self._processor = Sam2Processor.from_pretrained(options.model)
            self._loaded_key = key
        return self._model, self._processor, torch, selection.name, dtype

    def _save_best_masks(self, masks: Any, scores: Any) -> list[MaskResult]:
        self._output_directory.mkdir(parents=True, exist_ok=True)
        results = []
        for object_index in range(masks.shape[0]):
            candidate = int(scores[object_index].argmax().item())
            mask = masks[object_index, candidate].bool()
            coordinates = mask.nonzero(as_tuple=False)
            if coordinates.numel() == 0:
                continue
            y_min, x_min = coordinates.min(dim=0).values.tolist()
            y_max, x_max = coordinates.max(dim=0).values.tolist()
            path = self._output_directory / f"{uuid.uuid4()}.png"
            pixels = mask.byte().mul(255).numpy()
            Image.fromarray(pixels).save(path, format="PNG")
            results.append(
                MaskResult(
                    path=path.resolve(),
                    score=float(scores[object_index, candidate].item()),
                    area_pixels=int(mask.sum().item()),
                    bounding_box=BoxPrompt(float(x_min), float(y_min), float(x_max), float(y_max)),
                )
            )
        return results


# TODO: Add a SAM 3.1 implementation for authenticated CUDA installations and
# a separate text-grounding adapter; SAM 2.1 intentionally accepts geometry only.
