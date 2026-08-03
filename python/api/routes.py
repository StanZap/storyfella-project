from fastapi import APIRouter

from models.schemas import (
    CaptionRequest,
    CaptionResponse,
    DeviceCapabilitiesResponse,
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


@router.get("/capabilities", response_model=DeviceCapabilitiesResponse)
def runtime_capabilities() -> DeviceCapabilitiesResponse:
    return service.capabilities()


@router.post("/segment", response_model=SegmentResponse)
def segment(request: SegmentRequest) -> SegmentResponse:
    return service.segment(request)


@router.post("/generate", response_model=GenerateResponse)
def generate(request: GenerateRequest) -> GenerateResponse:
    return service.generate(request)


@router.post("/caption", response_model=CaptionResponse)
def caption(request: CaptionRequest) -> CaptionResponse:
    return service.caption(request)
