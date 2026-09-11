import { NextResponse } from "next/server";
import { getUser, deleteUser } from "@/lib/users";

type Params = { params: Promise<{ id: string }> };

export async function GET(_request: Request, { params }: Params) {
  const { id } = await params;
  const user = await getUser(id);
  return user ? NextResponse.json(user) : new NextResponse(null, { status: 404 });
}

async function remove(request: Request, { params }: Params) {
  const token = request.headers.get("authorization");
  if (!token) return new NextResponse(null, { status: 401 });
  const { id } = await params;
  await deleteUser(id);
  return new NextResponse(null, { status: 204 });
}

// A handler exported under a different name than it was declared with.
export { remove as DELETE };
