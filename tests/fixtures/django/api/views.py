"""Every kind of Django and DRF view the adapter understands."""

from django.http import JsonResponse
from django.views.decorators.http import require_http_methods
from rest_framework import mixins, status, viewsets
from rest_framework.decorators import action
from rest_framework.generics import ListCreateAPIView, RetrieveUpdateDestroyAPIView
from rest_framework.response import Response

from .auth import HasToken
from .serializers import TagSerializer, UserSerializer

USERS = {1: {"id": 1, "name": "Ada"}, 2: {"id": 2, "name": "Grace"}}
TAGS = [{"slug": "rust", "label": "Rust"}, {"slug": "python", "label": "Python"}]
ITEMS = {1: {"id": 1, "sku": "a-1"}}
NOTES = {1: {"id": 1, "name": "first"}}


# --- plain Django -------------------------------------------------------------------------


def health(request, year=None):
    """Liveness probe."""
    return JsonResponse({"ok": True, "year": year})


def legacy_ping(request):
    return JsonResponse({"pong": True})


@require_http_methods(["GET", "POST"])
def item_list(request):
    """List items, or create one from a form post."""
    if request.method == "POST":
        sku = request.POST["sku"]
        item = {"id": max(ITEMS) + 1, "sku": sku}
        ITEMS[item["id"]] = item
        return JsonResponse(item, status=201)
    limit = int(request.GET.get("limit", 20))
    return JsonResponse(list(ITEMS.values())[:limit], safe=False)


def item_detail(request, pk):
    item = ITEMS.get(pk)
    if item is None:
        return JsonResponse({"error": "no such item"}, status=404)
    return JsonResponse(item)


def file_by_path(request, filename):
    return JsonResponse({"filename": filename})


def report(request, name):
    return JsonResponse({"report": name})


def stats(request):
    return JsonResponse({"users": len(USERS), "items": len(ITEMS)})


# --- DRF generics -------------------------------------------------------------------------


class NoteList(ListCreateAPIView):
    """Notes, with a real serializer describing the body."""

    serializer_class = UserSerializer
    permission_classes = [HasToken]

    def get_queryset(self):
        return list(NOTES.values())

    def perform_create(self, serializer):
        note = {"id": max(NOTES) + 1, **serializer.validated_data}
        NOTES[note["id"]] = note


class NoteDetail(RetrieveUpdateDestroyAPIView):
    serializer_class = UserSerializer

    def get_object(self):
        return NOTES[int(self.kwargs["pk"])]


# --- DRF viewsets -------------------------------------------------------------------------


class UserViewSet(viewsets.ViewSet):
    """Users, hand-written action by action."""

    serializer_class = UserSerializer

    def list(self, request):
        q = request.query_params.get("q")
        found = [u for u in USERS.values() if not q or q.lower() in u["name"].lower()]
        return Response(found)

    def retrieve(self, request, pk=None):
        user = USERS.get(int(pk))
        if user is None:
            return Response(status=status.HTTP_404_NOT_FOUND)
        return Response(user)

    def create(self, request):
        """Create a user."""
        serializer = UserSerializer(data=request.data)
        serializer.is_valid(raise_exception=True)
        user = {"id": max(USERS) + 1, **serializer.validated_data}
        USERS[user["id"]] = user
        return Response(user, status=status.HTTP_201_CREATED)

    def destroy(self, request, pk=None):
        USERS.pop(int(pk), None)
        return Response(status=status.HTTP_204_NO_CONTENT)

    @action(detail=True, methods=["post"], url_path="set-password", permission_classes=[HasToken])
    def set_password(self, request, pk=None):
        return Response({"id": pk, "password": request.data.get("password")})

    @action(detail=False)
    def recent(self, request):
        return Response(list(USERS.values())[-1:])


class TagViewSet(mixins.ListModelMixin, viewsets.GenericViewSet):
    serializer_class = TagSerializer
    lookup_field = "slug"

    def get_queryset(self):
        return TAGS

    def retrieve(self, request, slug=None):
        tag = next((t for t in TAGS if t["slug"] == slug), None)
        return Response(tag) if tag else Response(status=status.HTTP_404_NOT_FOUND)
