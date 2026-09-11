// Optional catch-all: matches /api/search as well as /api/search/a/b
export function GET(request: Request) {
  const { searchParams } = new URL(request.url);
  const page = searchParams.get("page");
  return Response.json({ page });
}
