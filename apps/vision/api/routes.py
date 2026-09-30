from fastapi import APIRouter

from models.schemas import (
    CaptionRequest,
    CaptionResponse,
    DeviceCapabilitiesResponse,
    GenerateRequest,
    GenerateResponse,
    GenerationJobResponse,
    GenerationCapabilitiesResponse,
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


@router.post("/generation/jobs", response_model=GenerationJobResponse, status_code=202)
def submit_generation_job(
    request: GenerateRequest, priority: str = "interactive"
) -> GenerationJobResponse:
    normalized_priority = priority if priority in {"interactive", "background"} else "interactive"
    return service.submit_generation(request, normalized_priority)


@router.get(
    "/generation/capabilities", response_model=GenerationCapabilitiesResponse
)
def generation_capabilities() -> GenerationCapabilitiesResponse:
    return service.generation_capabilities()


@router.get("/generation/jobs/{job_id}", response_model=GenerationJobResponse)
def generation_job(job_id: str) -> GenerationJobResponse:
    return service.generation_job(job_id)


@router.post("/generation/jobs/{job_id}/cancel", response_model=GenerationJobResponse)
def cancel_generation_job(job_id: str) -> GenerationJobResponse:
    return service.cancel_generation(job_id)


@router.post("/caption", response_model=CaptionResponse)
def caption(request: CaptionRequest) -> CaptionResponse:
    return service.caption(request)
