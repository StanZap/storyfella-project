from models.schemas import (
    CaptionRequest,
    CaptionResponse,
    GenerateRequest,
    GenerateResponse,
    SegmentRequest,
    SegmentResponse,
)


class VisionService:
    """Placeholder implementation behind stable HTTP contracts."""

    def segment(self, request: SegmentRequest) -> SegmentResponse:
        del request
        # TODO: Load PyTorch and SAM 3.1 here through a dedicated model adapter.
        return SegmentResponse(status="not_implemented", masks=[])

    def generate(self, request: GenerateRequest) -> GenerateResponse:
        del request
        # TODO: Add an ImageGenerator interface and platform-specific backend.
        return GenerateResponse(status="not_implemented", image_path=None)

    def caption(self, request: CaptionRequest) -> CaptionResponse:
        del request
        # TODO: Add a captioning model without leaking Python model details into Rust.
        return CaptionResponse(status="not_implemented", caption="Placeholder caption")
