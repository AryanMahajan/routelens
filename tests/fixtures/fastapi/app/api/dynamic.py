"""Routes built in a loop.

Static analysis cannot see these, and the snapshot records that as an expected miss
rather than pretending otherwise.
"""

from fastapi import APIRouter

router = APIRouter(prefix="/dynamic")

for name in ("alpha", "beta", "gamma"):
    router.add_api_route(f"/{name}", (lambda n: (lambda: {"name": n}))(name), methods=["GET"])
