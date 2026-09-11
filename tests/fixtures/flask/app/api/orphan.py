"""A blueprint nobody registers. Its routes exist in source and are served by nothing."""

from flask import Blueprint

bp = Blueprint("orphan", __name__, url_prefix="/orphan")


@bp.get("/forgotten")
def forgotten():
    return {"reachable": False}
