"""Mounted at a prefix that cannot be resolved without running the app."""

from fastapi import APIRouter

router = APIRouter(tags=["admin"])


@router.get("/stats")
async def stats():
    return {}
