from flask import Blueprint, jsonify, request

bp = Blueprint("items", __name__)

ITEMS = [{"sku": "a-1", "price": 9.5}, {"sku": "b-2", "price": 12.0}]


@bp.route("/items")
def list_items():
    """Items — served under both /v1 and /v2."""
    return jsonify(ITEMS)


@bp.route("/items/<sku>", methods=["GET", "PUT"])
def item(sku):
    if request.method == "PUT":
        body = request.json
        for existing in ITEMS:
            if existing["sku"] == sku:
                existing["price"] = body["price"]
                return jsonify(existing)
    match = next((i for i in ITEMS if i["sku"] == sku), None)
    return (jsonify(match), 200) if match else ({"error": "no such item"}, 404)


@bp.get("/files/<path:filename>")
def file_by_path(filename):
    return {"filename": filename}
