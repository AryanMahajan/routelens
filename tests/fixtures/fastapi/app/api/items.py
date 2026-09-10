"""Mounted twice, under two version prefixes."""

from fastapi import APIRouter

router = APIRouter(tags=["items"])


@router.get("/items")
async def list_items(page: int = 1):
    return []


@router.api_route("/items/{item_id}", methods=["GET", "PUT"])
async def item_detail(item_id: str):
    return {}
