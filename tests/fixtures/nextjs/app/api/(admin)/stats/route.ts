// The `(admin)` route group organises files without appearing in the URL.
export const dynamic = "force-dynamic";

export const GET = async () => {
  return Response.json({ users: 0, orders: 0 });
};
