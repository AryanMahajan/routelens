// Catch-all: /api/docs/a/b/c
export function GET(_request: Request, { params }: { params: { slug: string[] } }) {
  return Response.json({ slug: params.slug });
}
