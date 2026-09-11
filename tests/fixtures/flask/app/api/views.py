"""A class-based view registered from the factory."""

from flask import jsonify, request
from flask.views import MethodView

NOTES = {1: "first"}


class NoteAPI(MethodView):
    def get(self, note_id):
        if note_id is None:
            return jsonify(list(NOTES.values()))
        return {"id": note_id, "text": NOTES.get(note_id, "")}

    def put(self, note_id):
        NOTES[note_id] = request.get_json()["text"]
        return {"id": note_id, "text": NOTES[note_id]}

    def delete(self, note_id):
        NOTES.pop(note_id, None)
        return "", 204
