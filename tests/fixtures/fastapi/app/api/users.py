"""The ordinary case: a router with its own prefix, mounted elsewhere."""

from fastapi import APIRouter, Depends, Header, Query

from ..schemas import UserCreate

router = APIRouter(prefix="/users", tags=["users"])


@router.get("/")
async def list_users(limit: int = 20, search: str = Query(None)):
    """List every user."""
    return []


@router.get("/{user_id}")
async def get_user(user_id: int, x_trace_id: str = Header(None)):
    """Fetch one user."""
    return {}


@router.post("/", summary="Create a user")
async def create_user(payload: UserCreate, current = Depends(get_current_user)):
    return {}


@router.delete("/{user_id}", deprecated=True)
async def delete_user(user_id: int, token = Depends(oauth2_scheme)):
    return None
