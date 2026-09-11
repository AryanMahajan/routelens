"""Mounted twice, under two version prefixes."""

from fastapi import APIRouter, Request

router = APIRouter(tags=["items"])

ITEMS = [{"id": f"item-{n}", "name": f"Item {n}"} for n in range(1, 8)]
PAGE_SIZE = 3


@router.get("/items")
async def list_items(page: int = 1):
    start = (page - 1) * PAGE_SIZE
    return {"page": page, "items": ITEMS[start : start + PAGE_SIZE]}


@router.api_route("/items/{item_id}", methods=["GET", "PUT"])
async def item_detail(item_id: str, request: Request):
    # The same handler under /v1 and /v2: the path says which one was called.
    return {"id": item_id, "method": request.method, "via": request.url.path}
