"""Nested blueprints: `/admin/reports/...` where `/admin` comes from the config."""

from flask import Blueprint, jsonify

from ..auth import token_required

bp = Blueprint("admin", __name__)
reports = Blueprint("reports", __name__, url_prefix="/reports")


@bp.get("/stats")
@token_required
def stats():
    return {"users": 2, "items": 2}


@reports.get("/daily")
@token_required
def daily():
    return jsonify([])


@reports.get("/by-date/<date>")
def by_date(date):
    return {"date": date}


bp.register_blueprint(reports)
