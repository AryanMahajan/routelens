"""A bearer-token guard, so the protected routes are really protected when the app runs.

The token is fixed so the fixture can be exercised by hand: `Authorization: Bearer letmein`.
"""

from fastapi import Depends, HTTPException, status
from fastapi.security import OAuth2PasswordBearer

VALID_TOKEN = "letmein"

oauth2_scheme = OAuth2PasswordBearer(tokenUrl="token")


async def get_current_user(token: str = Depends(oauth2_scheme)) -> dict:
    if token != VALID_TOKEN:
        raise HTTPException(status.HTTP_401_UNAUTHORIZED, "invalid token")
    return {"id": 0, "name": "admin"}
