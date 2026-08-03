from fastapi import FastAPI

from api.routes import router


def create_app() -> FastAPI:
    application = FastAPI(title="Smart Visual Sequencer Vision Runtime", version="0.1.0")
    application.include_router(router)
    return application
