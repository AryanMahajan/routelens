"""Mounted at a prefix that cannot be resolved without running the app."""

from fastapi import APIRouter

from .items import ITEMS
from .users import USERS

router = APIRouter(tags=["admin"])


@router.get("/stats")
async def stats():
    return {"users": len(USERS), "items": len(ITEMS)}
