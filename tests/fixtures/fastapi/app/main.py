"""Application entry point."""

from fastapi import FastAPI

from .api import admin, orphan  # noqa: F401  (orphan is deliberately unmounted)
from .api.items import router as items_router
from .api.users import router as users_router
from .config import settings

API_V1 = "/api/v1"

app = FastAPI(title="RouteLens fixture")

app.include_router(users_router, prefix=API_V1)
app.include_router(items_router, prefix="/v1")
app.include_router(items_router, prefix="/v2")
app.include_router(admin.router, prefix=settings.API_PREFIX)


@app.get("/health", tags=["meta"])
async def health():
    """Liveness probe."""
    return {"ok": True}
