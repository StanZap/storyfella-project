import threading
import time
from pathlib import Path

from models.schemas import (
    CaptionRequest,
    CaptionResponse,
    DeviceCapabilitiesResponse,
    GenerateRequest,
    GenerateResponse,
    GenerationJobResponse,
    GenerationCapabilitiesResponse,
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
    GenerationOptions,
    ImageGenerationError,
    ImageGenerator,
    RoutedImageGenerator,
)
from runtime.sd_cpp import LoraSelection, StableDiffusionCppClient, StableDiffusionCppError
from runtime.segmenter import (
    DEFAULT_GEOMETRY_MODEL,
    DEFAULT_SEGMENTATION_MODEL,
    BoxPrompt,
    PointPrompt,
    Sam2Segmenter,
    Sam3Segmenter,
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
        sam3_segmenter: Segmenter | None = None,
        grounder: TextGrounder | None = None,
        native_client: StableDiffusionCppClient | None = None,
    ) -> None:
        self._image_generator = (
            image_generator if image_generator is not None else RoutedImageGenerator()
        )
        # Text prompts route to SAM 3 (native text prompting); geometry
        # (points/boxes) stays on SAM 2. When SAM 3's gated weights are
        # unavailable, prompt segmentation degrades to DINO grounding +
        # SAM 2 — a soft dependency, like the LLM steps.
        self._sam3_segmenter = sam3_segmenter if sam3_segmenter is not None else Sam3Segmenter()
        self._segmenter = segmenter if segmenter is not None else Sam2Segmenter()
        self._grounder = grounder if grounder is not None else GroundingDinoGrounder()
        self._native_client = native_client or StableDiffusionCppClient()
        self._generation_jobs: dict[str, tuple[str, str]] = {}
        self._generation_jobs_lock = threading.Lock()

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
        text_prompt = bool(request.prompt) and not request.points and not request.boxes
        model = request.model or (
            DEFAULT_SEGMENTATION_MODEL if text_prompt else DEFAULT_GEOMETRY_MODEL
        )
        try:
            boxes = tuple(
                BoxPrompt(box.x_min, box.y_min, box.x_max, box.y_max)
                for box in request.boxes
            )
            if text_prompt:
                # SAM 3's native text prompting: the modify prompt selects
                # the region directly — no grounding step. Degrades to DINO
                # boxes + SAM 2 when the SAM 3 weights are unavailable.
                try:
                    result = self._sam3_segmenter.segment(
                        SegmentationOptions(
                            image_path=Path(request.image_path),
                            points=(),
                            boxes=(),
                            text=(request.prompt or "",),
                            model=model,
                            device=request.device,
                        )
                    )
                except SegmentationError as sam3_error:
                    degraded_model = DEFAULT_GEOMETRY_MODEL
                    try:
                        boxes = self._grounder.ground(
                            Path(request.image_path), request.prompt or "", request.device
                        ).boxes
                        result = self._segmenter.segment(
                            SegmentationOptions(
                                image_path=Path(request.image_path),
                                points=(),
                                boxes=boxes,
                                model=degraded_model,
                                device=request.device,
                            )
                        )
                        model = degraded_model
                        print(
                            f"warning: SAM 3 unavailable ({sam3_error}); "
                            "segmented with DINO + SAM 2.1 instead",
                            flush=True,
                        )
                    except (GroundingError, SegmentationError) as degrade_error:
                        raise SegmentationError(
                            f"SAM 3 unavailable ({sam3_error}) and the fallback also failed "
                            f"({degrade_error})"
                        ) from degrade_error
            else:
                result = self._segmenter.segment(
                    SegmentationOptions(
                        image_path=Path(request.image_path),
                        points=tuple(
                            PointPrompt(point.x, point.y, point.label)
                            for point in request.points
                        ),
                        boxes=boxes,
                        model=model,
                        device=request.device,
                    )
                )
        except (SegmentationError, RuntimeError, ValueError) as error:
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
                        score=mask.score,
                        bounding_box=SegmentBox(
                            x_min=mask.bounding_box.x_min,
                            y_min=mask.bounding_box.y_min,
                            x_max=mask.bounding_box.x_max,
                            y_max=mask.bounding_box.y_max,
                        ),
                    )
                    for mask in result.masks
                ]
                if text_prompt
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
                    loras=tuple(
                        LoraSelection(path=item.path, multiplier=item.multiplier)
                        for item in request.loras
                    ),
                    reference_image_path=(
                        Path(request.reference_image_path)
                        if request.reference_image_path
                        else None
                    ),
                    reference_strength=request.reference_strength,
                    mask_path=(
                        Path(request.mask_path) if request.mask_path else None
                    ),
                    init_image_path=(
                        Path(request.init_image_path)
                        if request.init_image_path
                        else None
                    ),
                    denoise_strength=request.denoise_strength,
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

    def submit_generation(
        self, request: GenerateRequest, priority: str = "interactive"
    ) -> GenerationJobResponse:
        model = request.model or DEFAULT_MODEL
        if model not in {"krea-2-turbo-q2", "krea-2-turbo-q4"}:
            return GenerationJobResponse(
                id="not_submitted",
                status="failed",
                model=model,
                priority=priority,
                error="background jobs require a native Krea 2 profile",
            )
        try:
            self._native_client.assert_profile(model)
            job = self._native_client.submit(
                prompt=request.prompt,
                width=request.width,
                height=request.height,
                steps=request.steps,
                seed=request.seed,
                loras=tuple(
                    LoraSelection(path=item.path, multiplier=item.multiplier)
                    for item in request.loras
                ),
                reference_images=(Path(request.reference_image_path),)
                if request.reference_image_path
                else (),
                mask_image=Path(request.mask_path) if request.mask_path else None,
                init_image=(
                    Path(request.init_image_path) if request.init_image_path else None
                ),
                reference_strength=request.reference_strength,
                denoise_strength=request.denoise_strength,
            )
        except StableDiffusionCppError as error:
            return GenerationJobResponse(
                id="not_submitted",
                status="failed",
                model=model,
                priority=priority,
                error=str(error),
            )
        with self._generation_jobs_lock:
            self._generation_jobs[job.id] = (model, priority)
        return GenerationJobResponse(
            id=job.id,
            status=job.status,
            model=model,
            priority=priority,
        )

    def generation_capabilities(self) -> GenerationCapabilitiesResponse:
        try:
            capabilities = self._native_client.capabilities()
        except StableDiffusionCppError as error:
            return GenerationCapabilitiesResponse(
                status="unavailable", error=str(error)
            )
        model = str(capabilities.get("model", {}).get("name", "")) or None
        return GenerationCapabilitiesResponse(status="ready", model=model)

    def generation_job(self, job_id: str) -> GenerationJobResponse:
        model, priority = self._job_metadata(job_id)
        try:
            job = self._native_client.job(job_id)
        except StableDiffusionCppError as error:
            return GenerationJobResponse(
                id=job_id,
                status="failed",
                model=model,
                priority=priority,
                error=str(error),
            )
        return GenerationJobResponse(
            id=job.id,
            status=job.status,
            queue_position=job.queue_position,
            image_path=str(job.image_path) if job.image_path else None,
            model=model,
            priority=priority,
            error=job.error,
        )

    def cancel_generation(self, job_id: str) -> GenerationJobResponse:
        model, priority = self._job_metadata(job_id)
        try:
            job = self._native_client.cancel(job_id)
        except StableDiffusionCppError as error:
            try:
                current = self._native_client.job(job_id, save_result=False)
            except StableDiffusionCppError:
                current = None
            if current is not None and current.status in {"queued", "generating"}:
                return GenerationJobResponse(
                    id=job_id,
                    status=current.status,
                    queue_position=current.queue_position,
                    model=model,
                    priority=priority,
                    error=str(error),
                )
            return GenerationJobResponse(
                id=job_id,
                status="failed",
                model=model,
                priority=priority,
                error=str(error),
            )
        return GenerationJobResponse(
            id=job.id,
            status=job.status,
            queue_position=job.queue_position,
            model=model,
            priority=priority,
            error=job.error,
        )

    def _job_metadata(self, job_id: str) -> tuple[str, str]:
        with self._generation_jobs_lock:
            return self._generation_jobs.get(job_id, (DEFAULT_MODEL, "interactive"))

    def caption(self, request: CaptionRequest) -> CaptionResponse:
        del request
        # TODO: Add a captioning model without leaking Python model details into Rust.
        return CaptionResponse(status="not_implemented", caption="Placeholder caption")
