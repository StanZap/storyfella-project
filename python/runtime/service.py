from models.schemas import (
    CaptionRequest,
    CaptionResponse,
    DeviceCapabilitiesResponse,
    GenerateRequest,
    GenerateResponse,
    SegmentRequest,
    SegmentResponse,
)
from runtime.device import capabilities
from runtime.image_generator import (
    DEFAULT_MODEL,
    DiffusersImageGenerator,
    GenerationOptions,
    ImageGenerationError,
    ImageGenerator,
)


class VisionService:
    """Vision operations behind stable, framework-neutral HTTP contracts."""

    def __init__(self, image_generator: ImageGenerator | None = None) -> None:
        self._image_generator = (
            image_generator if image_generator is not None else DiffusersImageGenerator()
        )

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
        del request
        # TODO: Load PyTorch and SAM 3.1 here through a dedicated model adapter.
        return SegmentResponse(status="not_implemented", masks=[])

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
