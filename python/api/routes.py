from fastapi import APIRouter

from models.schemas import (
    CaptionRequest,
    CaptionResponse,
    GenerateRequest,
    GenerateResponse,
    HealthResponse,
    SegmentRequest,
    SegmentResponse,
)
from runtime.service import VisionService

router = APIRouter()
service = VisionService()


@router.get("/health", response_model=HealthResponse)
def health() -> HealthResponse:
    return HealthResponse(status="ok")


@router.post("/segment", response_model=SegmentResponse)
def segment(request: SegmentRequest) -> SegmentResponse:
    return service.segment(request)


@router.post("/generate", response_model=GenerateResponse)
def generate(request: GenerateRequest) -> GenerateResponse:
    return service.generate(request)


@router.post("/caption", response_model=CaptionResponse)
def caption(request: CaptionRequest) -> CaptionResponse:
    return service.caption(request)
