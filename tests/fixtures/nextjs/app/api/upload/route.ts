export async function PUT(request: Request) {
  const form = await request.formData();
  const file = form.get("file");
  return Response.json({ received: file instanceof File });
}

// Not a method handler; Next ignores it and so should we.
export const runtime = "nodejs";
