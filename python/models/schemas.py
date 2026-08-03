from pydantic import BaseModel, Field


class HealthResponse(BaseModel):
    status: str


class SegmentRequest(BaseModel):
    image_path: str
    prompt: str


class SegmentResponse(BaseModel):
    status: str
    masks: list[str] = Field(default_factory=list)


class GenerateRequest(BaseModel):
    prompt: str


class GenerateResponse(BaseModel):
    status: str
    image_path: str | None = None


class CaptionRequest(BaseModel):
    image_path: str


class CaptionResponse(BaseModel):
    status: str
    caption: str
