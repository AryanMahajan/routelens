"""The ordinary case: a router with its own prefix, mounted elsewhere."""

from fastapi import APIRouter, Depends, Header, HTTPException, Query

from ..auth import get_current_user, oauth2_scheme
from ..schemas import UserCreate

router = APIRouter(prefix="/users", tags=["users"])

USERS: dict[int, dict] = {
    1: {"id": 1, "name": "Ann", "email": "ann@example.com"},
    2: {"id": 2, "name": "Ben", "email": "ben@example.com"},
    3: {"id": 3, "name": "Cara", "email": "cara@example.com"},
}


@router.get("/")
async def list_users(limit: int = 20, search: str = Query(None)):
    """List every user."""
    users = list(USERS.values())
    if search:
        users = [u for u in users if search.lower() in u["name"].lower()]
    return users[:limit]


@router.get("/{user_id}")
async def get_user(user_id: int, x_trace_id: str = Header(None)):
    """Fetch one user."""
    user = USERS.get(user_id)
    if user is None:
        raise HTTPException(404, f"no user {user_id}")
    return {**user, "trace_id": x_trace_id}


@router.post("/", summary="Create a user", status_code=201)
async def create_user(payload: UserCreate, current = Depends(get_current_user)):
    user_id = max(USERS) + 1
    USERS[user_id] = {"id": user_id, **payload.model_dump()}
    return USERS[user_id]


@router.delete("/{user_id}", deprecated=True, status_code=204)
async def delete_user(user_id: int, token = Depends(oauth2_scheme)):
    if token != "letmein":
        raise HTTPException(401, "invalid token")
    USERS.pop(user_id, None)
    return None
