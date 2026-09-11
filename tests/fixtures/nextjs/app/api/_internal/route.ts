// A folder starting with `_` is private: Next never routes to it.
export function GET() {
  return Response.json({ private: true });
}
