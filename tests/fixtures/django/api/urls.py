"""Function views and class-based views, the plain-Django way."""

from django.urls import path

from . import views
from .views import NoteDetail, NoteList

app_name = "api"

urlpatterns = [
    path("items/", views.item_list, name="item-list"),
    path("items/<int:pk>/", views.item_detail, name="item-detail"),
    path("files/<path:filename>", views.file_by_path),
    path("notes/", NoteList.as_view(), name="note-list"),
    path("notes/<int:pk>/", NoteDetail.as_view(), name="note-detail"),
]

# Routes built from data: real, and invisible to static analysis.
urlpatterns += [
    path(f"reports/{name}/", views.report, {"name": name}, name=f"report-{name}")
    for name in ("daily", "weekly")
]
