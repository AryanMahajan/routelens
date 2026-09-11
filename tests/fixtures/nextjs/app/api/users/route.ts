import { NextRequest, NextResponse } from "next/server";
import { listUsers, createUser } from "@/lib/users";

// GET /api/users?limit=20&search=ann
export async function GET(request: NextRequest) {
  const limit = request.nextUrl.searchParams.get("limit") ?? "20";
  const search = request.nextUrl.searchParams.get("search");
  return NextResponse.json(await listUsers({ limit: Number(limit), search }));
}

export async function POST(request: NextRequest) {
  const body = await request.json();
  const user = await createUser(body);
  return NextResponse.json(user, { status: 201 });
}
