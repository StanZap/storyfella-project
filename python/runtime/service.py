import time
from pathlib import Path

from models.schemas import (
    CaptionRequest,
    CaptionResponse,
    DeviceCapabilitiesResponse,
    GenerateRequest,
    GenerateResponse,
    SegmentRequest,
    SegmentBox,
    SegmentDetection,
    SegmentMask,
    SegmentResponse,
)
from runtime.device import capabilities
from runtime.grounder import GroundingDinoGrounder, GroundingError, TextGrounder
from runtime.image_generator import (
    DEFAULT_MODEL,
    DiffusersImageGenerator,
    GenerationOptions,
    ImageGenerationError,
    ImageGenerator,
)
from runtime.segmenter import (
    DEFAULT_SEGMENTATION_MODEL,
    BoxPrompt,
    PointPrompt,
    Sam2Segmenter,
    SegmentationError,
    SegmentationOptions,
    Segmenter,
)


class VisionService:
    """Vision operations behind stable, framework-neutral HTTP contracts."""

    def __init__(
        self,
        image_generator: ImageGenerator | None = None,
        segmenter: Segmenter | None = None,
        grounder: TextGrounder | None = None,
    ) -> None:
        self._image_generator = (
            image_generator if image_generator is not None else DiffusersImageGenerator()
        )
        self._segmenter = segmenter if segmenter is not None else Sam2Segmenter()
        self._grounder = grounder if grounder is not None else GroundingDinoGrounder()

    def capabilities(self) -> DeviceCapabilitiesResponse:
        detected = capabilities()
        return DeviceCapabilitiesResponse(
            torch_available=detected.torch_available,
            torch_version=detected.torch_version,
            cuda_available=detected.cuda_available,
            cuda_devices=list(detected.cuda_devices),
            mps_available=detected.mps_available,
            recommended_device=detected.recommended_device,
        )

    def segment(self, request: SegmentRequest) -> SegmentResponse:
        started = time.monotonic()
        model = request.model or DEFAULT_SEGMENTATION_MODEL
        try:
            boxes = tuple(
                BoxPrompt(box.x_min, box.y_min, box.x_max, box.y_max)
                for box in request.boxes
            )
            grounding = None
            if request.prompt and not request.points and not boxes:
                grounding = self._grounder.ground(
                    Path(request.image_path), request.prompt, request.device
                )
                boxes = grounding.boxes
            result = self._segmenter.segment(
                SegmentationOptions(
                    image_path=Path(request.image_path),
                    points=tuple(
                        PointPrompt(point.x, point.y, point.label) for point in request.points
                    ),
                    boxes=boxes,
                    model=model,
                    device=request.device,
                )
            )
        except (GroundingError, SegmentationError, RuntimeError, ValueError) as error:
            return SegmentResponse(status="failed", model=model, error=str(error))

        return SegmentResponse(
            status="completed",
            masks=[
                SegmentMask(
                    path=str(mask.path),
                    score=mask.score,
                    area_pixels=mask.area_pixels,
                    bounding_box=SegmentBox(
                        x_min=mask.bounding_box.x_min,
                        y_min=mask.bounding_box.y_min,
                        x_max=mask.bounding_box.x_max,
                        y_max=mask.bounding_box.y_max,
                    ),
                )
                for mask in result.masks
            ],
            detections=(
                [
                    SegmentDetection(
                        label=request.prompt or "object",
                        score=score,
                        bounding_box=SegmentBox(
                            x_min=box.x_min,
                            y_min=box.y_min,
                            x_max=box.x_max,
                            y_max=box.y_max,
                        ),
                    )
                    for box, score in zip(grounding.boxes, grounding.scores, strict=True)
                ]
                if grounding is not None
                else []
            ),
            model=result.model,
            device=result.device,
            dtype=result.dtype,
            duration_ms=round((time.monotonic() - started) * 1000),
        )

    def generate(self, request: GenerateRequest) -> GenerateResponse:
        model = request.model or DEFAULT_MODEL
        try:
            result = self._image_generator.generate(
                GenerationOptions(
                    prompt=request.prompt,
                    width=request.width,
                    height=request.height,
                    steps=request.steps,
                    seed=request.seed,
                    model=model,
                    device=request.device,
                )
            )
        except (ImageGenerationError, RuntimeError, ValueError) as error:
            return GenerateResponse(status="failed", model=model, error=str(error))

        return GenerateResponse(
            status="completed",
            image_path=str(result.image_path),
            model=result.model,
            device=result.device,
            dtype=result.dtype,
            seed=result.seed,
            width=result.width,
            height=result.height,
            duration_ms=result.duration_ms,
        )

    def caption(self, request: CaptionRequest) -> CaptionResponse:
        del request
        # TODO: Add a captioning model without leaking Python model details into Rust.
        return CaptionResponse(status="not_implemented", caption="Placeholder caption")
