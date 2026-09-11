from flask import Blueprint, abort, jsonify, request

from ..auth import token_required

bp = Blueprint("users", __name__, url_prefix="/users")

USERS = {1: {"id": 1, "name": "Ada"}, 2: {"id": 2, "name": "Grace"}}


@bp.get("/")
def list_users():
    """List users."""
    limit = request.args.get("limit", 20, type=int)
    q = request.args.get("q")
    found = [u for u in USERS.values() if not q or q.lower() in u["name"].lower()]
    return jsonify(found[:limit])


@bp.get("/<int:user_id>")
def get_user(user_id):
    user = USERS.get(user_id)
    if user is None:
        abort(404)
    return jsonify(user)


@bp.post("/")
@token_required
def create_user():
    """Create a user."""
    data = request.get_json()
    name = data["name"]
    user = {"id": max(USERS) + 1, "name": name, "email": data.get("email")}
    USERS[user["id"]] = user
    return jsonify(user), 201


@bp.route("/<int:user_id>", methods=["DELETE"])
@token_required
def delete_user(user_id):
    USERS.pop(user_id, None)
    return "", 204


@bp.post("/login")
def login():
    username = request.form["username"]
    password = request.form.get("password", "")
    return {"token": "letmein" if password else None, "user": username}
