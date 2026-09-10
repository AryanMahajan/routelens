"""Declared but never mounted — usually a bug in the project being inspected."""

from fastapi import APIRouter

router = APIRouter(prefix="/orphan")


@router.get("/forgotten")
async def forgotten():
    return {}
